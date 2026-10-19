#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce MCP display observations in a disposable pinned SDK archive."""
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
        paths = ['pkg/agentsdk/mcp/sanitize.go', 'pkg/agentsdk/mcp/sanitize_test.go', 'examples/features/mcp/mcp_test.go', 'go.mod', 'go.sum']
        provenance = {'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
                      'license': 'GPL-3.0-only',
                      'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
                      'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
                      'stdlibSHA256': {p: hashlib.sha256((Path(env['GOROOT']) / p).read_bytes()).hexdigest() for p in ['src/unicode/graphic.go', 'src/unicode/tables.go', 'LICENSE']},
                      'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/mcp-display-reference/run.py', 'scripts/mcp-display-reference/reference_test.go']}}
        shutil.copyfile(ROOT / 'scripts/mcp-display-reference/reference_test.go', source / 'pkg/agentsdk/mcp/native_mcp_display_reference_test.go')
        output = source / 'observations.json'
        env['MCP_DISPLAY_OUTPUT'] = str(output)
        execution = subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeMCPDisplayReference)$', './pkg/agentsdk/mcp'], cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(execution.stdout.decode('utf-8', errors='backslashreplace'), end='')
        execution.check_returncode()
        subprocess.run([go, 'test', '-count=1', '-v', './pkg/agentsdk/mcp', '-run', '^TestSanitizeMCP(Display|ToolDescription)'], cwd=source, env=env, check=True)
        subprocess.run([go, 'test', '-count=1', '-v', './examples/features/mcp'], cwd=source, env=env, check=True)
        result = dict(provenance=provenance, **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/mcp-display/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('MCP display observations/provenance differ')
            print('Pinned MCP display observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
