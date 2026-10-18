#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce governed agent-kind observations in a disposable pinned SDK archive."""
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
    env = dict(os.environ, GOROOT=os.environ.get('GOROOT', '/usr/local/go'), GOTOOLCHAIN='local', GOTELEMETRY='off', GOWORK='off', GOFLAGS='-mod=readonly')
    env.setdefault('GOCACHE', '/workspace/scratch/go-cache')
    env.setdefault('GOMODCACHE', '/workspace/scratch/go/pkg/mod')
    go = str(Path(env['GOROOT']) / 'bin/go')
    with tempfile.TemporaryDirectory() as temp:
        source = Path(temp)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=ROOT / 'repos/sdk')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        paths = ['internal/agent/agents/types.go', 'internal/agent/agents/types_test.go', 'go.mod', 'go.sum']
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
                      'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/agent-kinds-reference/run.py', 'scripts/agent-kinds-reference/reference_test.go']}}
        shutil.copyfile(ROOT / 'scripts/agent-kinds-reference/reference_test.go', source / 'internal/agent/agents/native_kinds_reference_test.go')
        output = source / 'observations.json'
        env['KINDS_OUTPUT'] = str(output)
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeKindsReference)$', './internal/agent/agents'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        subprocess.run([go, 'test', '-count=1', '-v', './internal/agent/agents', '-run', '^Test(BuiltInKindsAreStable|ResolveKindSupportsLegacyAliases|ResolveKindUnknown|IsAllowedResolvesAliases)$'], cwd=source, env=env, check=True)
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/agent-kinds/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Agent-kind observations/provenance differ')
            print('Pinned agent-kind observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
