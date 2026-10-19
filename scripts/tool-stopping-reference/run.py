#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce tool stopping observations in a disposable pinned SDK archive."""
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
        paths = ['internal/agent/agent.go', 'internal/agent/runner_test.go', 'pkg/agentsdk/subagent_tools_test.go', 'internal/agent/runner.go', 'internal/agent/run_config.go', 'internal/agent/run_result.go', 'go.mod', 'go.sum']
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
                      'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/tool-stopping-reference/run.py', 'scripts/tool-stopping-reference/reference_test.go']}}
        shutil.copyfile(ROOT / 'scripts/tool-stopping-reference/reference_test.go', source / 'pkg/agentsdk/native_tool_stopping_test.go')
        output = source / 'observations.json'
        env['METADATA_OUTPUT'] = str(output)
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeToolStoppingReference)$', './pkg/agentsdk'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        subprocess.run([go, 'test', '-count=1', '-v', './internal/agent', '-run', '^(TestRunnerStopOnFirstTool|TestRunnerStopAtToolsRunsOutputGuardrails|TestRunnerStopAtToolsUsesToolOutputAsFinal)$'], cwd=source, env=env, check=True)
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/tool-stopping/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Tool stopping observations/provenance differ')
            print('Pinned tool stopping observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
