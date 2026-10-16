#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Execute item helpers in a disposable archive of the immutable SDK."""
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
        provenance = {
            'repository': 'https://github.com/gratefulagents/sdk', 'commit': SHA, 'license': 'GPL-3.0-only',
            'goVersion': subprocess.check_output([go, 'version'], env=env, text=True).strip(),
            'sourceSHA256': {p: hashlib.sha256((source / p).read_bytes()).hexdigest() for p in ['internal/agent/items.go', 'internal/agent/types_test.go', 'go.mod', 'go.sum']},
            'harnessSHA256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ['scripts/item-helpers-reference/run.py', 'scripts/item-helpers-reference/reference_test.go']},
        }
        shutil.copyfile(ROOT / 'scripts/item-helpers-reference/reference_test.go', source / 'internal/agent/native_item_helpers_test.go')
        output = source / 'observations.json'
        env['ITEM_HELPERS_OUTPUT'] = str(output)
        subprocess.run([go, 'test', '-count=1', '-v', '-run', '^(TestNativeItemHelpers|TestItemHelpersExtractText|TestItemHelpersExtractLastText|TestItemHelpersExtractToolCalls)$', './internal/agent'], cwd=source, env=env, check=True)
        serialized = json.dumps(dict(provenance=provenance, **json.loads(output.read_text())), indent=2, sort_keys=True) + '\n'
        destination = ROOT / 'fixtures/item-helpers/observations.json'
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit('Item helper observations/provenance differ')
            print('Pinned item helper observations reproduced exactly')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(serialized)
            print(f'Wrote {destination.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
