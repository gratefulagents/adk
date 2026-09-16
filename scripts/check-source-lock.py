#!/usr/bin/env python3
"""Verify attached source inputs without network access or Go dependencies."""
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
lock = json.loads((ROOT / 'docs/migration/source-lock.json').read_text())
assert lock['schema_version'] == 1
for name in ('sdk', 'platform'):
    item = lock[name]
    path = ROOT / item['path']
    revision = subprocess.check_output(['git', '-C', str(path), 'rev-parse', 'HEAD'], text=True).strip()
    assert revision == item['revision'], f'{name}: revision mismatch: {revision}'
    for filename, key in [('go.mod', 'go_mod_sha256'), ('go.sum', 'go_sum_sha256')]:
        digest = hashlib.sha256((path / filename).read_bytes()).hexdigest()
        assert digest == item[key], f'{name}: changed {filename}'
    license_copy = ROOT / 'docs/migration' / item['license_file']
    assert license_copy.read_bytes() == (path / 'LICENSE').read_bytes(), f'{name}: license mismatch'
    dirty = subprocess.check_output(['git', '-C', str(path), 'status', '--porcelain', '--untracked-files=no'], text=True)
    assert not dirty, f'{name}: tracked source changes: {dirty}'
    print(f'{name}: pinned revision, dependency hashes, clean tracked source and license verified')
sdk = lock['sdk']
tag_revision = subprocess.check_output(['git', '-C', str(ROOT / sdk['path']), 'rev-list', '-n', '1', sdk['tag']], text=True).strip()
assert tag_revision == sdk['tag_revision'], 'SDK tag moved'
platform_mod = (ROOT / lock['platform']['path'] / 'go.mod').read_text()
assert f"github.com/gratefulagents/sdk {lock['platform']['required_sdk']}" in platform_mod
platform = lock['platform']
changed = subprocess.check_output(['git', '-C', str(ROOT / platform['path']), 'diff', '--name-only', platform['epic_requested_revision'], platform['revision']], text=True).splitlines()
assert sorted(changed) == sorted(p for delta in platform['checkout_delta'] for p in delta['changed_paths']), 'unreviewed platform baseline delta'
print(f"{lock['baseline_id']}: source lock verified")
