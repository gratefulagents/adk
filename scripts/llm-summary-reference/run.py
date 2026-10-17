#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reproduce LLM summary observations from an unmodified, disposable SDK archive."""
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
UPSTREAM_TESTS = [
    'TestApplyLLMSummaryToPlanUsesModelSummary',
    'TestApplyLLMSummaryToPlanFallsBackOnModelError',
    'TestFlattenRunItemsForSummaryTruncatesMiddleOut',
    'TestMaybeCompactRunItemsPreservesOriginalTaskAndAddsSummary',
    'TestSummarizeCompactedHistoryBuildsReadableHandoff',
    'TestSummarizeCompactedHistoryMergesPriorSummaryLikeClaw',
    'TestMaybeCompactRunItemsNoopBelowThreshold',
    'TestMaybeCompactRunItemsForRequestCountsOverhead',
    'TestMaybeCompactRunItemsPreservesToolPairs',
    'TestMaybeCompactRunItemsReducesFurtherToMeetTargetTokens',
    'TestCompactionRejectsSummaryThatGrowsTokens',
    'TestTruncateMiddleBytesBoundedUTF8',
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    env = dict(os.environ, GOROOT=os.environ.get('GOROOT', '/usr/local/go'),
               GOTOOLCHAIN='local', GOTELEMETRY='off', GOWORK='off',
               GOFLAGS='-mod=readonly', GOPROXY='off', GOSUMDB='off')
    env.setdefault('GOCACHE', '/workspace/scratch/go-cache')
    env.setdefault('GOMODCACHE', '/workspace/scratch/go/pkg/mod')
    env['PATH'] = str(Path(env['GOROOT']) / 'bin') + ':/usr/bin:/bin'
    go = str(Path(env['GOROOT']) / 'bin/go')
    with tempfile.TemporaryDirectory() as temp:
        source = Path(temp)
        archive = subprocess.check_output(['git', 'archive', SHA], cwd=ROOT / 'repos/sdk')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter='data')
        paths = ['internal/agent/' + name for name in [
            'compaction_llm.go', 'compaction_llm_test.go', 'history_compaction.go',
            'history_compaction_test.go', 'audit_fixes_test.go', 'stream.go',
            'tool_output_spill_test.go', 'model.go', 'model_settings.go',
            'items.go', 'usage.go', 'run_config.go',
        ]] + ['go.mod', 'go.sum', 'LICENSE']
        provenance = {
            'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA,
            'license': 'GPL-3.0-only',
            'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
            'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in paths},
            'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in [
                'scripts/llm-summary-reference/run.py',
                'scripts/llm-summary-reference/reference_test.go',
            ]},
        }
        shutil.copyfile(ROOT / 'scripts/llm-summary-reference/reference_test.go',
                        source / 'internal/agent/native_llm_summary_reference_test.go')
        output = source / 'observations.json'
        env['LLM_SUMMARY_OUTPUT'] = str(output)
        names = ['TestNativeLLMSummaryReference'] + UPSTREAM_TESTS
        command = [go, 'test', '-count=1', '-json', '-run', '^(' + '|'.join(names) + ')$', './internal/agent']
        run = subprocess.run(command, cwd=source, env=env, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT)
        passed = set()
        for line in run.stdout.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                print(line)
                continue
            if event.get('Action') == 'pass' and 'Test' in event:
                passed.add(event['Test'])
            if 'Output' in event:
                print(event['Output'], end='')
        run.check_returncode()
        if set(names) - passed:
            raise SystemExit('Selected tests did not pass: ' + ', '.join(sorted(set(names) - passed)))
        result = dict(provenance=provenance, upstreamTestsPassed=UPSTREAM_TESTS,
                      **json.loads(output.read_text()))
        serialized = json.dumps(result, indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/llm-summary/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('LLM summary observations/provenance differ')
            print('Pinned LLM summary observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
