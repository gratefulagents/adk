import copy
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("issue11_ledger", Path(__file__).with_name("issue11-ledger.py"))
ledger = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ledger)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.claims = ledger.read_json(ledger.RUST_CLAIMS)
        self.evidence = ledger.read_json(ledger.RUST_EVIDENCE)
        self.reference = ledger.read_json(ledger.REFERENCE_EVIDENCE)
        self.records = ledger.read_json(ledger.LEDGER)["records"]
        self.entries = {key: {"scope": "in_scope", "disposition": "unresolved"}
                        for key in self.claims["claims"]}

    def apply(self):
        with patch.object(ledger, "read_json", side_effect=[self.claims, self.evidence]):
            ledger.apply_rust_evidence(self.entries, self.records, self.reference)

    def test_only_explicit_claims_are_verified(self):
        self.entries["unrelated"] = {"scope": "in_scope", "disposition": "unresolved"}
        self.apply()
        self.assertEqual(self.entries["unrelated"]["disposition"], "unresolved")
        self.assertEqual(sum(row["disposition"] == "verified" for row in self.entries.values()), len(self.claims["claims"]))

    def test_missing_or_stale_input_hash_is_rejected(self):
        original = copy.deepcopy(self.evidence)
        for change in ("missing", "stale"):
            with self.subTest(change=change):
                self.evidence = copy.deepcopy(original)
                if change == "missing":
                    self.evidence["files"].pop("Cargo.lock")
                else:
                    self.evidence["files"]["Cargo.lock"] = "0" * 64
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_failed_ignored_or_absent_rust_test_is_rejected(self):
        original = copy.deepcopy(self.evidence)
        for change in ("failed", "ignored", "absent"):
            with self.subTest(change=change):
                self.evidence = copy.deepcopy(original)
                if change == "failed":
                    self.evidence["exit_code"] = 1
                elif change == "ignored":
                    self.evidence["stdout"] = self.evidence["stdout"].replace(" ... ok", " ... ignored")
                else:
                    self.evidence["passed_tests"] = []
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_wrong_pin_or_compiler_is_rejected(self):
        original = copy.deepcopy(self.evidence)
        for field in ("baseline_revision", "compiler"):
            with self.subTest(field=field):
                self.evidence = copy.deepcopy(original)
                self.evidence[field] = "wrong"
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_excluded_entry_cannot_be_verified(self):
        self.entries[next(iter(self.entries))]["scope"] = "out_of_scope"
        with self.assertRaises(SystemExit):
            self.apply()

    def test_source_identity_and_reference_test_are_required(self):
        key = next(iter(self.claims["claims"]))
        claim = self.claims["claims"][key]
        original = copy.deepcopy(claim)
        for field in ("source_name", "reference_test"):
            with self.subTest(field=field):
                self.claims["claims"][key] = copy.deepcopy(original)
                self.claims["claims"][key][field] = "nonexistent"
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_snapshot_write_is_rejected(self):
        with self.assertRaises(SystemExit):
            ledger.reject_snapshot_path(ledger.SNAPSHOT / "inventory.json")


if __name__ == "__main__":
    unittest.main()
