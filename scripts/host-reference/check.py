#!/usr/bin/env python3
"""Verify the clean local SDK pin and byte-identical Go fixture regeneration."""
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SDK = ROOT / "repos/sdk"
PIN = "1dc92b73900fac74dc357a938e4b5eee6392b418"
ENV = dict(os.environ)
GO = os.environ.get("GO", "go")


def run(args, cwd=HERE):
    result = subprocess.run(args, cwd=cwd, env=ENV, capture_output=True)
    if result.returncode:
        sys.stderr.buffer.write(result.stderr)
        sys.exit(result.returncode)
    return result.stdout


def check_sdk():
    revision = run(["git", "rev-parse", "HEAD"], SDK).decode().strip()
    if revision != PIN or run(["git", "status", "--porcelain", "--untracked-files=all"], SDK).strip():
        sys.exit(f"SDK must be clean at {PIN}; found {revision}")


def main():
    check_sdk()
    module = json.loads(run([GO, "list", "-mod=readonly", "-m", "-json", "github.com/gratefulagents/sdk"]))
    if Path(module.get("Replace", {}).get("Dir", "")).resolve() != SDK.resolve():
        sys.exit("SDK must use the local repos/sdk replacement")
    expected = (ROOT / "fixtures/host-loop/sdk-chatloop.json").read_bytes()
    for attempt in range(2):
        if run([GO, "run", "-mod=readonly", "."]) != expected:
            sys.exit(f"fixture differs from pinned Go regeneration (pass {attempt + 1})")
    check_sdk()
    count = len(json.loads(expected)["cases"])
    print(f"Verified {count} ChatLoop cases twice, byte-for-byte, with clean SDK {PIN}")


if __name__ == "__main__":
    main()
