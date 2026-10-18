#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce sandbox expansion observations in a disposable pinned SDK archive."""
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
        paths = ['pkg/agentsdk/sandbox/executor.go', 'go.mod', 'go.sum']
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
                      'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'stdlibSHA256': {p: hashlib.sha256((Path(env['GOROOT']) / p).read_bytes()).hexdigest() for p in ['src/strconv/number.go', 'src/os/env.go', 'src/strings/strings.go', 'src/unicode/graphic.go', 'src/unicode/tables.go', 'src/path/filepath/path.go', 'src/internal/filepathlite/path.go', 'src/internal/filepathlite/path_unix.go']},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/sandbox-env-reference/run.py', 'scripts/sandbox-env-reference/reference_test.go', 'scripts/sandbox-env-reference/environment_test.go']}}
        shutil.copyfile(ROOT / 'scripts/sandbox-env-reference/reference_test.go', source / 'pkg/agentsdk/sandbox/native_expansion_reference_test.go')
        shutil.copyfile(ROOT / 'scripts/sandbox-env-reference/environment_test.go', source / 'pkg/agentsdk/sandbox/native_environment_reference_test.go')
        output = source / 'observations.json'
        env['EXPANSION_OUTPUT'] = str(output)
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeExpansionReference)$', './pkg/agentsdk/sandbox'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/sandbox-env/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Sandbox expansion observations/provenance differ')
            print('Pinned sandbox expansion observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
