#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Execute pinned SDK NewBuilder.Build in a disposable archive, never the checkout."""
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

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--scratch', type=Path, default=Path('/workspace/scratch/project-state-runtime-reference'))
    args = parser.parse_args()
    args.scratch.mkdir(parents=True, exist_ok=True)
    sdk = ROOT / 'repos/sdk'
    if subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=sdk, text=True).strip() != SHA:
        raise SystemExit('SDK checkout differs from source lock')
    env = os.environ.copy()
    env.update(GOROOT=env.get('GOROOT', '/usr/local/go'), GOTOOLCHAIN='local', GOTELEMETRY='off', GOWORK='off', GOFLAGS='-mod=readonly')
    env.setdefault('GOCACHE', str(args.scratch / 'go-build'))
    env.setdefault('GOMODCACHE', str(args.scratch / 'go-mod'))
    go = str(Path(env['GOROOT']) / 'bin/go')
    version = subprocess.check_output([go, 'version'], env=env, text=True).strip()
    with tempfile.TemporaryDirectory(dir=args.scratch) as temporary:
        source = Path(temporary)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=sdk)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        runtime = source / 'pkg/agentsdk/runtime'
        sources = sorted(runtime.glob('*.go')) + sorted((source / 'pkg/agentsdk/projectstate').glob('*.go')) + [source / 'go.mod', source / 'go.sum']
        harness = [Path(__file__).resolve(), ROOT / 'scripts/project-state-runtime-reference/reference_test.go']
        provenance = {
            'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA, 'goVersion': version,
            'launchPolicy': 'Offline builder construction only; dummy provider key is never used for requests. Explicit disabled SDK command sandbox; no tools executed. No sandbox or provider-I/O parity claim.',
            'sourceSHA256': {str(p.relative_to(source)): digest(p) for p in sources},
            'harnessSHA256': {str(p.relative_to(ROOT)): digest(p) for p in harness},
            'normalization': 'Sort tool names; replace temporary workdir with /fixture in working state. No expected outputs read by Go. Explicit native filesystem authority, redacted diagnostics and draining lifecycle are intentional differences.',
        }
        shutil.copyfile(ROOT / 'scripts/project-state-runtime-reference/reference_test.go', runtime / 'native_reference_test.go')
        output = source / 'observations.json'
        env.update(PROJECT_STATE_OUTPUT=str(output))
        command = [go, 'test', '-count=1', '-run', '^TestProjectStateRuntimeReference$', '-v', './pkg/agentsdk/runtime']
        subprocess.run(command, cwd=source, env=env, check=True)
        result = {'provenance': provenance, 'cases': json.loads(output.read_text())}
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/project-state/runtime-observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Project-state runtime observations/provenance differ')
            print('Pinned SDK project-state runtime observations reproduced exactly')
        else:
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')

if __name__ == '__main__':
    main()
