#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Execute the pinned SDK assembler in a disposable source archive."""
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
        paths = ['internal/anthropic/stream.go', 'internal/anthropic/types.go', 'internal/anthropic/messages.go', 'internal/anthropic/audit_fixes_test.go', 'go.mod', 'go.sum']
        harness = ['scripts/anthropic-assembler-reference/run.py', 'scripts/anthropic-assembler-reference/reference_test.go']
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA, 'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in harness}}
        shutil.copyfile(ROOT / harness[1], source / 'internal/anthropic/native_assembler_test.go')
        output = source / 'observations.json'
        env['ASSEMBLER_OUTPUT'] = str(output)
        subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeAssemblerReference|TestStreamAssembler.*)$', './internal/anthropic'], cwd=source, env=env, check=True)
        result = dict(provenance=provenance, cases=json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/anthropic-assembler/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Anthropic assembler observations/provenance differ')
            print('Pinned Anthropic assembler observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
