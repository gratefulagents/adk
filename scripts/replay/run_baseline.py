#!/usr/bin/env python3
"""Credential-free selected Go baselines; dependency downloads may require network."""
import argparse
import gzip
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'docs/migration/baseline'
SUITES = {
    'sdk-core': ('repos/sdk', ['./internal/agent', './pkg/agentsdk/events', './pkg/agentsdk/projectstate', './pkg/agentsdk/policy']),
    'sdk-session': ('repos/sdk', ['-run', 'Test(NewSessionEventStream|ContentEventLine|BuildConversation|BuildWorkingState|BuildAssistantTurn|SelectNextUser|CollectImmediate|ChatLoop|QuickAction|ExtractAskUser|ExtractPresentPlan|DetectUserInput|BuildAutoTurn)', './pkg/agentsdk']),
    'platform-state': ('repos/gratefulagents', ['./api/platform/v1alpha1', './internal/projectstate']),
    'platform-transcript': ('repos/gratefulagents', ['-run', 'Test(TranscriptSnapshot|PersistedItemsFromRun|RunItemsFromPersisted)', './cmd/agent']),
}

def environment():
    # Deliberate allowlist: never inherit provider, cloud, kube, database or telemetry credentials.
    env = {k: os.environ[k] for k in ('PATH', 'GOPATH', 'GOMODCACHE', 'GOCACHE', 'TMPDIR') if k in os.environ}
    home = ROOT / 'scripts/replay/.home'
    home.mkdir(exist_ok=True)
    env.update(HOME=str(home), GOROOT=os.environ.get('GOROOT', '/usr/local/go'), GOTOOLCHAIN='local',
               GOTELEMETRY='off', GRATEFUL_LIVE_TESTS='skip', GOMAXPROCS='2', CGO_ENABLED='0')
    return env

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('suites', nargs='*', choices=list(SUITES))
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    failed = False
    for name in args.suites or SUITES:
        cwd, packages = SUITES[name]
        cmd = ['go', 'test', '-mod=readonly', '-p=2', '-count=1', '-timeout=180s', '-json', *packages]
        env = environment()
        start = time.monotonic()
        with (OUT / (name + '.jsonl')).open('w') as log:
            try:
                result = subprocess.run(cmd, cwd=ROOT/cwd, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=600)
                code = result.returncode
            except subprocess.TimeoutExpired:
                code = 124
                log.write('\nHARNESS TIMEOUT after 600s\n')
        counts = {'pass': 0, 'fail': 0, 'skip': 0}
        packages_done = []
        for line in (OUT/(name+'.jsonl')).read_text().splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            action = event.get('Action')
            if action in counts:
                if event.get('Test'):
                    counts[action] += 1
                else:
                    packages_done.append({k: event[k] for k in ('Package', 'Action', 'Elapsed') if k in event})
        record = dict(command=cmd, cwd=cwd, environment=env, exit_code=code,
                      elapsed_seconds=round(time.monotonic()-start, 3), test_events=counts,
                      packages=packages_done,
                      revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT/cwd, text=True).strip())
        (OUT/(name+'.json')).write_text(json.dumps(record, indent=2)+'\n')
        raw_log = OUT/(name+'.jsonl')
        (OUT/(name+'.jsonl.gz')).write_bytes(gzip.compress(raw_log.read_bytes(), mtime=0))
        raw_log.unlink()
        print(name, code, counts, flush=True)
        failed |= code != 0
    raise SystemExit(int(failed))

if __name__ == '__main__':
    main()
