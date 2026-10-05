#!/usr/bin/env python3
"""Check the clean SDK pin, local replacement, and two byte-identical regenerations."""
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SDK = ROOT / "repos/sdk"
PIN = "1dc92b73900fac74dc357a938e4b5eee6392b418"
GO = os.environ.get("GO", "go")


def run(args, cwd=HERE):
    result = subprocess.run(args, cwd=cwd, env=os.environ, capture_output=True)
    if result.returncode:
        sys.stderr.buffer.write(result.stdout)
        sys.stderr.buffer.write(result.stderr)
        sys.exit(result.returncode)
    return result.stdout


def check_sdk():
    revision = run(["git", "rev-parse", "HEAD"], SDK).decode().strip()
    dirty = run(["git", "status", "--porcelain", "--untracked-files=all"], SDK).strip()
    if revision != PIN or dirty:
        sys.exit(f"SDK must be clean at {PIN}; found {revision}, dirty={bool(dirty)}")


def main():
    check_sdk()
    module = json.loads(run([GO, "list", "-mod=readonly", "-m", "-json", "github.com/gratefulagents/sdk"]))
    if Path(module.get("Replace", {}).get("Dir", "")).resolve() != SDK.resolve():
        sys.exit("SDK must use the local repos/sdk replacement")
    expected = (ROOT / "fixtures/handoff/sdk-handoff-filter.json").read_bytes()
    fixture = json.loads(expected)
    if fixture["sdk_revision"] != PIN or fixture["schema_version"] != 1:
        sys.exit("Unexpected fixture revision/schema")
    for attempt in range(2):
        check_sdk()
        if run([GO, "run", "-mod=readonly", "."]) != expected:
            sys.exit(f"sdk-handoff-filter.json differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    cases = fixture["cases"]
    print(f"Verified sdk-handoff-filter.json twice, byte-for-byte: {len(cases)} cases, "
          f"{sum(len(c['input'] or []) for c in cases)} input items, "
          f"{sum(len(c['new_items'] or []) for c in cases)} new items, "
          f"{sum(len(c['output']) for c in cases)} returned items, "
          f"{sum(bool(c['source_only']) for c in cases)} source-only cases")
    expected = (ROOT / "fixtures/handoff/sdk-catalog-handoffs.json").read_bytes()
    fixture = json.loads(expected)
    if fixture["sdk_revision"] != PIN or fixture["schema_version"] != 1:
        sys.exit("Unexpected catalog fixture revision/schema")
    for attempt in range(2):
        check_sdk()
        if run([GO, "run", "-mod=readonly", ".", "catalog"]) != expected:
            sys.exit(f"sdk-catalog-handoffs.json differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    cases = fixture["cases"]
    print(f"Verified sdk-catalog-handoffs.json twice, byte-for-byte: {len(cases)} cases, "
          f"{sum(len(c['output']['specialists'] or {}) for c in cases)} specialist entries, "
          f"{sum(len(c['output']['handoffs'] or []) for c in cases)} handoffs, "
          f"{sum(bool(c['source_only']) for c in cases)} source-only cases")
    expected = (ROOT / "fixtures/handoff/sdk-subagent-selection.json").read_bytes()
    fixture = json.loads(expected)
    if fixture["sdk_revision"] != PIN or fixture["schema_version"] != 1:
        sys.exit("Unexpected subagent fixture revision/schema")
    for attempt in range(2):
        check_sdk()
        if run([GO, "run", "-mod=readonly", ".", "subagents"]) != expected:
            sys.exit(f"sdk-subagent-selection.json differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    print(f"Verified sdk-subagent-selection.json twice, byte-for-byte: {len(fixture['cases'])} public builder selections")
    expected = (ROOT / "fixtures/handoff/sdk-final-summary.json").read_bytes()
    fixture = json.loads(expected)
    if fixture["sdk_revision"] != PIN or fixture["schema_version"] != 1:
        sys.exit("Unexpected final-summary fixture revision/schema")
    for attempt in range(2):
        check_sdk()
        if run([GO, "run", "-mod=readonly", ".", "final-summary"]) != expected:
            sys.exit(f"sdk-final-summary.json differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    print(f"Verified sdk-final-summary.json twice, byte-for-byte: {len(fixture['cases'])} runner cases")
    print(f"final summary reference verified at {PIN}")
    expected = (ROOT / "fixtures/handoff/sdk-immediate-input.json").read_bytes()
    fixture = json.loads(expected)
    if fixture["sdk_revision"] != PIN or fixture["schema_version"] != 1:
        sys.exit("Unexpected immediate-input fixture revision/schema")
    for attempt in range(2):
        check_sdk()
        if run([GO, "run", "-mod=readonly", ".", "immediate-input"]) != expected:
            sys.exit(f"sdk-immediate-input.json differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    print(f"Verified sdk-immediate-input.json twice, byte-for-byte: {len(fixture['cases'])} runner cases")
    print(f"immediate input reference verified at {PIN}")
    print(f"subagent selection reference verified at {PIN}")
    print(f"handoff reference verified at {PIN}")


if __name__ == "__main__":
    main()
