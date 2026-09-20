#!/usr/bin/env python3
"""Generate/check actual pinned Go subagent observations; never synthesize expectations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT / 'repos/sdk'
PIN = '1dc92b73900fac74dc357a938e4b5eee6392b418'
GENERATOR = 'crates/adk-runtime/tests/fixtures/subagents.go'
FIXTURE = ROOT / 'crates/adk-runtime/tests/fixtures/go-subagents.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--allow-downloads', action='store_true')
    args = parser.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=SDK, text=True).strip()
    if revision != PIN:
        raise SystemExit(f'SDK pin mismatch: {revision}')
    if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=SDK):
        raise SystemExit('SDK tracked sources must be clean')
    version_tag = subprocess.check_output(['git', 'describe', '--tags', '--exact-match', PIN], cwd=SDK, text=True).strip()
    if version_tag != 'v0.0.115':
        raise SystemExit(f'SDK version mismatch: {version_tag}')
    env = {k: os.environ[k] for k in ('PATH', 'GOROOT', 'GOPATH', 'GOMODCACHE', 'GOCACHE', 'TMPDIR') if k in os.environ}
    env.update(GOTOOLCHAIN='local', GOTELEMETRY='off', GRATEFUL_LIVE_TESTS='skip', CGO_ENABLED='0', GOMAXPROCS='2')
    env.setdefault('GOROOT', '/usr/local/go')
    if not args.allow_downloads:
        env.update(GOPROXY='off', GOSUMDB='off')
    go = str(Path(env['GOROOT']) / 'bin/go')
    command = [go, 'run', '-mod=readonly', '../../' + GENERATOR]
    with tempfile.TemporaryDirectory(prefix='subagent-reference-') as home:
        env['HOME'] = home
        env.setdefault('GOCACHE', str(Path.home() / '.cache/go-build'))
        env.setdefault('GOMODCACHE', str(Path.home() / 'go/pkg/mod'))
        toolchain = subprocess.check_output([go, 'version'], env=env, text=True, stderr=subprocess.PIPE).strip()
        result = subprocess.run(command, cwd=SDK, env=env, capture_output=True, timeout=300)
    if result.returncode:
        raise SystemExit(result.stderr.decode())
    fixture = json.loads(result.stdout)
    fixture['provenance'] = {
        'sdk_version': version_tag, 'sdk_revision': PIN, 'go_version': toolchain,
        'generator': GENERATOR,
        'generator_sha256': hashlib.sha256((ROOT / GENERATOR).read_bytes()).hexdigest(),
        'exporter_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'cwd': 'repos/sdk', 'command': ['go', *command[1:]],
        'environment': {k: env[k] for k in ('GOTOOLCHAIN', 'GOTELEMETRY', 'GRATEFUL_LIVE_TESTS', 'CGO_ENABLED', 'GOMAXPROCS')},
        'normalization': 'Only generated task IDs (including references inside strings) and existing duration/started_at/timestamp/duration_ms values are replaced. No response fields are removed.',
        'sources': {p: hashlib.sha256((SDK / p).read_bytes()).hexdigest() for p in (
            'go.mod', 'go.sum', 'pkg/agentsdk/subagent_tools.go', 'pkg/agentsdk/scheduler.go',
            'internal/agent/subagent_registry.go', 'internal/agent/subagent_run.go')},
    }
    text = json.dumps(fixture, indent=2, sort_keys=True, ensure_ascii=False) + '\n'
    if args.check:
        if FIXTURE.read_text() != text:
            raise SystemExit('go-subagents.json: stale or nondeterministic; regenerate and inspect diff')
    else:
        FIXTURE.write_text(text)
    print(f"Go subagents: {len(fixture['cases'])} scenarios {'verified unchanged' if args.check else 'exported'}")


if __name__ == '__main__':
    main()
