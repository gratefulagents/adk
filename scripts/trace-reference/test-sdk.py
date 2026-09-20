#!/usr/bin/env python3
"""Execute pinned public SDK tests and retain per-test reference evidence, not Rust closure."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT / "repos/sdk"
PIN = "1dc92b73900fac74dc357a938e4b5eee6392b418"

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    baseline = (ROOT / "docs/migration/ledger/sdk-v0.0.115").resolve()
    if args.output.resolve() == baseline or baseline in args.output.resolve().parents:
        sys.exit("refusing to overwrite immutable baseline")
    revision = subprocess.check_output(["git", "-C", str(SDK), "rev-parse", "HEAD"], text=True).strip()
    if revision != PIN or subprocess.check_output(["git", "-C", str(SDK), "status", "--porcelain"], text=True).strip():
        sys.exit("reference must be clean at the authoritative pin")
    go = os.environ.get("GO", "go")
    command = [go, "test", "-count=1", "-json", "./pkg/agentsdk/..."]
    result = subprocess.run(command, cwd=SDK, text=True, capture_output=True)
    tests = {}
    packages = {}
    for line in result.stdout.splitlines():
        event = json.loads(line)
        action = event["Action"]
        if action in ("pass", "fail", "skip"):
            package = event["Package"]
            if "Test" in event:
                tests[package + "/" + event["Test"]] = action
            else:
                packages[package] = action
    document = {
        "sdk_revision": revision,
        "go_version": subprocess.check_output([go, "version"], text=True).strip(),
        "command": "go test -count=1 -json ./pkg/agentsdk/...",
        "exit_code": result.returncode,
        "baseline_inventory_sha256": hashlib.sha256((baseline / "inventory.json").read_bytes()).hexdigest(),
        "purpose": "Independent Go reference execution only; does not verify any Rust acceptance ID.",
        "counts": dict(sorted(Counter(tests.values()).items())),
        "packages": dict(sorted(packages.items())),
        "tests": dict(sorted(tests.items())),
        "stderr": result.stderr,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"exit_code": result.returncode, "counts": document["counts"], "packages": len(packages)}))
    sys.exit(result.returncode)

if __name__ == "__main__":
    main()
