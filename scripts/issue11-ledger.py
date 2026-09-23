#!/usr/bin/env python3
"""Generate or check the issue #11 ledger overlay without touching snapshots."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs/migration/ledger/sdk-v0.0.115/inventory.json"
MANIFEST = ROOT / "docs/migration/ledger/sdk-v0.0.115/manifest.json"
OVERLAY = ROOT / "docs/migration/ledger/issue-11-overlay.json"
REFERENCE_EVIDENCE = ROOT / "docs/verification/issue-11-pinned-reference.json"
RUST_EVIDENCE = ROOT / "docs/verification/issue-11-rust-evidence.json"
RUST_CLAIMS = ROOT / "docs/migration/ledger/issue-11-rust-claims.json"
SNAPSHOT = ROOT / "docs/migration/ledger/sdk-v0.0.115"

BASELINE_REVISION = "1dc92b73900fac74dc357a938e4b5eee6392b418"
AUDIT_RUST_REVISION = "489b1886aa760b6a2d4680d42bd3dce0a1397407"
AUDIT_SDK_CHECKOUT = "63afe2ed8cc5f13ca7469054f2c1cb812fcac801"
TOTAL = 9142
EXCLUDED = 251
RETAINED = 8891
RUST_INPUTS = ["crates/adk/src/tracing_runtime.rs", "crates/adk/tests/tracing_runtime.rs",
               "crates/adk/examples/features.rs", "crates/adk-runtime/src/guardrails.rs", "crates/adk-runtime/tests/guardrails.rs",
               "crates/adk/src/tracing.rs", "crates/adk/tests/tracing.rs",
               "crates/adk/tests/tracing_sinks.rs", "crates/adk/src/lib.rs", "Cargo.lock", "crates/adk/Cargo.toml", "crates/adk/src/tracestore.rs",
               "crates/adk/tests/tracestore.rs", "crates/adk/src/telemetry.rs",
               "crates/adk/src/telemetry_spans.rs", "crates/adk/src/tracewriter.rs",
               "crates/adk/tests/telemetry.rs", "crates/adk/tests/telemetry_spans.rs",
               "fixtures/tracestore/sdk-otel.json", "scripts/trace-reference/otel.go",
               "scripts/trace-reference/go.mod", "scripts/trace-reference/go.sum",
               "crates/adk-codec/Cargo.toml", "crates/adk-codec/src/snapshots.rs",
               "crates/adk-codec/src/dto.rs", "crates/adk-codec/src/lib.rs",
               "crates/adk-codec/tests/request_snapshot.rs", "crates/adk/tests/tracewriter.rs",
               "crates/adk-codec/tests/native_snapshot.rs", "crates/adk-codec/src/approval.rs",
               "crates/adk-runtime/src/durable.rs", "crates/adk-runtime/tests/durable.rs",
               "crates/adk-runtime/tests/runner.rs", "crates/adk/src/observability.rs",
               "crates/adk/tests/observability.rs",
               "crates/adk-runtime/src/tracing.rs", "crates/adk-runtime/src/runner.rs",
               "crates/adk-core/src/contracts.rs", "crates/adk-core/src/types.rs",
               "crates/adk-core/tests/contracts.rs", "crates/adk-providers/src/wire.rs",
               "crates/adk-providers/tests/contracts.rs", "crates/adk-providers/tests/http.rs",
               "crates/adk-runtime/Cargo.toml",
               "fixtures/tracestore/sdk-writer.json", "scripts/trace-reference/writer.go",
               "fixtures/tracestore/sdk-store.json", "scripts/trace-reference/main.go",
               "scripts/trace-reference/check.py",
               "crates/adk/src/telemetry_stdout.rs", "crates/adk/tests/telemetry_stdout.rs",
               "crates/adk/tests/telemetry_defaults.rs", "scripts/trace-reference/stdout.go",
               "fixtures/tracestore/sdk-stdout.json",
               str(RUST_CLAIMS.relative_to(ROOT))]


def read_json(path: Path) -> object:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def excluded(record: dict[str, object]) -> tuple[bool, str | None]:
    module = record["rust_module"]
    source = record.get("source")
    if module == "sdk_cli":
        return True, "grateful-agent-run CLI is outside issue #11 scope"
    if module == "sdk_evals" or module.startswith("sdk_evals::"):
        return True, "evaluation and automation adapters are outside issue #11 scope"
    if source == ".github/workflows/terminal-bench.yml":
        return True, "Terminal-Bench workflow is outside issue #11 scope despite sdk::ci routing"
    return False, None


def module_reference_map() -> dict[str, dict[str, object]]:
    """Association-only references; none supplies per-record semantic closure."""
    return {
        "core": {
            "facade_feature": "always available",
            "api_paths": ["crates/adk/src/lib.rs", "crates/adk-core/src/contracts.rs", "crates/adk-core/src/types.rs"],
            "test_paths": ["crates/adk-core/tests/contracts.rs"],
        },
        "runtime": {
            "facade_feature": "runtime",
            "api_paths": ["crates/adk/src/lib.rs", "crates/adk-runtime/src/runner.rs"],
            "test_paths": ["crates/adk-runtime/tests/runner.rs", "crates/adk/tests/tool_runtime.rs"],
        },
        "tools": {
            "facade_feature": "tools",
            "api_paths": ["crates/adk/src/lib.rs", "crates/adk-tools/src/bundle.rs"],
            "test_paths": ["crates/adk-tools/tests/registry.rs", "crates/adk-tools/tests/registry_matrix.rs"],
        },
        "providers": {
            "facade_feature": "providers or providers-runtime",
            "api_paths": ["crates/adk/src/lib.rs", "crates/adk-providers/src/factory.rs"],
            "test_paths": ["crates/adk-providers/tests/contracts.rs", "crates/adk-providers/tests/retry_parity.rs"],
        },
        "durable": {
            "facade_feature": "durable",
            "api_paths": ["crates/adk/src/lib.rs", "crates/adk-durable/src/lib.rs"],
            "test_paths": ["crates/adk-durable/tests/contracts.rs", "crates/adk-runtime/tests/durable.rs"],
        },
        "mcp": {
            "facade_feature": "mcp",
            "api_paths": ["crates/adk/src/lib.rs", "crates/adk-mcp/src/lib.rs"],
            "test_paths": ["crates/adk-mcp/tests/interop.rs", "crates/adk-mcp/tests/reference.rs"],
        },
        "execution": {
            "facade_feature": "execution",
            "api_paths": ["crates/adk/src/execution.rs", "crates/adk-security/src/policy.rs"],
            "test_paths": ["crates/adk/tests/execution.rs", "crates/adk-security/tests/security.rs"],
        },
        "observability": {
            "facade_feature": "observability or otel",
            "api_paths": ["crates/adk/src/observability.rs"],
            "test_paths": [],
        },
    }


def verify_rust() -> None:
    claims = read_json(RUST_CLAIMS)
    fixture_check = subprocess.run([sys.executable, str(ROOT / "scripts/trace-reference/check.py")],
                                   cwd=ROOT, text=True, capture_output=True, check=True)
    command = [os.environ.get("CARGO", "cargo"), "test", "--locked", "-p", "adk", "-p", "adk-codec", "-p", "adk-runtime",
               "--features", "otel", "--test", "tracestore", "--test", "telemetry", "--test", "telemetry_spans",
               "--test", "tracewriter", "--test", "request_snapshot", "--test", "native_snapshot", "--lib",
               "--test", "telemetry_stdout", "--test", "telemetry_defaults",
               "--test", "tracing", "--test", "tracing_sinks",
               "--test", "guardrails", "--test", "runner", "--test", "tracing_runtime"]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    print(result.stdout, end="")
    print(result.stderr, end="")
    if result.returncode:
        raise SystemExit(result.returncode)
    tests = sorted({test for claim in claims["claims"].values()
                    for test in claim["rust_test_identifiers"]})
    for test in tests:
        if f"test {test} ... ok" not in result.stdout:
            raise SystemExit(f"required Rust regression did not pass: {test}")
    compiler = subprocess.check_output([os.environ.get("RUSTC", "rustc"), "-vV"], text=True)
    if "host: x86_64-unknown-linux-gnu" not in compiler or "release: 1.88.0\n" not in compiler:
        raise SystemExit("these claims require the recorded Linux x86_64 verification target")
    evidence = {
        "schema_version": 1,
        "baseline_revision": BASELINE_REVISION,
        "compiler": compiler,
        "command": ["cargo", *command[1:]],
        "exit_code": result.returncode,
        "reference_fixture_check": fixture_check.stdout,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "files": {path: sha256(ROOT / path) for path in RUST_INPUTS},
        "passed_tests": tests,
    }
    RUST_EVIDENCE.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")


def apply_rust_evidence(entries, records, reference):
    claims = read_json(RUST_CLAIMS)
    evidence = read_json(RUST_EVIDENCE)
    if claims["baseline_revision"] != BASELINE_REVISION or evidence["baseline_revision"] != BASELINE_REVISION:
        raise SystemExit("Rust evidence must use the authoritative baseline")
    if f"trace-store reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust evidence requires independently executed pinned trace fixtures")
    if evidence["exit_code"] != 0:
        raise SystemExit("failed Rust commands cannot verify a ledger entry")
    if set(evidence["files"]) != set(RUST_INPUTS):
        raise SystemExit("Rust evidence must bind all implementation/test/claim/dependency inputs")
    if "host: x86_64-unknown-linux-gnu" not in evidence["compiler"] or "release: 1.88.0\n" not in evidence["compiler"]:
        raise SystemExit("Rust evidence requires the pinned compiler and Linux verification target")
    for path, expected in evidence["files"].items():
        if sha256(ROOT / path) != expected:
            raise SystemExit(f"stale Rust evidence for {path}; run verify-rust again")
    indexed = {record["acceptance_id"]: record for rows in records.values() for record in rows}
    for acceptance_id, claim in claims["claims"].items():
        if acceptance_id not in entries or entries[acceptance_id]["scope"] != "in_scope":
            raise SystemExit(f"cannot verify unknown/excluded acceptance ID: {acceptance_id}")
        record = indexed[acceptance_id]
        if (record.get("source"), record.get("name")) != (claim["source"], claim["source_name"]):
            raise SystemExit(f"claim identity mismatch: {acceptance_id}")
        if reference["tests"].get(claim["reference_test"]) != "pass":
            raise SystemExit(f"required pinned regression is not passing: {acceptance_id}")
        if not claim["rust_test_identifiers"] or not claim["implementation_symbols"] or not claim["rationale"]:
            raise SystemExit(f"incomplete claim: {acceptance_id}")
        for implementation in claim["implementation_symbols"]:
            path, separator, symbol = implementation.partition("::")
            if not separator or not symbol or not path.endswith(".rs") or path not in evidence["files"]:
                raise SystemExit(f"implementation must name a hash-bound Rust source: {implementation}")
        for test in claim["rust_test_identifiers"]:
            if test not in evidence["passed_tests"] or f"test {test} ... ok" not in evidence["stdout"]:
                raise SystemExit(f"missing successful Rust regression: {test}")
        entries[acceptance_id].update({
            "disposition": "verified",
            "disposition_rationale": claim["rationale"],
            "implementation_symbols": claim["implementation_symbols"],
            "rust_test_identifiers": claim["rust_test_identifiers"],
            "command_log": [str(RUST_EVIDENCE.relative_to(ROOT))],
            "target_features": claim["target_features"],
            "verification_status": "passed",
            "semantic_closure": "verified",
        })


def overlay() -> dict[str, object]:
    inventory = read_json(LEDGER)
    manifest = read_json(MANIFEST)
    assert isinstance(inventory, dict)
    assert isinstance(manifest, dict)
    if inventory["sdk_version"] != "v0.0.115" or inventory["commit"] != BASELINE_REVISION:
        raise SystemExit("issue #11 overlay only accepts the v0.0.115 generated baseline")
    if manifest["commit"] != BASELINE_REVISION:
        raise SystemExit("manifest does not match the v0.0.115 generated baseline")
    records = inventory["records"]
    assert isinstance(records, dict)
    tests = records["tests"]
    groups = records["verification_groups"]
    assert isinstance(tests, list)
    assert isinstance(groups, list)
    upstream_regression_reference_index = {
        row["id"]: {
            "source": row["source"],
            "line": row["line"],
            "name": row["name"],
            "signature": row["signature"],
        }
        for row in sorted(tests, key=lambda item: item["id"])
    }
    upstream_regression_reference_groups = {
        row["id"]: sorted(row["source_test_ids"])
        for row in sorted(groups, key=lambda item: item["id"])
    }

    reference = read_json(REFERENCE_EVIDENCE)
    if reference["sdk_revision"] != BASELINE_REVISION or reference["exit_code"] != 0:
        raise SystemExit("reference evidence must come from successful pinned SDK tests")
    if reference["baseline_inventory_sha256"] != sha256(LEDGER):
        raise SystemExit("reference evidence was generated for another baseline")
    entries: dict[str, dict[str, object]] = {}
    category_counts: dict[str, Counter[str]] = {}
    for category in sorted(records):
        rows = records[category]
        assert isinstance(rows, list)
        category_counts[category] = Counter()
        for record in sorted(rows, key=lambda item: item["acceptance_id"]):
            is_excluded, rationale = excluded(record)
            disposition = "excluded" if is_excluded else "unresolved"
            category_counts[category][disposition] += 1
            acceptance_id = record["acceptance_id"]
            assert acceptance_id not in entries
            entries[acceptance_id] = {
                "ledger_record_id": record["id"],
                "category": category,
                "source": record.get("source"),
                "proposed_rust_module": record["rust_module"],
                "scope": "out_of_scope" if is_excluded else "in_scope",
                "disposition": disposition,
                "disposition_rationale": rationale,
                "implementation_symbols": [],
                "rust_test_identifiers": [],
                "upstream_regression_reference_group": record["verification_group_id"],
                "command_log": [],
                "target_features": [],
                "verification_status": "not_run",
                "semantic_closure": "excluded" if is_excluded else "unresolved",
                "approved_divergence_reference": None,
            }
            if category == "tests" and record.get("source"):
                package = "github.com/gratefulagents/sdk/" + str(Path(record["source"]).parent)
                key = package + "/" + record["name"]
                if key in reference["tests"]:
                    entries[acceptance_id]["pinned_reference_verification"] = {
                        "evidence_path": str(REFERENCE_EVIDENCE.relative_to(ROOT)),
                        "test": key,
                        "status": reference["tests"][key],
                        "rust_semantic_closure": False,
                    }

    apply_rust_evidence(entries, records, reference)
    counts = Counter(entry["disposition"] for entry in entries.values())
    category_counts = {category: Counter() for category in records}
    for entry in entries.values():
        category_counts[entry["category"]][entry["disposition"]] += 1
    if (len(entries), counts["excluded"], counts["unresolved"] + counts["verified"]) != (TOTAL, EXCLUDED, RETAINED):
        raise SystemExit("ledger no longer has 9,142 IDs, 251 exclusions and 8,891 retained obligations")
    return {
        "schema_version": 1,
        "purpose": "Issue #11 current evidence overlay retaining its historical audit snapshot. Explicit Rust claims require passing command evidence and unchanged source/test hashes; the generated v0.0.115 baseline is immutable.",
        "baseline": {
            "sdk_version": inventory["sdk_version"],
            "sdk_revision": BASELINE_REVISION,
            "inventory_path": str(LEDGER.relative_to(ROOT)),
            "inventory_sha256": sha256(LEDGER),
            "sources_sha256": manifest["artifact_sha256"]["sources.json"],
            "record_count": sum(len(rows) for rows in records.values()),
        },
        "pinned_reference_execution": {
            "evidence_path": str(REFERENCE_EVIDENCE.relative_to(ROOT)),
            "evidence_sha256": sha256(REFERENCE_EVIDENCE),
            "sdk_revision": reference["sdk_revision"],
            "counts": reference["counts"],
            "rust_semantic_closure": False,
        },
        "audit_snapshot": {
            "rust_revision": AUDIT_RUST_REVISION,
            "sdk_checkout_revision": AUDIT_SDK_CHECKOUT,
            "sdk_checkout_tag": "v0.0.116",
            "pin_drift": "The local repos/sdk checkout was v0.0.116, not the v0.0.115 baseline. The overlay is keyed only to the archived baseline.",
            "status": "pre_implementation_snapshot",
        },
        "disposition_rules": [
            "Exclude sdk_cli records.",
            "Exclude sdk_evals and sdk_evals::* records.",
            "Exclude .github/workflows/terminal-bench.yml even though its route is sdk::ci.",
            "Verify only explicit source-identified claims with passing pinned reference and Rust evidence; all other retained IDs remain unresolved.",
        ],
        "counts": {
            "total": len(entries),
            "excluded": counts["excluded"],
            "unresolved": counts["unresolved"],
            "verified": counts["verified"],
            "by_category": {category: dict(sorted(counter.items())) for category, counter in sorted(category_counts.items())},
        },
        "module_reference_map": module_reference_map(),
        "module_reference_map_note": "This is an API/test association index, not implementation evidence, a test result, approved divergence, or semantic closure for any acceptance ID.",
        "upstream_regression_reference_groups": upstream_regression_reference_groups,
        "upstream_regression_reference_index": upstream_regression_reference_index,
        "upstream_regression_reference_note": "An entry points to a group of candidate baseline test record IDs; the index resolves each ID. They are upstream regression references, not executed tests and not proof of semantic closure.",
        "acceptance": entries,
    }


def serialized(document: dict[str, object]) -> bytes:
    # One acceptance entry per line keeps the exhaustive index reviewable.
    header = dict(document)
    entries = header.pop("acceptance")
    text = json.dumps(header, indent=2, sort_keys=True)
    rows = ["    " + json.dumps(key) + ": " + json.dumps(value, sort_keys=True)
            for key, value in sorted(entries.items())]
    return (text[:-2] + ',\n  "acceptance": {\n' + ',\n'.join(rows)
            + '\n  }\n}\n').encode()


def reject_snapshot_path(path: Path) -> None:
    resolved = path.resolve()
    if resolved == SNAPSHOT.resolve() or SNAPSHOT.resolve() in resolved.parents:
        raise SystemExit("refusing to write or check a generated baseline snapshot")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "check", "verify-rust"))
    args = parser.parse_args()
    if args.command == "verify-rust":
        verify_rust()
        return
    output = OVERLAY.resolve()
    reject_snapshot_path(output)
    expected = serialized(overlay())

    if args.command == "generate":
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(expected)
        print(f"wrote {output.relative_to(ROOT)}")
        return

    if not output.exists() or output.read_bytes() != expected:
        raise SystemExit("overlay differs; run: python3 scripts/issue11-ledger.py generate")
    document = json.loads(output.read_text(encoding="utf-8"))
    counts = document["counts"]
    if counts["total"] != TOTAL or counts["excluded"] != EXCLUDED or counts["unresolved"] + counts["verified"] != RETAINED:
        raise SystemExit("overlay counts do not preserve the audited scope")
    print(f"issue #11 overlay valid: {TOTAL:,} IDs; {EXCLUDED:,} excluded; {counts['verified']:,} verified; {counts['unresolved']:,} unresolved")


if __name__ == "__main__":
    main()
