#!/usr/bin/env python3
"""Build actual pinned Go SDK sources in git archive; run live Rust/Go cross-wire tests."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SHA = "1dc92b73900fac74dc357a938e4b5eee6392b418"
PROTOCOL = "github.com/modelcontextprotocol/go-sdk"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(command, cwd, env, timeout=600):
    print("+", " ".join(map(str, command)), flush=True)
    proc = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, start_new_session=True)
    try:
        output, _ = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        output, _ = proc.communicate()
        output += "\nHARNESS TIMEOUT: process group killed\n"
    finally:
        # Includes orphaned helper children on failure, not just the cargo parent.
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    print(output, flush=True)
    return {"command": list(map(str, command)), "exitCode": proc.returncode, "output": output}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=ROOT / "repos/sdk")
    parser.add_argument("--scratch", type=Path, default=ROOT.parent / "scratch/mcp-interop")
    args = parser.parse_args()
    scratch = args.scratch.resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    # Cargo/build scripts change cwd; resolve caller-provided relative toolchain paths.
    for key in ("PATH", "LD_LIBRARY_PATH"):
        if key in env:
            env[key] = os.pathsep.join(str(Path(p).resolve()) for p in env[key].split(os.pathsep) if p)
    for key in ("CARGO_HOME", "CARGO_TARGET_DIR"):
        if key in env:
            env[key] = str(Path(env[key]).resolve())
    # Match scripts/mcp-reference/run.py's archived module + isolated module cache methodology.
    cache = ROOT.parent / "scratch/mcp-reference"
    cache.mkdir(parents=True, exist_ok=True)
    env.update(GOCACHE=str(cache / "go-build"), GOMODCACHE=str(cache / "go-mod"),
               GOPATH=str(cache / "gopath"), GOTMPDIR=str(scratch), GOTOOLCHAIN="local",
               GOTELEMETRY="off", GOWORK="off", GOFLAGS="-mod=readonly")
    env.setdefault("GOROOT", "/usr/local/go")
    go = str(Path(env["GOROOT"]) / "bin/go")
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.sdk, text=True).strip()
    if actual != SHA:
        raise SystemExit(f"SDK HEAD {actual} != pinned {SHA}")
    report = {"provenance": {"repository": "https://github.com/gratefulagents/sdk", "commit": SHA,
                            "protocolModule": PROTOCOL + " v1.4.1",
                            "method": "git archive of pinned commit, helper copied into archived module, no expected observations read"},
              "passed": False, "interopPassed": False, "commands": [], "observations": {}}
    destination = ROOT / "fixtures/mcp/interop/results.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    harness = [Path(__file__).resolve(), ROOT / "scripts/mcp-interop/main.go",
               ROOT / "scripts/mcp-interop/surface_test.go", ROOT / "crates/adk-mcp/tests/interop.rs"]
    report["provenance"]["harnessSHA256"] = {str(p.relative_to(ROOT)): digest(p) for p in harness}
    report["provenance"]["rustSourceSHA256"] = {str(p.relative_to(ROOT)): digest(p) for p in sorted((ROOT / "crates/adk-mcp/src").rglob("*.rs"))}
    report["provenance"]["cargoLockSHA256"] = digest(ROOT / "Cargo.lock")
    def run(command, cwd, timeout=600):
        result = capture(command, cwd, env, timeout)
        report["commands"].append(result)
        if result["exitCode"] != 0:
            raise RuntimeError(f"command failed: {command}")
        return result["output"].strip()
    try:
        report["provenance"]["goVersion"] = run([go, "version"], ROOT)
        report["provenance"]["rustVersion"] = run(["rustc", "--version"], ROOT)
        with tempfile.TemporaryDirectory(prefix="sdk-", dir=scratch) as temporary:
            source = Path(temporary)
            archive = subprocess.check_output(["git", "archive", SHA], cwd=args.sdk)
            report["provenance"]["archiveSHA256"] = hashlib.sha256(archive).hexdigest()
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(source, filter="data")
            paths = sorted((source / "pkg/agentsdk/mcp").glob("*.go")) + [source / "go.mod", source / "go.sum"]
            report["provenance"]["sourceSHA256"] = {str(p.relative_to(source)): digest(p) for p in paths}
            module = json.loads(run([go, "list", "-m", "-json", PROTOCOL], source))
            if module["Version"] != "v1.4.1" or "Replace" in module:
                raise RuntimeError("protocol dependency is not pristine v1.4.1")
            report["provenance"]["resolvedProtocol"] = {k: module[k] for k in ("Path", "Version", "Sum", "GoModSum")}
            run([go, "mod", "verify"], source)
            protocol_dir = Path(module["Dir"])
            report["provenance"]["protocolSourceSHA256"] = {str(p.relative_to(protocol_dir)): digest(p) for p in sorted((protocol_dir / "mcp").glob("*.go"))}
            helper = source / "cmd/mcp-interop"
            helper.mkdir(parents=True)
            for name in ("main.go", "surface_test.go"):
                shutil.copyfile(ROOT / "scripts/mcp-interop" / name, helper / name)
            run([go, "test", "-count=1", "-v", "./cmd/mcp-interop"], source)
            run([go, "test", "-count=1", "-v", "-run", "^TestServerMode", "./pkg/agentsdk/mcp"], source)
            binary = source / "interop-go"
            run([go, "build", "-o", str(binary), "./cmd/mcp-interop"], source)
            report["provenance"]["helperBinarySHA256"] = digest(binary)
            results = source / "results"
            results.mkdir()
            env.update(MCP_INTEROP_GO_BINARY=str(binary), MCP_INTEROP_RESULTS=str(results))
            try:
                run(["cargo", "test", "--locked", "-p", "adk-mcp", "--test", "interop", "--", "--ignored", "--test-threads=1", "--nocapture"], ROOT)
            finally:
                report["observations"] = {p.stem: json.loads(p.read_text()) for p in sorted(results.glob("*.json"))}
            if len(report["observations"]) != 28:
                raise RuntimeError("expected all 28 cross-wire behavior observations")
            for key, paths in (("harnessSHA256", harness), ("rustSourceSHA256", sorted((ROOT / "crates/adk-mcp/src").rglob("*.rs")))):
                current = {str(p.relative_to(ROOT)): digest(p) for p in paths}
                if current != report["provenance"][key]:
                    raise RuntimeError(f"{key} changed during run; rerun against stable sources")
            if digest(ROOT / "Cargo.lock") != report["provenance"]["cargoLockSHA256"]:
                raise RuntimeError("Cargo.lock changed during cross-wire tests; rerun")
            report["interopSourcesStable"] = True
            report["interopPassed"] = True
            run(["cargo", "test", "--locked", "-p", "adk-mcp"], ROOT)
            report["regressionPassed"] = True
            report["passed"] = True
    except Exception as exc:
        report.update(passed=False, failure=str(exc))
    finally:
        # Paths/timings are intentionally real evidence, not golden comparison data.
        destination.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        print(f"Evidence: {destination.relative_to(ROOT)}; passed={report['passed']}", flush=True)
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
