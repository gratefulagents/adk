#!/usr/bin/env python3
"""Generate or check the issue #11 ledger overlay without touching snapshots."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs/migration/ledger/sdk-v0.0.115/inventory.json"
MANIFEST = ROOT / "docs/migration/ledger/sdk-v0.0.115/manifest.json"
OVERLAY = ROOT / "docs/migration/ledger/issue-11-overlay.json"
REFERENCE_EVIDENCE = ROOT / "docs/verification/issue-11-pinned-reference.json"
SNAPSHOT = ROOT / "docs/migration/ledger/sdk-v0.0.115"

BASELINE_REVISION = "1dc92b73900fac74dc357a938e4b5eee6392b418"
AUDIT_RUST_REVISION = "489b1886aa760b6a2d4680d42bd3dce0a1397407"
AUDIT_SDK_CHECKOUT = "63afe2ed8cc5f13ca7469054f2c1cb812fcac801"
TOTAL = 9142
EXCLUDED = 251
UNRESOLVED = 8891


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

    counts = Counter(entry["disposition"] for entry in entries.values())
    if (len(entries), counts["excluded"], counts["unresolved"]) != (TOTAL, EXCLUDED, UNRESOLVED):
        raise SystemExit("ledger no longer matches the audited 9,142 / 251 / 8,891 disposition")
    return {
        "schema_version": 1,
        "purpose": "Issue #11 pre-implementation audit overlay. It adds current audit dispositions without changing the generated v0.0.115 baseline.",
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
            "Mark every other baseline acceptance ID unresolved; module and test associations do not close a record.",
        ],
        "counts": {
            "total": len(entries),
            "excluded": counts["excluded"],
            "unresolved": counts["unresolved"],
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
    parser.add_argument("command", choices=("generate", "check"))
    args = parser.parse_args()
    output = OVERLAY.resolve()
    reject_snapshot_path(output)
    expected = serialized(overlay())

    if args.command == "generate":
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(expected)
        print(f"wrote {output.relative_to(ROOT)}: {TOTAL:,} IDs; {EXCLUDED:,} excluded; {UNRESOLVED:,} unresolved")
        return

    if not output.exists() or output.read_bytes() != expected:
        raise SystemExit("overlay differs; run: python3 scripts/issue11-ledger.py generate")
    document = json.loads(output.read_text(encoding="utf-8"))
    counts = document["counts"]
    if counts["total"] != TOTAL or counts["excluded"] != EXCLUDED or counts["unresolved"] != UNRESOLVED:
        raise SystemExit("overlay counts are not the audited 9,142 / 251 / 8,891")
    print(f"issue #11 overlay valid: {TOTAL:,} IDs; {EXCLUDED:,} excluded; {UNRESOLVED:,} unresolved")


if __name__ == "__main__":
    main()
