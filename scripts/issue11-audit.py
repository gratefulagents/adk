#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Build a deterministic review queue; never infer parity or modify dispositions."""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INPUTS = {
    "inventory": "docs/migration/ledger/sdk-v0.0.115/inventory.json",
    "overlay": "docs/migration/ledger/issue-11-overlay.json",
    "claims": "docs/migration/ledger/issue-11-rust-claims.json",
    "artifact_claims": "docs/migration/ledger/issue-11-artifact-claims.json",
}
OUTPUT = ROOT / "docs/verification/issue-11-audit.json"


def build(inventory, overlay, claims, artifact_claims=None):
    artifact_claims = artifact_claims if artifact_claims is not None else {"claims": {}}
    rust_ids = set(claims["claims"])
    artifact_ids = set(artifact_claims["claims"])
    if rust_ids & artifact_ids:
        raise ValueError("Rust/artifact claim overlap")
    verified_ids = rust_ids | artifact_ids
    records = {}
    facets = defaultdict(list)
    for category, rows in inventory["records"].items():
        for row in rows:
            key = row["acceptance_id"]
            if key in records:
                raise ValueError(f"duplicate acceptance ID: {key}")
            records[key] = (category, row)
            # Same declaration can have distinct API/config/capability obligations.
            # These are review candidates, NOT equivalent contracts.
            if row.get("source") and row.get("name"):
                facets[(row["source"], row["name"])].append(key)
    if set(records) != set(overlay["acceptance"]):
        raise ValueError("overlay must account for every inventory ID exactly once")
    if not verified_ids <= set(records):
        raise ValueError("unknown claim ID")
    allowed_artifacts = {"SDK-B9810B40B3C11D74": "artifacts", "SDK-0B2DC29E8C9CA68D": "files"}
    for key in rust_ids:
        claim = claims["claims"][key]
        if claim.get("evidence_kind", "rust_test") != "rust_test" or any(
                field in claim for field in ("artifact_paths", "review_approval_reference")):
            raise ValueError(f"Rust claim evidence kind mismatch: {key}")
    for key in artifact_ids:
        claim = artifact_claims["claims"][key]
        category, row = records[key]
        if (allowed_artifacts.get(key) != category or claim.get("category") != category
                or claim.get("evidence_kind") != "artifact_review"
                or row.get("source") != claim.get("source") or row.get("source") != "LICENSE"
                or row.get("rust_module") != "sdk::distribution"
                or claim.get("rust_module") != row.get("rust_module")
                or claim.get("ledger_record_id") != row.get("id")
                or any(field in claim for field in ("reference_test", "rust_test_identifiers", "implementation_symbols"))):
            raise ValueError(f"artifact claim evidence kind/identity mismatch: {key}")
    groups = defaultdict(lambda: {"counts": Counter(), "unresolved_ids": [],
                                  "verified_ids": [], "source_files": set()})
    counts = Counter()
    for key, (category, row) in sorted(records.items()):
        entry = overlay["acceptance"][key]
        state = entry["disposition"]
        if state not in {"verified", "excluded", "unresolved"}:
            raise ValueError(f"unknown disposition: {state}")
        if (state == "verified") != (key in verified_ids):
            raise ValueError(f"claim/disposition mismatch: {key}")
        kind = "artifact_review" if key in artifact_ids else "rust_test"
        if state == "verified" and entry.get("evidence_kind") != kind:
            raise ValueError(f"overlay evidence kind mismatch: {key}")
        if state != "verified" and entry.get("evidence_kind") is not None:
            raise ValueError(f"unverified entry has evidence kind: {key}")
        if key in artifact_ids:
            claim = artifact_claims["claims"][key]
            if (entry.get("implementation_symbols") != [] or entry.get("rust_test_identifiers") != []
                    or entry.get("target_features") != [] or entry.get("approved_divergence_reference") is not None
                    or not claim.get("artifact_paths") or not claim.get("review_approval_reference")
                    or entry.get("artifact_paths") != claim["artifact_paths"]
                    or entry.get("review_approval_reference") != claim["review_approval_reference"]):
                raise ValueError(f"artifact overlay proof mismatch: {key}")
        elif key in rust_ids and any(field in entry for field in ("artifact_paths", "review_approval_reference")):
            raise ValueError(f"Rust overlay has artifact proof: {key}")
        if (state == "excluded") != (entry["scope"] == "out_of_scope"):
            raise ValueError(f"scope/disposition mismatch: {key}")
        counts[state] += 1
        group = groups[row["verification_group_id"]]
        group["counts"][f"{category}:{state}"] += 1
        if row.get("source"):
            group["source_files"].add(row["source"])
        if state != "excluded":
            group[f"{state}_ids"].append(key)
    expected = {key: overlay["counts"].get(key, 0) for key in counts}
    if dict(counts) != expected:
        raise ValueError("overlay counts do not match entries")
    candidates = []
    for (source, name), ids in sorted(facets.items()):
        verified = [key for key in ids if key in verified_ids]
        unresolved = [key for key in ids
                      if overlay["acceptance"][key]["disposition"] == "unresolved"]
        if verified and unresolved:
            candidates.append({"source": source, "name": name,
                               "verified_facets": sorted(verified),
                               "review_required": sorted(unresolved)})
    queue = []
    for name, group in sorted(groups.items()):
        queue.append({"group": name, "counts": dict(sorted(group["counts"].items())),
                      "source_files": sorted(group["source_files"]),
                      "verified_ids": group["verified_ids"],
                      "unresolved_ids": group["unresolved_ids"]})
    return {"schema_version": 1, "sdk_revision": inventory["commit"],
            "meaning": "Review queue only. Same-declaration facets have different obligations; no automatic closure, missing-capability inference, or approved divergence.",
            "counts": dict(sorted(counts.items())), "groups": queue,
            "verified_by_evidence_kind": {"rust_test": len(rust_ids), "artifact_review": len(artifact_ids)},
            "same_declaration_review_candidates": candidates}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = {name: json.loads((ROOT / path).read_text()) for name, path in INPUTS.items()}
    report = build(**data)
    report["input_hashes"] = {
        path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
        for path in INPUTS.values()
    }
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text() != rendered:
            raise SystemExit("stale issue11 audit; run scripts/issue11-audit.py")
    else:
        OUTPUT.write_text(rendered)
    print(f"Audit: {len(report['groups'])} groups; {report['counts']}; "
          f"{len(report['same_declaration_review_candidates'])} same-declaration review candidates")


if __name__ == "__main__":
    main()
