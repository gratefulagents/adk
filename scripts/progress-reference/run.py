#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce progress tracker observations in a disposable pinned SDK archive."""
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
    with tempfile.TemporaryDirectory(dir=os.environ.get("ADK_REFERENCE_TMPDIR", "/workspace/scratch")) as temp:
        source = Path(temp)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=ROOT / 'repos/sdk')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        paths = ['internal/agent/stream.go', 'internal/agent/record.go', 'internal/agent/tracing.go', 'internal/agent/usage.go', 'internal/agent/runner.go', 'internal/agent/platform_hooks.go', 'internal/agent/stream_test.go', 'internal/agent/record_forwarding_test.go', 'go.mod', 'go.sum']
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
                      'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/progress-reference/run.py', 'scripts/progress-reference/reference_test.go', 'scripts/progress-reference/helper_reference_test.go', 'scripts/progress-reference/cases.json', 'scripts/progress-reference/runtime_reference_test.go']}}
        shutil.copyfile(ROOT / 'scripts/progress-reference/reference_test.go', source / 'internal/agent/native_progress_test.go')
        shutil.copyfile(ROOT / 'scripts/progress-reference/helper_reference_test.go', source / 'internal/agent/native_progress_helpers_test.go')
        shutil.copyfile(ROOT / 'scripts/progress-reference/runtime_reference_test.go', source / 'internal/agent/native_progress_runtime_test.go')
        env['PROGRESS_RUNTIME_OUTPUT'] = str(source / 'runtime.json')
        env['PROGRESS_OUTPUT'] = str(source / 'observations.json')
        env['PROGRESS_HELPERS_OUTPUT'] = str(source / 'helpers.json')
        env['PROGRESS_INPUT'] = str(ROOT / 'scripts/progress-reference/cases.json')
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeProgress.*Reference|TestProgressTracker_.*|TestNewProgressTracker|TestNewChildTracker|TestChildTrackerForwards.*|TestRecordSubagentCompletedDoesNotDoubleCountForwardedUsage)$', './internal/agent'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        for filename in ['observations.json', 'helpers.json', 'runtime.json']:
            result = dict(provenance=provenance, **json.loads((source / filename).read_text()))
            serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
            destination = ROOT / 'fixtures/progress' / filename
            if args.check:
                if destination.read_text() != serialized:
                    raise SystemExit(f'Progress observations/provenance differ: {filename}')
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text(serialized)
                print(f'Wrote {destination.relative_to(ROOT)}')
        if args.check:
            print('Pinned progress observations reproduced exactly')


if __name__ == '__main__':
    main()
