#!/usr/bin/env python3
"""Verify pinned source witnesses, complete package scope, and deterministic outputs."""
import hashlib
import json
import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path("docs/migration/platform")
closure = json.loads((ROOT / "closure.json").read_text())
environment = json.loads((ROOT / "environment.json").read_text())
assert closure["counts"] == {
    "packages": 62, "platform_packages": 26, "sdk_packages": 36,
    "production_files": 470, "test_files": 373,
}
assert not closure["test_import_added_packages"]
paths, archived, tool_paths, ids = [], [], [], set()
for package in closure["packages"]:
    for key in ("rust_module", "role_owner", "acceptance_id", "acceptance_contract", "disposition"):
        assert package[key], (package["package"], key)
    assert package["acceptance_id"] not in ids
    ids.add(package["acceptance_id"])
    assert package["implementation_status"] == "not_implemented"
    assert package["verification_status"] == "not_run"
    for source in package["files"]:
        path = pathlib.Path(source["path"])
        assert hashlib.sha256(path.read_bytes()).hexdigest() == source["sha256"], path
        assert [e["package"] for e in source["imports"]] == environment["go_imports"][str(path)], ("AST import mismatch", path)
        if not source["test"]:
            paths.append(str(path))
            if str(path).startswith("repos/gratefulagents/"):
                assert source["content"].encode() == path.read_bytes(), path
                archived.append(str(path))
            if str(path).startswith("repos/gratefulagents/internal/tools/"):
                tool_paths.append(str(path))
expected = "b42f09f901303e913d871b439ea457d83e6907daf77482e742120c481b6fbcca"
assert hashlib.sha256(("\n".join(sorted(paths)) + "\n").encode()).hexdigest() == expected
assert closure["production_path_list_sha256"] == expected
tracked_tools = subprocess.check_output(
    ["git", "-C", "repos/gratefulagents", "ls-files", "internal/tools/*.go"], text=True
).splitlines()
assert set(tool_paths) == {"repos/gratefulagents/" + p for p in tracked_tools if not p.endswith("_test.go")}
assert len(archived) == 278
for path, digest in environment["source_sha256"].items():
    assert hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest() == digest, path
for row in environment["records"]:
    lines = pathlib.Path(row["source"]).read_text().splitlines()
    assert row["expression"] in "\n".join(lines[row["line"] - 1:row["end_line"]]), row["id"]
    assert row["context_id"] in environment["contexts"]
expressions = "\n".join(r["expression"] for r in environment["records"])
for required in ("SandboxConfigEnvNames", "SecretEnvPodName", "MODE_MAX_TURNS", "providerAPIKeyEnvName", "providerEnvVarName", "AGENTRUN_PARENT_NAME", "OPENAI_OAUTH_FALLBACK_AUTH_JSON_PATHS"):
    assert required in expressions, required
metadata = "\n".join(r["expression"] for r in environment["metadata_key_and_access_expressions"])
for key in ("triggers.gratefulagents.dev/runtime-trigger-name", "triggers.gratefulagents.dev/project-uid", "triggers.gratefulagents.dev/generated-runtime", "triggers.gratefulagents.dev/review-round"):
    assert key in metadata, key
registrations = [r for r in environment["registration_expressions"] if r["source"] == "repos/gratefulagents/cmd/agent/loop.go"]
for name in ("RegisterSecurityScanTools", "RegisterMaintainerTools", "RegisterTaskOutputTool", "RegisterSlackReadTools", "RegisterPRReviewTools", "setupAskTeammateTool"):
    assert any(name in r["expression"] for r in registrations), name
regenerated = subprocess.check_output([sys.executable, "scripts/platform/closure_inventory.py"])
assert regenerated == (ROOT / "closure.json").read_bytes(), "closure drift"
env = {**os.environ, "GOROOT": "/usr/local/go", "GOTOOLCHAIN": "local"}
regenerated = subprocess.check_output(["go", "run", "scripts/platform/environment_inventory.go"], env=env)
assert regenerated == (ROOT / "environment.json").read_bytes(), "environment drift"
print("PASS: closure 62 packages / 470 production / 373 tests; no test-only package additions")
print("PASS: every closure file import list agrees with Go AST")
print("PASS: production path SHA-256 " + expected)
print(f"PASS: {len(archived)} exact platform source archives, {len(tool_paths)} internal/tools files, {len(registrations)} worker registration expressions")
print("PASS: environment/metadata witnesses, package assignments, source hashes, byte-identical regeneration")
print("Environment inventory counts: " + json.dumps(environment["counts"], sort_keys=True))
