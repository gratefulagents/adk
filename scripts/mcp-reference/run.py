#!/usr/bin/env python3
"""Run actual pinned SDK code in a disposable git archive, never in repos/sdk."""
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

SHA = "1dc92b73900fac74dc357a938e4b5eee6392b418"
ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(*args, cwd=None, env=None):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare, do not overwrite observations")
    parser.add_argument("--baseline-tests", action="store_true", help="also run original SDK package tests")
    parser.add_argument("--sdk", type=Path, default=ROOT / "repos/sdk")
    parser.add_argument("--scratch", type=Path, default=ROOT.parent / "scratch/mcp-reference")
    args = parser.parse_args()
    scratch = args.scratch.resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(GOCACHE=str(scratch / "go-build"), GOMODCACHE=str(scratch / "go-mod"),
               GOPATH=str(scratch / "gopath"), GOTMPDIR=str(scratch), GOTOOLCHAIN="local",
               GOTELEMETRY="off", GOWORK="off", GOFLAGS="-mod=readonly")
    env.setdefault("GOROOT", "/usr/local/go")
    go = str(Path(env["GOROOT"]) / "bin/go")
    go_version = run(go, "version", env=env).strip()
    actual_sha = run("git", "rev-parse", "HEAD", cwd=args.sdk).strip()
    if actual_sha != SHA:
        raise SystemExit(f"SDK HEAD {actual_sha} != pinned {SHA}")
    fixture = ROOT / "fixtures/mcp/reference"
    with tempfile.TemporaryDirectory(prefix="sdk-", dir=scratch) as temporary:
        source = Path(temporary)
        archive = subprocess.check_output(["git", "archive", SHA], cwd=args.sdk)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter="data")
        package = source / "pkg/agentsdk/mcp"
        source_paths = (sorted(package.glob("*.go"))
                        + sorted((source / "pkg/agentsdk/tools/web").glob("*.go"))
                        + [source / "go.mod", source / "go.sum"])
        provenance = {
            "repository": "https://github.com/gratefulagents/sdk",
            "commit": SHA,
            "goVersion": go_version,
            "module": "github.com/gratefulagents/sdk",
            "protocolModule": "github.com/modelcontextprotocol/go-sdk v1.4.1",
            "sourceSHA256": {str(p.relative_to(source)): digest(p) for p in source_paths},
            "harnessSHA256": {str(p.relative_to(ROOT)): digest(p) for p in [Path(__file__).resolve(), ROOT / "scripts/mcp-reference/reference_test.go", fixture / "inputs.json"]},
            "command": "go test -count=1 -run ^TestReferenceCorpus$ -v ./pkg/agentsdk/mcp",
            "normalization": "JSON key order ignored; saved paths -> <blob>; blob bytes/SHA256/mode/confinement retained; blob errors -> <blob-error> with Go diagnostics retained; long text -> bytes/SHA256; no expected observations read by Go",
        }
        shutil.copyfile(ROOT / "scripts/mcp-reference/reference_test.go", package / "reference_corpus_test.go")
        output = source / "observations.json"
        env.update(MCP_REFERENCE_INPUT=str(fixture / "inputs.json"), MCP_REFERENCE_OUTPUT=str(output))
        command = [go, "test", "-count=1", "-run", "^TestReferenceCorpus$", "-v", "./pkg/agentsdk/mcp"]
        print("+", " ".join(command), flush=True)
        subprocess.run(command, cwd=source, env=env, check=True)
        observations = json.loads(output.read_text())
        observations["provenance"] = provenance
        serialized = json.dumps(observations, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
        destination = fixture / "observations.json"
        if args.check:
            if destination.read_text() != serialized:
                raise SystemExit("observations/provenance differ; inspect before regenerating")
            print("Pinned Go observations and provenance reproduced exactly", flush=True)
        else:
            destination.write_text(serialized)
            print(f"Wrote {destination.relative_to(ROOT)}", flush=True)
        if args.baseline_tests:
            command = [go, "test", "-count=1", "-json", "./pkg/agentsdk/mcp"]
            print("+", " ".join(command), flush=True)
            completed = subprocess.run(command, cwd=source, env=env, text=True, stdout=subprocess.PIPE)
            (scratch / "baseline.log").write_text(completed.stdout)
            events = [json.loads(line) for line in completed.stdout.splitlines()]
            tests = {e["Test"]: e["Action"] for e in events
                     if "Test" in e and e["Action"] in ("pass", "fail", "skip")}
            report = {
                "commit": SHA, "goVersion": go_version,
                "command": "go test -count=1 -json ./pkg/agentsdk/mcp",
                "observationsSHA256": hashlib.sha256(serialized.encode()).hexdigest(),
                "exitCode": completed.returncode, "tests": tests,
            }
            report_text = json.dumps(report, indent=2, sort_keys=True) + "\n"
            report_path = fixture / "baseline-tests.json"
            if args.check:
                if report_path.read_text() != report_text:
                    raise SystemExit(f"baseline test evidence changed; inspect {scratch / 'baseline.log'}")
            else:
                report_path.write_text(report_text)
            print(f"Baseline: {len(tests)} test/subtest outcomes; exit {completed.returncode}; log {scratch / 'baseline.log'}", flush=True)
            completed.check_returncode()


if __name__ == "__main__":
    main()
