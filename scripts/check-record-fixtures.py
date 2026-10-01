#!/usr/bin/env python3
"""Regenerate independent typed-record expectations from the locked Go SDK."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SDK = ROOT / "repos/sdk"
PIN = "1dc92b73900fac74dc357a938e4b5eee6392b418"


def main():
    revision = subprocess.check_output(["git", "-C", str(SDK), "rev-parse", "HEAD"], text=True).strip()
    if revision != PIN or subprocess.check_output(["git", "-C", str(SDK), "status", "--porcelain"], text=True).strip():
        raise SystemExit("record oracle requires the clean pinned SDK")
    go = os.environ.get("GO", "go")
    with tempfile.TemporaryDirectory(prefix=".record-check-", dir=ROOT / "fixtures") as scratch:
        output = Path(scratch)
        subprocess.run([go, "run", str(ROOT / "fixtures/durable/generate.go"), "generate", str(output / "durable")], cwd=SDK, check=True)
        for expected in sorted((ROOT / "fixtures/durable").glob("*.json")):
            actual = output / "durable" / expected.name
            if not actual.exists() or actual.read_bytes() != expected.read_bytes():
                raise SystemExit(f"durable oracle mismatch: {expected.name}")
        subprocess.run([go, "run", str(ROOT / "fixtures/project-state/records/main.go"), "-out", str(output / "project-state.json")], cwd=SDK, check=True)
        if (output / "project-state.json").read_bytes() != (ROOT / "fixtures/project-state/records.json").read_bytes():
            raise SystemExit("project-state record oracle mismatch")
    print(f"typed-record reference fixtures verified at {PIN}")


if __name__ == "__main__":
    main()
