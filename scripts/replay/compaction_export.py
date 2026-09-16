#!/usr/bin/env python3
"""Export/check deterministic local compaction using clean pinned Go sources, offline by default."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT / 'repos/sdk'
PIN = '1dc92b73900fac74dc357a938e4b5eee6392b418'
SOURCES = ['LICENSE', 'go.mod', 'go.sum', 'internal/agent/history_compaction.go',
           'internal/agent/history_compaction_test.go', 'internal/agent/run_config.go',
           'internal/agent/runner.go', 'internal/agent/items.go', 'internal/agent/stream.go',
           'internal/agent/compaction_llm.go', 'internal/agent/prompt_cache_key_test.go']


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    def message(role, text, repeat=1):
        content = {'type': 'text', 'text': text}
        if repeat != 1:
            content['repeat'] = repeat
        return {'type': 'message', 'message': {'role': role, 'content': [content]}}

    def call(id, name='Bash', args=None):
        return {'type': 'tool_call', 'call': {'id': id, 'name': name,
                'arguments': args if args is not None else {'command': 'cat src/main.rs'}}}

    def result(id, text, error=False, repeat=1):
        content = {'type': 'text', 'text': text}
        if repeat != 1:
            content['repeat'] = repeat
        return {'type': 'tool_result', 'call_id': id, 'output': {
            'content': [content], 'is_error': error, 'should_pause': False}}

    def approval(id, phase, agent='reviewer', args=None):
        marker = dict(call(id, 'deploy', args), type='approval', phase=phase)
        if agent is not None:
            marker['agent'] = agent
        return marker

    user = lambda text, repeat=1: message('user', text, repeat)
    assistant = lambda text, repeat=1: message('assistant', text, repeat)
    cases = []

    def add(name, history, policy=None, overhead=0, disabled=False):
        cases.append(dict(name=name, history=history, policy=policy or {}, overhead=overhead, disabled=disabled))

    small = dict(trigger_tokens=100, target_tokens=80, preserve_recent_items=2, preserve_initial_user_messages=1)
    ordinary = [user('Implement the original task'), assistant('Investigated src/main.rs. Next run tests. ', 50),
                call('a'), result('a', 'Tests passed. ', repeat=200), user('Keep the original constraint'), assistant('Continue')]
    add('empty', [])
    add('disabled', ordinary, small, disabled=True)
    add('below_default', ordinary)
    add('default_triggered', [user('Original task'), assistant('old ', 190000), user('Second user'), assistant('Continue')])
    add('normalization_zero', ordinary, {k: 0 for k in small})
    add('at_threshold', [user('abcd'), assistant('abcd')], {'trigger_tokens': 20, 'target_tokens': 10})
    add('no_removable', [user('abcd'), assistant('abcd')], {'trigger_tokens': 19, 'target_tokens': 10})
    add('ineffective', [assistant('a'), assistant('b'), assistant('c')], {'trigger_tokens': 20, 'target_tokens': 10})
    add('full_summary', ordinary, dict(small, target_tokens=1500, trigger_tokens=1600), overhead=1000)
    add('terse_summary', ordinary, dict(small, trigger_tokens=800, target_tokens=200))
    add('minimal_summary', ordinary, small)
    add('overhead_crossing', ordinary, dict(small, trigger_tokens=1200, target_tokens=900), overhead=500)
    add('overhead_exceeds_target', ordinary, small, overhead=100)
    add('negative_overhead', ordinary, small, overhead=-100)
    add('target_renormalized', ordinary, dict(small, target_tokens=1000))
    add('forced_trigger_one_renormalizes_target', ordinary, dict(small, trigger_tokens=1, target_tokens=500))
    add('initial_exclusions', [user('[SYSTEM] '+('s'*500)), user('[PHASE TRANSITION planning] '+('p'*500)),
        user('[COMPACTION CARRY-FORWARD] stale'), user('Real task'), user('Second task'),
        assistant('analysis ', 150), assistant('last')], dict(small, preserve_initial_user_messages=2))
    add('stale_carry_forward_in_tail', [user('task'), assistant('old ', 500), user('[COMPACTION CARRY-FORWARD] stale'), assistant('latest')], small)
    add('pair_repair_orphan_and_duplicate', [user('task'), assistant('old ', 500), result('unknown', 'orphan'), call('a'), call('a'), result('a', 'ok'), result('a', 'duplicate')], dict(small, preserve_recent_items=5, target_tokens=300, trigger_tokens=400))
    add('summary_after_first_user', [assistant('obsolete ', 300), user('Original task'), assistant('last')], small)
    add('pair_output_in_tail', [user('task'), call('a'), assistant('old ', 500), result('a', 'ok')], small)
    add('pair_call_in_tail', [user('task'), result('a', 'old result'), assistant('old ', 500), call('a')], small)
    add('tail_search', [user('task'), assistant('old ', 400), assistant('recent ', 100), assistant('latest')], small)
    add('best_above_target', [user('large original ', 80), assistant('old ', 500), assistant('latest')], small)
    previous = assistant('[COMPACTED HISTORY SUMMARY]\nConversation summary:\n- Keep important original findings.\n- Key timeline:\n  - old timeline')
    add('recompact_full', [user('task'), previous, assistant('Pending implement src/main.rs. ', 600), assistant('latest')],
        dict(small, trigger_tokens=1000, target_tokens=800))
    add('recompact_terse', [user('task'), previous, assistant('next ', 500), assistant('latest')], dict(small, target_tokens=180, trigger_tokens=200))
    add('recompact_minimal', [user('task'), previous, assistant('next ', 500), assistant('latest')], small)
    add('recompact_crlf_terse', [user('task'), assistant('[COMPACTED HISTORY SUMMARY]\r\nConversation summary:\r\n- Keep findings.\r\n- Next action.\r\n'), assistant('next ', 500), assistant('latest')], dict(small, target_tokens=180, trigger_tokens=200))
    add('tag_summary', [user('task'), assistant('[COMPACTED HISTORY SUMMARY]\n<summary>remember state\n\n\nnext action</summary>'),
        assistant('next ', 800), assistant('latest')], dict(small, trigger_tokens=800, target_tokens=700))
    add('unicode_summary', [user('初期の依頼'), assistant('日本語🙂 next src/日本語.rs ', 200), assistant('終わり')], dict(small, trigger_tokens=500, target_tokens=400))
    add('paths_and_tools', [user('task'), assistant('next inspect ./src/main.rs src/app.tsx .git/config.json node_modules/pkg/a.ts .github/workflows/ci.yml dist/a.js README.md ', 100),
        call('a', 'bash'), result('a', 'error in src/lib.rs', True), call('b', 'Bash', {'command': 'cargo test'}), result('b', 'ok'),
        call('c', 'Read', {'path': 'src/lib.rs'}), result('c', 'ok'), assistant('latest')], dict(small, trigger_tokens=900, target_tokens=800, preserve_recent_items=1))
    add('unique_bullets_timeline_cap', [user('task')] + [assistant('Next duplicate ', 100)] * 35 + [assistant('latest')], dict(small, trigger_tokens=1000, target_tokens=900, summary_bullet_limit=1))
    for phase in ['pending', 'approved', 'denied']:
        marker = approval('a', phase)
        add(f'approval_{phase}_below', [user('task'), call('a', 'deploy'), marker])
        add(f'approval_{phase}_disabled', [marker], small, disabled=True)
        add(f'approval_{phase}_drop_old', [user('task'), call('a', 'deploy'), marker, result('a', 'done'),
            assistant('old ', 2000), assistant('latest')], dict(small, trigger_tokens=1000, target_tokens=800, preserve_recent_items=1))
        add(f'approval_{phase}_retain_recent', [user('task'), assistant('old ', 2000),
            call('a', 'deploy'), marker, result('a', 'done')], dict(small, trigger_tokens=1000, target_tokens=800))
        add(f'approval_{phase}_pending_call_repair', [user('task'), assistant('old ', 2000),
            call('a', 'deploy'), marker, call('b'), result('b', 'done')],
            dict(small, trigger_tokens=1000, target_tokens=800, preserve_recent_items=4))
        add(f'approval_{phase}_previous_marker_call_repair', [user('task'), marker, assistant('old ', 2000),
            call('a', 'deploy'), call('b'), result('b', 'done')],
            dict(small, trigger_tokens=1000, target_tokens=800, preserve_recent_items=3))
    add('approval_markers_only', [approval('first', 'pending'), approval('second', 'approved'), approval('third', 'denied')])
    add('approval_only_trigger', [approval('first', 'pending', args={'data': 'x'*10000}), approval('last', 'denied')], small)
    add('approval_same_boundary_order', [approval('prefix', 'pending', agent=None), user('task'), assistant('old ', 2000),
        approval('same', 'pending', agent=''), approval('same', 'approved'), approval('denied', 'denied'), assistant('latest')],
        dict(small, trigger_tokens=1000, target_tokens=800, preserve_recent_items=4))
    add('approval_marker_cost_triggers', [user('task'), assistant('old ', 100), approval('a', 'pending', args={'data': 'x'*2000})],
        dict(small, trigger_tokens=500, target_tokens=400))
    add('approval_marker_consumes_recent_slot', [user('task'), assistant('old ', 2000), call('a'),
        approval('a', 'pending')], dict(small, trigger_tokens=1000, target_tokens=800, preserve_recent_items=1))
    add('approval_finalizer_rebases_boundaries', [user('task'), assistant('old ', 2000), call('a'), approval('a', 'approved'),
        user('[COMPACTION CARRY-FORWARD] stale'), call('a'), approval('b', 'denied'), result('a', 'done'), result('a', 'duplicate'),
        approval('c', 'pending')], dict(small, trigger_tokens=1000, target_tokens=800, preserve_recent_items=8))
    # A fixed seed broadens split/threshold/pair coverage without making refresh nondeterministic.
    rng = random.Random(4192)
    for index in range(48):
        history = [user('task')]
        for turn in range(rng.randrange(3, 16)):
            if rng.randrange(3) == 0:
                history += [call(str(turn), rng.choice(['Read', 'Bash', 'test']), {'path': 'src/file.rs'}),
                            result(str(turn), 'result ', repeat=rng.randrange(1, 200))]
            else:
                history.append(message(rng.choice(['user', 'assistant']), rng.choice(['Next inspect src/lib.rs. ', '日本語🙂 ', 'done ']), rng.randrange(1, 200)))
        trigger = rng.randrange(50, 1500)
        add(f'seeded_{index:02}', history, dict(trigger_tokens=trigger, target_tokens=rng.randrange(1, trigger),
            preserve_recent_items=rng.randrange(1, 12), preserve_initial_user_messages=rng.randrange(1, 4), summary_bullet_limit=rng.randrange(1, 5)), overhead=rng.randrange(0, 500))
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--allow-downloads', action='store_true')
    args = parser.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=SDK, text=True).strip()
    if revision != PIN:
        raise SystemExit(f'SDK pin mismatch: {revision} != {PIN}')
    if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=SDK):
        raise SystemExit('SDK tracked sources must be clean')
    cases = inputs()
    input_text = json.dumps(cases, indent=2, sort_keys=True, ensure_ascii=False) + '\n'
    env = {k: os.environ[k] for k in ('PATH', 'GOPATH', 'GOMODCACHE', 'GOCACHE', 'TMPDIR') if k in os.environ}
    env.update(GOROOT=os.environ.get('GOROOT', '/usr/local/go'), GOTOOLCHAIN='local', GOTELEMETRY='off',
               GRATEFUL_LIVE_TESTS='skip', GOMAXPROCS='2', CGO_ENABLED='0')
    env.setdefault('GOMODCACHE', str(Path.home() / 'go/pkg/mod'))
    env.setdefault('GOCACHE', str(Path.home() / '.cache/go-build'))
    if not args.allow_downloads:
        env.update(GOPROXY='off', GOSUMDB='off')
    command = [str(Path(env['GOROOT']) / 'bin/go'), 'test', '-mod=readonly', '-p=2', '-count=1', '-timeout=120s',
               '-run', '^TestExportCompactionReference$', './internal/agent']
    with tempfile.TemporaryDirectory(prefix='compaction-replay-') as directory:
        temp = Path(directory)
        env.update(HOME=directory, COMPACTION_INPUT=str(temp / 'inputs.json'), COMPACTION_OUTPUT=str(temp / 'output.json'))
        (temp / 'inputs.json').write_text(input_text)
        overlay = temp / 'overlay.json'
        overlay.write_text(json.dumps({'Replace': {str(SDK / 'internal/agent/compaction_replay_export_test.go'):
                           str(ROOT / 'scripts/replay/compaction_export.go')}}))
        toolchain = subprocess.check_output([command[0], 'version'], env=env, text=True).strip()
        result = subprocess.run(command[:2] + ['-overlay='+str(overlay)] + command[2:], cwd=SDK, env=env,
                                capture_output=True, timeout=300)
        if result.returncode:
            raise SystemExit(result.stdout.decode() + result.stderr.decode())
        expected = json.loads((temp / 'output.json').read_text())
    if set(expected['cases']) != {c['name'] for c in cases}:
        raise SystemExit('Go exporter case set mismatch')
    fixture = {'schema_version': 1, 'sdk_revision': PIN, **expected}
    text = json.dumps(fixture, sort_keys=True, indent=2, ensure_ascii=False) + '\n'
    manifest = {'schema_version': 1, 'sdk_revision': PIN, 'license': 'GPL-3.0-only',
                'license_copy': 'licenses/SDK-GPL-3.0.txt', 'toolchain': toolchain,
                'command': ['go', 'test', '-overlay=<temporary overlay>', *command[2:]],
                'environment': {k: env[k] for k in ('GOTOOLCHAIN', 'GOTELEMETRY', 'GRATEFUL_LIVE_TESTS', 'GOMAXPROCS', 'CGO_ENABLED')},
                'fixtures': {'compaction.json': hashlib.sha256(text.encode()).hexdigest(),
                             'compaction_inputs.json': hashlib.sha256(input_text.encode()).hexdigest()},
                'harness': {p: digest(ROOT / p) for p in ['scripts/replay/compaction_export.go', 'scripts/replay/compaction_export.py']},
                'sources': [{'path': p, 'sha256': digest(SDK / p), 'url': f'https://github.com/gratefulagents/sdk/blob/{PIN}/{p}'} for p in SOURCES]}
    for name, content in [('compaction_inputs.json', input_text), ('compaction.json', text),
                          ('compaction_manifest.json', json.dumps(manifest, sort_keys=True, indent=2)+'\n')]:
        path = ROOT / 'fixtures' / name
        if args.check:
            if path.read_text() != content:
                raise SystemExit(f'{name}: stale or nondeterministic; refresh and inspect diff')
        else:
            path.write_text(content)
    print(f'Go local compaction: {len(cases)} cases {"verified unchanged" if args.check else "exported"}')


if __name__ == '__main__':
    main()
