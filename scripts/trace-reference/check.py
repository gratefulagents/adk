#!/usr/bin/env python3
"""Compare trace fixtures with an independently executed authoritative SDK."""
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT / "repos/sdk"
PIN = "1dc92b73900fac74dc357a938e4b5eee6392b418"

def main():
    revision = subprocess.check_output(["git", "-C", str(SDK), "rev-parse", "HEAD"], text=True).strip()
    if revision != PIN:
        sys.exit(f"reference checkout mismatch: expected {PIN}, got {revision}")
    if subprocess.check_output(["git", "-C", str(SDK), "status", "--porcelain"], text=True).strip():
        sys.exit("reference checkout must be clean")
    subprocess.run([sys.executable, str(ROOT / "scripts/inventory/validate.py")], check=True)
    result = subprocess.check_output([os.environ.get("GO", "go"), "run", "-mod=readonly", "."], cwd=Path(__file__).parent)
    expected = json.loads((ROOT / "fixtures/tracestore/sdk-store.json").read_text())
    actual = json.loads(result)
    if actual != expected:
        sys.exit("pinned reference trace fixture mismatch")
    print(f"trace-store reference verified at {revision}")

if __name__ == "__main__":
    main()
