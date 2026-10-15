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
            "a": {"disposition": "verified", "scope": "in_scope"},
            "b": {"disposition": "unresolved", "scope": "in_scope"},
            "c": {"disposition": "excluded", "scope": "out_of_scope"},
        }, "counts": {"verified": 1, "unresolved": 1, "excluded": 1}}
        self.claims = {"claims": {"a": {}}}

    def build(self):
        return audit.build(self.inventory, self.overlay, self.claims)

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
