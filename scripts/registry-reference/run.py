#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Observe actual NewRegistry selection in a disposable pinned SDK git archive."""
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
               GOFLAGS='-mod=readonly', GOPROXY='off', GOSUMDB='off')
    env.setdefault('GOCACHE', '/workspace/scratch/go-cache')
    env.setdefault('GOMODCACHE', '/workspace/scratch/go/pkg/mod')
    go = str(Path(env['GOROOT']) / 'bin/go')
    scratch = os.environ.get('REGISTRY_REFERENCE_TMPDIR', '/workspace/scratch')
    env['TMPDIR'] = scratch
    env['GOTMPDIR'] = scratch
    with tempfile.TemporaryDirectory(dir=scratch) as temp:
        source = Path(temp)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=ROOT / 'repos/sdk')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        # Hash the complete tools/policy sources, not just a static inventory.
        paths = ['go.mod', 'go.sum', 'internal/agent/tool.go']
        for directory in ['pkg/agentsdk/tools', 'pkg/agentsdk/policy', 'internal/policy']:
            paths.extend(str(p.relative_to(source)) for p in sorted((source / directory).rglob('*.go')))
        provenance = {
            'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
            'license': 'GPL-3.0-only',
            'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
            'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
            'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in
                              ['scripts/registry-reference/run.py', 'scripts/registry-reference/reference_test.go']},
        }
        shutil.copyfile(ROOT / 'scripts/registry-reference/reference_test.go',
                        source / 'pkg/agentsdk/tools/native_registry_selection_test.go')
        output = source / 'observations.json'
        env['REGISTRY_OUTPUT'] = str(output)
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run',
                                    '^TestNativeRegistrySelectionReference$', './pkg/agentsdk/tools'],
                                   cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/tools/registry-selection.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Registry selection observations/provenance differ')
            print('Pinned registry selection observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
