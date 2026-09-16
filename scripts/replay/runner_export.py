#!/usr/bin/env python3
"""Refresh/check actual Go runner observations with pinned, clean sources."""
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
SOURCES = ['LICENSE', 'go.mod', 'go.sum', 'internal/agent/runner.go',
           'internal/agent/model.go', 'internal/agent/items.go',
           'internal/agent/agent.go', 'internal/agent/run_config.go',
           'internal/agent/run_result.go', 'internal/agent/stream_events.go',
           'internal/agent/tool.go', 'internal/agent/usage.go',
           'internal/agent/model_fallback.go', 'internal/agent/output_schema.go',
           'internal/agent/retry.go', 'internal/agent/model_settings.go',
           'internal/agent/hooks.go']


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='compare without updating fixtures')
    parser.add_argument('--allow-downloads', action='store_true', help='bootstrap missing Go modules')
    args = parser.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=SDK, text=True).strip()
    if revision != PIN:
        raise SystemExit(f'SDK pin mismatch: {revision} != {PIN}')
    if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=SDK):
        raise SystemExit('SDK tracked sources must be clean')
    inputs = ROOT / 'fixtures/runner_inputs.json'
    env = {key: os.environ[key] for key in ('PATH', 'GOPATH', 'GOMODCACHE', 'GOCACHE', 'TMPDIR') if key in os.environ}
    env.update(GOROOT=os.environ.get('GOROOT', '/usr/local/go'), GOTOOLCHAIN='local',
               GOTELEMETRY='off', GRATEFUL_LIVE_TESTS='skip', GOMAXPROCS='2', CGO_ENABLED='0')
    if not args.allow_downloads:
        env.update(GOPROXY='off', GOSUMDB='off')
    command = [str(Path(env['GOROOT']) / 'bin/go'), 'run', '-mod=readonly', '../../scripts/replay/runner_export.go']
    with tempfile.TemporaryDirectory(prefix='runner-replay-') as home:
        env['HOME'] = home
        # Keep the existing module/build cache, never the existing credential-bearing HOME.
        env.setdefault('GOMODCACHE', str(Path.home() / 'go/pkg/mod'))
        env.setdefault('GOCACHE', str(Path.home() / '.cache/go-build'))
        toolchain = subprocess.check_output([command[0], 'version'], env=env, text=True, stderr=subprocess.PIPE).strip()
        result = subprocess.run(command, cwd=SDK, env=env, input=inputs.read_bytes(), capture_output=True, timeout=300)
    if result.returncode:
        raise SystemExit(result.stderr.decode())
    expected = json.loads(result.stdout)
    cases = json.loads(inputs.read_text())
    if set(expected) != {case['name'] for case in cases}:
        raise SystemExit('Go exporter case set mismatch')
    fixture = {'schema_version': 1, 'sdk_revision': PIN,
               'cases': [dict(input=case, expected=expected[case['name']]) for case in cases]}
    text = json.dumps(fixture, sort_keys=True, indent=2, ensure_ascii=False) + '\n'
    manifest = {'schema_version': 1, 'sdk_revision': PIN, 'license': 'GPL-3.0-only',
                'generator': {'toolchain': toolchain, 'cwd': 'repos/sdk',
                              'command': ['go', *command[1:]],
                              'environment': {key: env[key] for key in ('GOTOOLCHAIN', 'GOTELEMETRY', 'GRATEFUL_LIVE_TESTS', 'GOMAXPROCS', 'CGO_ENABLED')}},
                'license_copy': 'licenses/SDK-GPL-3.0.txt',
                'fixture_sha256': hashlib.sha256(text.encode()).hexdigest(),
                'harness': {p: digest(ROOT / p) for p in ['scripts/replay/runner_export.go',
                            'scripts/replay/runner_export.py', 'fixtures/runner_inputs.json']},
                'sources': [{'path': p, 'sha256': digest(SDK / p),
                             'url': f'https://github.com/gratefulagents/sdk/blob/{PIN}/{p}'} for p in SOURCES]}
    for name, content in [('runner.json', text), ('runner_manifest.json', json.dumps(manifest, sort_keys=True, indent=2) + '\n')]:
        path = ROOT / 'fixtures' / name
        if args.check:
            if path.read_text() != content:
                raise SystemExit(f'{name}: stale or nondeterministic; refresh and inspect diff')
        else:
            path.write_text(content)
    print(f'Go runner: {len(cases)} cases {"verified unchanged" if args.check else "exported"}')


if __name__ == '__main__':
    main()
