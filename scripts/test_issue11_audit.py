import copy
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("audit", Path(__file__).with_name("issue11-audit.py"))
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.inventory = {"commit": "pin", "records": {
            "apis": [{"acceptance_id": "a", "source": "features.go", "name": "Feature",
                      "verification_group_id": "runtime"}],
            "capabilities": [{"acceptance_id": "b", "source": "features.go", "name": "Feature",
                              "verification_group_id": "runtime"}],
            "files": [{"acceptance_id": "c", "source": "cli.go",
                       "verification_group_id": "cli"}],
        }}
        self.overlay = {"acceptance": {
            "a": {"disposition": "verified", "scope": "in_scope", "evidence_kind": "rust_test"},
            "b": {"disposition": "unresolved", "scope": "in_scope"},
            "c": {"disposition": "excluded", "scope": "out_of_scope"},
        }, "counts": {"verified": 1, "unresolved": 1, "excluded": 1}}
        self.claims = {"claims": {"a": {}}}
        self.artifact_claims = {"claims": {}}

    def build(self):
        return audit.build(self.inventory, self.overlay, self.claims, self.artifact_claims)

    def test_candidates_do_not_inherit_closure(self):
        before = copy.deepcopy((self.inventory, self.overlay, self.claims))
        result = self.build()
        self.assertEqual(result["counts"]["unresolved"], 1)
        self.assertEqual(result["same_declaration_review_candidates"][0]["review_required"], ["b"])
        self.assertEqual(before, (self.inventory, self.overlay, self.claims))
        self.assertEqual(result, self.build())

    def test_missing_id_rejected(self):
        del self.overlay["acceptance"]["b"]
        with self.assertRaisesRegex(ValueError, "every inventory ID"):
            self.build()

    def test_duplicate_id_rejected(self):
        self.inventory["records"]["apis"].append(self.inventory["records"]["apis"][0])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.build()

    def test_unclaimed_verification_rejected(self):
        self.overlay["acceptance"]["b"]["disposition"] = "verified"
        with self.assertRaisesRegex(ValueError, "claim/disposition"):
            self.build()

    def test_exclusion_does_not_hide_scope(self):
        self.overlay["acceptance"]["c"]["scope"] = "in_scope"
        with self.assertRaisesRegex(ValueError, "scope/disposition"):
            self.build()

    def test_counts_must_match(self):
        self.overlay["counts"]["unresolved"] = 0
        with self.assertRaisesRegex(ValueError, "counts"):
            self.build()

    def test_unknown_claim_rejected(self):
        self.claims["claims"]["absent"] = {}
        with self.assertRaisesRegex(ValueError, "unknown claim"):
            self.build()

    def test_unknown_disposition_rejected(self):
        self.overlay["acceptance"]["b"]["disposition"] = "assumed"
        with self.assertRaisesRegex(ValueError, "unknown disposition"):
            self.build()

    def add_artifact(self):
        key = "SDK-B9810B40B3C11D74"
        row = {"acceptance_id": key, "id": "artifacts:LICENSE", "source": "LICENSE",
               "rust_module": "sdk::distribution", "verification_group_id": "distribution"}
        self.inventory["records"]["artifacts"] = [row]
        self.artifact_claims["claims"][key] = {
            "evidence_kind": "artifact_review", "category": "artifacts", "source": "LICENSE",
            "rust_module": "sdk::distribution", "ledger_record_id": "artifacts:LICENSE",
            "artifact_paths": {"LICENSE": "hash"}, "review_approval_reference": "review#/approval",
        }
        self.overlay["acceptance"][key] = {
            "disposition": "verified", "scope": "in_scope", "evidence_kind": "artifact_review",
            "artifact_paths": {"LICENSE": "hash"}, "review_approval_reference": "review#/approval",
            "implementation_symbols": [], "rust_test_identifiers": [], "target_features": [],
            "approved_divergence_reference": None,
        }
        self.overlay["counts"]["verified"] += 1
        return key

    def test_disjoint_union_counts_and_no_group_inheritance(self):
        self.add_artifact()
        self.inventory["records"]["verification_groups"] = [
            {"acceptance_id": "group", "verification_group_id": "distribution"}]
        self.overlay["acceptance"]["group"] = {"scope": "in_scope", "disposition": "unresolved"}
        self.overlay["counts"]["unresolved"] += 1
        result = self.build()
        self.assertEqual(result["counts"], {"verified": 2, "unresolved": 2, "excluded": 1})
        self.assertEqual(result["verified_by_evidence_kind"], {"rust_test": 1, "artifact_review": 1})
        group = next(row for row in result["groups"] if row["group"] == "distribution")
        self.assertEqual(group["unresolved_ids"], ["group"])

    def test_rust_artifact_overlap_rejected(self):
        key = self.add_artifact()
        self.claims["claims"][key] = {}
        with self.assertRaisesRegex(ValueError, "overlap"):
            self.build()

    def test_claim_and_overlay_kinds_must_agree(self):
        key = self.add_artifact()
        for candidate in (key, "a"):
            entry = self.overlay["acceptance"][candidate]
            original = entry["evidence_kind"]
            for value in (None, "wrong", "artifact_review" if candidate == "a" else "rust_test"):
                with self.subTest(candidate=candidate, value=value):
                    entry["evidence_kind"] = value
                    with self.assertRaisesRegex(ValueError, "evidence kind"):
                        self.build()
            entry["evidence_kind"] = original
        self.claims["claims"]["a"]["evidence_kind"] = "artifact_review"
        with self.assertRaisesRegex(ValueError, "evidence kind"):
            self.build()

    def test_artifact_cannot_masquerade_as_source_test_or_excluded(self):
        key = self.add_artifact()
        claim = self.artifact_claims["claims"][key]
        original = copy.deepcopy(claim)
        for field, value in (("category", "tests"), ("source", "fixture_test.go"),
                             ("reference_test", "fake"), ("rust_test_identifiers", [])):
            with self.subTest(field=field):
                claim.clear()
                claim.update({**original, field: value})
                with self.assertRaisesRegex(ValueError, "identity mismatch"):
                    self.build()
        claim.clear()
        claim.update(original)
        self.overlay["acceptance"][key]["scope"] = "out_of_scope"
        with self.assertRaisesRegex(ValueError, "scope/disposition"):
            self.build()

    def test_artifact_proof_fields_cannot_include_native_evidence(self):
        key = self.add_artifact()
        original = copy.deepcopy(self.overlay["acceptance"][key])
        for field, value in (("implementation_symbols", ["fake.rs::symbol"]),
                             ("rust_test_identifiers", ["fake"]), ("target_features", ["all"]),
                             ("approved_divergence_reference", "divergence"),
                             ("artifact_paths", {}), ("review_approval_reference", "wrong")):
            with self.subTest(field=field):
                self.overlay["acceptance"][key] = {**original, field: value}
                with self.assertRaisesRegex(ValueError, "artifact overlay proof"):
                    self.build()

    def test_unverified_entry_cannot_inherit_evidence_kind(self):
        self.overlay["acceptance"]["b"]["evidence_kind"] = "artifact_review"
        with self.assertRaisesRegex(ValueError, "unverified entry"):
            self.build()
