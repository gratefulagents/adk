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
    expected = (ROOT / "fixtures/fileconfig/sdk-fileconfig.json").read_bytes()
    fixture = json.loads(expected)
    if fixture["sdk_revision"] != PIN or fixture["schema_version"] != 1:
        sys.exit("Unexpected fixture revision/schema")
    for attempt in range(2):
        check_sdk()
        if run([GO, "run", "-mod=readonly", "."]) != expected:
            sys.exit(f"sdk-fileconfig.json differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    cases = fixture["cases"]
    counts = {}
    for case in cases:
        for query in case["input"]["queries"]:
            op = query["operation"]
            counts[op] = counts.get(op, 0) + 1
    print(f"Verified sdk-fileconfig.json twice, byte-for-byte: {len(cases)} cases, "
          f"{sum(counts.values())} queries, {sum(bool(c['source_only']) for c in cases)} source-only cases")
    print(f"Operations: {json.dumps(counts, sort_keys=True)}")
    print(f"Verified clean SDK {PIN}")


if __name__ == "__main__":
    main()
