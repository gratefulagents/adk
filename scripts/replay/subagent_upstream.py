#!/usr/bin/env python3
"""Execute unmodified pinned SDK child/session tests and retain auditable evidence."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
PATTERN = (
    "SubAgent|Subagent|Durable.*Child|Child.*Durable|SessionState|FailedSpawn|"
    "RestoredActiveChild|ChildRunnerCheckpoint|RestoreRequeues|ResumeRestored|"
    "CheckpointHook|ReconcileRestored|DurableResumePreserves"
)
PACKAGES = ["./internal/agent", "./pkg/agentsdk", "./pkg/agentsdk/runtime"]


def run(args, cwd):
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True).stdout.decode().strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=ROOT / "repos/sdk")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/verification/subagents")
    args = parser.parse_args()
    lock = json.loads((ROOT / "docs/migration/source-lock.json").read_text())["sdk"]
    revision = run(["git", "rev-parse", "HEAD"], args.sdk)
    if revision != lock["revision"] or run(["git", "status", "--porcelain"], args.sdk):
        raise SystemExit("reference checkout must be clean and exactly source-locked")
    for name, key in [("go.mod", "go_mod_sha256"), ("go.sum", "go_sum_sha256")]:
        if hashlib.sha256((args.sdk / name).read_bytes()).hexdigest() != lock[key]:
            raise SystemExit(f"reference {name} differs from source lock")
    go = os.environ.get("ADK_GO", "go")
    version = run([go, "version"], args.sdk)
    command = [go, "test", "-count=1", "-json", *PACKAGES, "-run", PATTERN]
    result = subprocess.run(command, cwd=args.sdk, capture_output=True)
    events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    cases = [
        {"package": e["Package"], "test": e["Test"], "outcome": e["Action"]}
        for e in events
        if "Test" in e and e["Action"] in ("pass", "fail", "skip")
    ]
    package_results = [
        {"package": e["Package"], "outcome": e["Action"]}
        for e in events
        if "Test" not in e and e["Action"] in ("pass", "fail", "skip")
    ]
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "go-upstream-tests.jsonl.gz").write_bytes(gzip.compress(result.stdout, mtime=0))
    (args.output / "go-upstream-tests.stderr.txt").write_bytes(result.stderr)
    evidence = {
        "schema_version": 1,
        "source_revision": revision,
        "source_tag": lock["tag"],
        "source_go_mod_sha256": lock["go_mod_sha256"],
        "source_go_sum_sha256": lock["go_sum_sha256"],
        "go_version": version,
        "command": ["go", *command[1:]],
        "working_directory": "repos/sdk",
        "exit_code": result.returncode,
        "raw_log": "go-upstream-tests.jsonl.gz",
        "uncompressed_log_sha256": hashlib.sha256(result.stdout).hexdigest(),
        "packages": package_results,
        "cases": sorted(cases, key=lambda item: (item["package"], item["test"])),
        "counts": {outcome: sum(c["outcome"] == outcome for c in cases) for outcome in ("pass", "fail", "skip")},
    }
    (args.output / "go-upstream-tests.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence["counts"], sort_keys=True))
    if result.returncode or len(package_results) != len(PACKAGES) or not cases:
        raise SystemExit(result.returncode or 1)
    if any(p["outcome"] != "pass" for p in package_results):
        raise SystemExit("not all reference packages passed")
    if run(["git", "status", "--porcelain"], args.sdk):
        raise SystemExit("reference execution modified the pinned checkout")


if __name__ == "__main__":
    main()
