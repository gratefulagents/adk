#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce payload JSON observations in a disposable immutable SDK archive."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SHA = '1dc92b73900fac74dc357a938e4b5eee6392b418'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    env = dict(os.environ, GOROOT=os.environ.get('GOROOT', '/usr/local/go'),
               GOTOOLCHAIN='local', GOTELEMETRY='off', GOWORK='off',
               GOFLAGS='-mod=readonly', GOARCH='amd64', CGO_ENABLED='0')
    env.setdefault('GOCACHE', '/workspace/scratch/go-cache')
    env.setdefault('GOMODCACHE', '/workspace/scratch/go/pkg/mod')
    go = str(Path(env['GOROOT']) / 'bin/go')
    with tempfile.TemporaryDirectory(dir=os.environ.get('ADK_REFERENCE_TMPDIR')) as temp:
        source = Path(temp)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=ROOT / 'repos/sdk')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        paths = ['internal/agent/items.go', 'internal/agent/llm_snapshot.go', 'pkg/agentsdk/aliases.go', 'go.mod', 'go.sum']
        harness = ['scripts/payload-reference/run.py', 'scripts/payload-reference/reference_test.go']
        provenance = {
            'repository': 'https://github.com/gratefulagents/sdk',
            'commit': SHA,
            'license': 'GPL-3.0-only',
            'archiveSHA256': hashlib.sha256(archive).hexdigest(),
            'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
            'goOS': subprocess.check_output([go, 'env', 'GOOS'], env=env, text=True).strip(),
            'goARCH': env['GOARCH'],
            'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
            'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in harness},
        }
        shutil.copyfile(ROOT / harness[1], source / 'internal/agent/native_payload_reference_test.go')
        output = source / 'payload-observations.json'
        env['PAYLOAD_OUTPUT'] = str(output)
        execution = subprocess.run(
            [go, 'test', '-count=1', '-v', '-run', '^TestNativePayloadReference$', './internal/agent'],
            cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/payloads/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Payload observations/provenance differ')
            print('Pinned payload observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
