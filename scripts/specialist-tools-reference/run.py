#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce specialist tools observations in a disposable pinned SDK archive."""
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
    env = dict(os.environ, GOROOT=os.environ.get('GOROOT', '/usr/local/go'), GOTOOLCHAIN='local', GOTELEMETRY='off', GOWORK='off', GOFLAGS='-mod=readonly', GOPROXY='off', GOSUMDB='off')
    env.setdefault('GOCACHE', '/workspace/scratch/go-cache')
    env.setdefault('GOMODCACHE', '/workspace/scratch/go/pkg/mod')
    go = str(Path(env['GOROOT']) / 'bin/go')
    with tempfile.TemporaryDirectory() as temp:
        source = Path(temp)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=ROOT / 'repos/sdk')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        paths = ["pkg/agentsdk/specialists.go", "pkg/agentsdk/specialists_test.go", "pkg/agentsdk/aliases.go", "internal/agent/run_context.go", "internal/agent/tool.go", "go.mod", "go.sum"]
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
                      'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/specialist-tools-reference/run.py', 'scripts/specialist-tools-reference/reference_test.go', 'scripts/specialist-tools-reference/delegation_test.go']}}
        shutil.copyfile(ROOT / 'scripts/specialist-tools-reference/reference_test.go', source / 'pkg/agentsdk/native_specialist_tools_test.go')
        shutil.copyfile(ROOT / 'scripts/specialist-tools-reference/delegation_test.go', source / 'pkg/agentsdk/native_delegation_test.go')
        output = source / 'observations.json'
        env['METADATA_OUTPUT'] = str(output)
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeSpecialistToolsReference)$', './pkg/agentsdk'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        regression = subprocess.run([go, 'test', '-count=1', '-v', './pkg/agentsdk', '-run', '^(TestFilterToolsByAccessUsesAdapterForReadOnly|TestBuildDelegationGuideIncludesCompactTaskPacketGuidance)$'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(regression.stdout.decode('utf-8', errors='backslashreplace'), end='')
        regression.check_returncode()
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/specialist-tools/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Specialist tools observations/provenance differ')
            print('Pinned specialist tools observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
