#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Independent standard-library validation; run from the repository root."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

BASE = Path("docs/migration/ledger")
OUT = BASE / "sdk-v0.0.115"
SDK = Path("repos/sdk")
PIN = "1dc92b73900fac74dc357a938e4b5eee6392b418"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def git(*args):
    return subprocess.check_output(["git", "-C", str(SDK), *args], text=True).strip()


assert git("rev-parse", "HEAD") == PIN
assert git("rev-parse", "v0.0.115^{commit}") == PIN
assert not git("status", "--porcelain", "--untracked-files=all")
inventory = json.loads((OUT / "inventory.json").read_text())
manifest = json.loads((OUT / "manifest.json").read_text())
archive = json.loads((OUT / "sources.json").read_text())
schema = json.loads((BASE / "inventory.schema.json").read_text())
contracts = json.loads((BASE / "acceptance-contracts.json").read_text())
assert inventory["commit"] == manifest["commit"] == archive["commit"] == PIN
assert inventory["schema_version"] == manifest["schema_version"] == archive["schema_version"] == 1
assert manifest["generator_sha256"] == digest(Path("scripts/inventory/main.go").read_bytes())
assert manifest["routes_sha256"] == digest(Path("scripts/inventory/routes.json").read_bytes())
for name, expected in manifest["artifact_sha256"].items():
    assert digest((OUT / name).read_bytes()) == expected, name

sources = {row["path"]: row for row in archive["files"]}
tracked = git("ls-files").splitlines()
assert set(sources) == set(tracked)
assert len(sources) == len(archive["files"]) == manifest["tracked_files"] == 460
assert sum(name.endswith(".go") for name in sources) == manifest["parsed_go_files"] == 398
for name, source in sources.items():
    data = source["content"].encode("utf-8")
    assert data == (SDK / name).read_bytes(), name
    assert digest(data) == source["sha256"], name

records = inventory["records"]
assert set(records) == set(contracts["category_obligations"])
assert {kind: len(rows) for kind, rows in records.items()} == inventory["counts"] == manifest["counts"]
rows = [row for group in records.values() for row in group]
by_id = {row["id"]: row for row in rows}
assert len(rows) == len(by_id)
assert len(rows) == len({row["acceptance_id"] for row in rows})
common = schema["$defs"]["record"]
for row in rows:
    assert set(common["required"]) <= set(row), row["id"]
    for key, rule in common["properties"].items():
        if key not in row:
            continue
        value = row[key]
        if rule.get("type") == "string":
            assert isinstance(value, str), (row["id"], key)
        if rule.get("type") == "integer":
            assert isinstance(value, int) and value >= rule.get("minimum", 0)
        if "const" in rule:
            assert value == rule["const"], (row["id"], key)
        if "pattern" in rule:
            assert re.search(rule["pattern"], value), (row["id"], key, value)
    assert row["acceptance_id"] == "SDK-" + digest(row["id"].encode())[:16].upper()
    for key in ("parent", "target_id", "resolved_type_id", "tool_id", "api_id", "verification_group_id"):
        if key in row:
            assert row[key] in by_id, (row["id"], key)
    for key in ("member_ids", "resolved_member_ids", "source_test_ids"):
        for ref in row.get(key) or []:
            assert ref in by_id, (row["id"], key, ref)
    group = by_id[row["verification_group_id"]]
    assert group["rust_module"] == row["rust_module"]
    if "source" in row:
        assert row["source"] in sources
        if "start_byte" in row:
            data = sources[row["source"]]["content"].encode()
            assert 0 <= row["start_byte"] < row["end_byte"] <= len(data)
            assert row["line"] == 1 + data[:row["start_byte"]].count(b"\n")
    if row.get("alias"):
        assert row.get("resolved_type_id"), row["id"]

for group in records["verification_groups"]:
    tests = group["source_test_ids"]
    assert tests or group.get("coverage_gap"), group["id"]
    assert all(by_id[ref]["id"].startswith("tests:") for ref in tests)
for capability in records["capabilities"]:
    assert by_id[capability["verification_group_id"]]["source_test_ids"], capability["id"]
assert len({row["flag"] for row in records["cli_flags"]}) == 49
assert manifest["unmapped_records"] == manifest["parse_failures"] == 0
print(json.dumps({
    "validation": "passed",
    "tracked_files": len(sources),
    "mapped_records": len(rows),
    "capabilities_linked_to_concrete_go_tests": len(records["capabilities"]),
    "verification_groups_without_direct_tests": sum(not row["source_test_ids"] for row in records["verification_groups"]),
    "sdk_checkout_clean": True,
    "sdk_behavioral_tests_run": False,
}, indent=2))
