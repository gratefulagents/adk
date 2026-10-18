import copy
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("issue11_ledger", Path(__file__).with_name("issue11-ledger.py"))
ledger = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ledger)


class ReferenceSelectionTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "sdk_reference", Path(__file__).parent / "trace-reference/test-sdk.py")
        self.reference = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.reference)

    def test_claimed_tests_keep_exact_package_and_prior_regressions(self):
        selected = {"internal/agent": {"TestHistorical"}}
        claims = {"claims": {str(i): {"reference_test": name} for i, name in enumerate([
            "github.com/gratefulagents/sdk/internal/agent/TestRunner",
            "github.com/gratefulagents/sdk/examples/features/context_compaction/TestContextCompactionExample",
            "github.com/gratefulagents/sdk/examples/features/unrelated/TestNotSelected",
            "github.com/gratefulagents/sdk/internal/agent/agents/TestBuiltInKindsAreStable",
            "github.com/gratefulagents/sdk/internal/openai/TestMetadata",
            "github.com/gratefulagents/sdk/internal/openai/TestMetadata",
            "github.com/gratefulagents/sdk/internal/modelactivity/TestReadActivity",
            "github.com/gratefulagents/sdk/internal/anthropic/TestAssembler",
            "github.com/gratefulagents/sdk/pkg/agentsdk/TestPublic",
            "github.com/gratefulagents/sdk/internal/unrelated/TestNotSelected",
        ])}}
        self.reference.add_claimed_tests(claims, selected)
        self.assertEqual(selected, {"examples/features/context_compaction": {"TestContextCompactionExample"}, "internal/agent": {"TestHistorical", "TestRunner"},
                                    "internal/agent/agents": {"TestBuiltInKindsAreStable"},
                                    "internal/openai": {"TestMetadata"},
                                    "internal/modelactivity": {"TestReadActivity"},
                                    "internal/anthropic": {"TestAssembler"}})

    def test_reference_failures_keep_diagnostics_and_nonzero_status(self):
        output = json.dumps({"Action": "fail", "Package": "reference", "Test": "TestFailure"}) + "\n"
        result = self.reference.subprocess.CompletedProcess([], 1, output, "underlying failure\n")
        stderr = io.StringIO()
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "reference.json"
            with (
                patch.object(self.reference.sys, "argv", ["test-sdk", "--output", str(destination)]),
                patch.object(self.reference.subprocess, "check_output",
                             side_effect=[self.reference.PIN + "\n", b"", "go version test\n"]),
                patch.object(self.reference.subprocess, "run", return_value=result),
                patch.object(self.reference.sys, "stderr", stderr),
                patch.object(self.reference.sys, "stdout", io.StringIO()),
                self.assertRaises(SystemExit) as raised,
            ):
                self.reference.main()
            self.assertEqual(raised.exception.code, 1)
            report = json.loads(destination.read_text())
            self.assertEqual(report["exit_code"], 1)
            self.assertEqual(report["tests"]["reference/TestFailure"], "fail")
        self.assertIn(output, stderr.getvalue())
        self.assertIn("underlying failure", stderr.getvalue())

    def test_reference_patterns_and_subtests_cannot_expand_execution(self):
        for package in ("internal/openai", "examples/features/context_compaction"):
            for name in ("TestAny|TestOther", "TestAny/subtest", "TestAny$", "../TestOther", ""):
                with self.subTest(package=package, name=name), self.assertRaises(SystemExit):
                    self.reference.add_claimed_tests({"claims": {
                        "id": {"reference_test": "github.com/gratefulagents/sdk/" + package + "/" + name}
                    }}, {})


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

    def test_builder_fixture_execution_markers_are_required(self):
        original = self.evidence["reference_fixture_check"]
        for marker in ("Pinned tool schemas reproduced exactly",
                       "272 SDK lifecycle results and resulting filesystem trees verified",
                       "1176 SDK ApplyPatch results and resulting filesystem trees verified",
                       "Pinned SDK MCP prompt observations reproduced exactly",
                       "Pinned SDK run instructions observations reproduced exactly",
                       "Pinned SDK workspace context observations reproduced exactly",
                       "Pinned SDK project-state runtime observations reproduced exactly",
                       "Pinned metadata compaction observations reproduced exactly",
                       "Pinned provider continuation observations reproduced exactly",
                       "Pinned LLM summary observations reproduced exactly",
                       "Pinned autoloop observations reproduced exactly",
                       "Pinned child result-cost observations reproduced exactly",
                       "Pinned progress observations reproduced exactly",
                       "Pinned registry selection observations reproduced exactly",
                       "Pinned payload observations reproduced exactly",
                       "Go subagents: 6 scenarios verified unchanged",
                       "Pinned content event observations reproduced exactly",
                       "Pinned line writer observations reproduced exactly",
                       "Pinned content stream observations reproduced exactly",
                       "Pinned content logger observations and printable ranges reproduced exactly"):
            with self.subTest(marker=marker):
                self.evidence["reference_fixture_check"] = original.replace(marker, "")
                with self.assertRaisesRegex(SystemExit, "builder evidence requires"):
                    self.apply()

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

    def test_workspace_feature_manifest_is_hash_bound(self):
        self.assertIn("Cargo.toml", self.evidence["files"])
        self.evidence["files"]["Cargo.toml"] = "0" * 64
        with self.assertRaisesRegex(SystemExit, "stale Rust evidence for Cargo.toml"):
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

    def test_unexecuted_or_wrong_pin_fixture_evidence_is_rejected(self):
        for value in ("", "trace-store reference verified at wrong"):
            with self.subTest(value=value):
                self.evidence["reference_fixture_check"] = value
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_record_oracle_and_live_go_execution_are_required(self):
        original = copy.deepcopy(self.evidence)
        for change in ("missing_oracle", "wrong_pin", "disabled_go"):
            with self.subTest(change=change):
                self.evidence = copy.deepcopy(original)
                if change == "disabled_go":
                    self.evidence["environment"]["ADK_TEST_GO"] = "0"
                else:
                    self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                        f"typed-record reference fixtures verified at {ledger.BASELINE_REVISION}",
                        "" if change == "missing_oracle" else "typed-record reference fixtures verified at wrong")
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_fileconfig_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "fileconfig reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"fileconfig reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_handoff_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "handoff reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"handoff reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_subagent_selection_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "subagent selection reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"subagent selection reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_final_summary_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "final summary reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"final summary reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_immediate_input_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "immediate input reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"immediate input reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_immediate_signal_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "immediate signal reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"immediate signal reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_builtin_guardrails_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "builtin guardrails reference verified at wrong"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    f"builtin guardrails reference verified at {ledger.BASELINE_REVISION}", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_specialist_catalog_oracle_is_required(self):
        original = copy.deepcopy(self.evidence)
        for replacement in ("", "Specialist catalog not executed"):
            with self.subTest(replacement=replacement):
                self.evidence = copy.deepcopy(original)
                self.evidence["reference_fixture_check"] = self.evidence["reference_fixture_check"].replace(
                    "Pinned specialist catalog observations reproduced exactly", replacement)
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_role_catalog_oracle_is_required(self):
        marker = "Pinned role/catalog observations reproduced exactly"
        original = self.evidence["reference_fixture_check"].replace(marker, "") + marker
        for replacement in ("", "Role/catalog oracle not executed"):
            with self.subTest(replacement=replacement):
                self.evidence["reference_fixture_check"] = original.replace(marker, replacement)
                with self.assertRaisesRegex(SystemExit, "builder evidence requires"):
                    self.apply()

    def test_mode_codec_oracle_is_required(self):
        marker = "Pinned mode codec observations reproduced exactly"
        original = self.evidence["reference_fixture_check"].replace(marker, "") + marker
        for replacement in ("", "Mode codec oracle not executed"):
            with self.subTest(replacement=replacement):
                self.evidence["reference_fixture_check"] = original.replace(marker, replacement)
                with self.assertRaisesRegex(SystemExit, "builder evidence requires"):
                    self.apply()

    def test_mode_routing_oracle_is_required(self):
        marker = "Pinned mode routing observations reproduced exactly"
        original = self.evidence["reference_fixture_check"].replace(marker, "") + marker
        for replacement in ("", "Mode routing oracle not executed"):
            with self.subTest(replacement=replacement):
                self.evidence["reference_fixture_check"] = original.replace(marker, replacement)
                with self.assertRaisesRegex(SystemExit, "builder evidence requires"):
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

    def test_unbound_implementation_source_is_rejected(self):
        claim = next(iter(self.claims["claims"].values()))
        claim["implementation_symbols"] = ["crates/missing.rs::missing"]
        with self.assertRaises(SystemExit):
            self.apply()

    def test_snapshot_write_is_rejected(self):
        with self.assertRaises(SystemExit):
            ledger.reject_snapshot_path(ledger.SNAPSHOT / "inventory.json")


class ArtifactEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.claims = ledger.read_json(ledger.ARTIFACT_CLAIMS)
        self.inventory = ledger.read_json(ledger.LEDGER)
        self.sources = ledger.read_json(ledger.SNAPSHOT / "sources.json")
        self.manifest = ledger.read_json(ledger.MANIFEST)
        self.report = ledger.read_json(ledger.ROOT / ledger.ARTIFACT_REVIEW)
        self.rust_claims = ledger.read_json(ledger.RUST_CLAIMS)
        self.entries = {row["acceptance_id"]: {"scope": "in_scope", "disposition": "unresolved"}
                        for rows in self.inventory["records"].values() for row in rows}
        self.evidence = {
            "schema_version": 1, "baseline_revision": ledger.BASELINE_REVISION,
            "artifact_revision": ledger.ARTIFACT_REVISION, "evidence_kind": "artifact_review",
            "command": ledger.ARTIFACT_COMMAND, "profile": ledger.ARTIFACT_PROFILE,
            "platform": "test-platform", "python": "3.11.0", "exit_code": 0,
            "results": {key: "pass" for key in ledger.ARTIFACT_ALLOWLIST},
            "files": ledger.artifact_input_hashes(),
        }
        documents = {
            ledger.ARTIFACT_CLAIMS: self.claims, ledger.LEDGER: self.inventory,
            ledger.SNAPSHOT / "sources.json": self.sources, ledger.MANIFEST: self.manifest,
            ledger.ROOT / ledger.ARTIFACT_REVIEW: self.report,
            ledger.RUST_CLAIMS: self.rust_claims, ledger.ARTIFACT_EVIDENCE: self.evidence,
        }
        self.reader = patch.object(ledger, "read_json", side_effect=documents.__getitem__)
        self.reader.start()
        self.addCleanup(self.reader.stop)

    def validate(self):
        return ledger.validate_artifact_claims(self.claims, self.inventory["records"])

    def apply(self):
        ledger.apply_artifact_evidence(self.entries, self.inventory["records"])

    def test_only_exact_license_ids_close_without_native_proof_or_inheritance(self):
        self.apply()
        verified = {key for key, row in self.entries.items() if row["disposition"] == "verified"}
        self.assertEqual(verified, set(ledger.ARTIFACT_ALLOWLIST))
        for key in verified:
            entry = self.entries[key]
            self.assertEqual(entry["evidence_kind"], "artifact_review")
            self.assertEqual(entry["artifact_paths"], ledger.ARTIFACT_PATHS)
            self.assertEqual(entry["review_approval_reference"], ledger.ARTIFACT_REVIEW_REFERENCE)
            self.assertEqual(entry["implementation_symbols"], [])
            self.assertEqual(entry["rust_test_identifiers"], [])
            self.assertEqual(entry["target_features"], [])
            self.assertIsNone(entry["approved_divergence_reference"])
        group = next(row for row in self.inventory["records"]["verification_groups"]
                     if row["rust_module"] == "sdk::distribution")
        self.assertEqual(self.entries[group["acceptance_id"]]["disposition"], "unresolved")

    def test_wrong_id_and_source_test_masquerade_are_rejected(self):
        original = copy.deepcopy(self.claims["claims"])
        for key in ("missing", self.inventory["records"]["tests"][0]["acceptance_id"]):
            with self.subTest(key=key):
                self.claims["claims"] = copy.deepcopy(original)
                self.claims["claims"][key] = self.claims["claims"].pop(next(iter(original)))
                with self.assertRaisesRegex(SystemExit, "allowlist"):
                    self.validate()

    def test_claim_fields_and_schema_are_strict(self):
        key = next(iter(self.claims["claims"]))
        original = copy.deepcopy(self.claims["claims"][key])
        changes = {field: "wrong" for field in original}
        changes.update({"rust_test_identifiers": [], "implementation_symbols": [], "reference_test": "fake"})
        for field, value in changes.items():
            if field == "rationale":
                value = " "
            with self.subTest(field=field):
                self.claims["claims"][key] = {**original, field: value}
                with self.assertRaises(SystemExit):
                    self.validate()
        self.claims["claims"][key] = original
        del self.claims["claims"][key]["category"]
        with self.assertRaisesRegex(SystemExit, "schema"):
            self.validate()

    def test_envelope_pin_and_version(self):
        for field, value in (("baseline_revision", "wrong"), ("schema_version", True),
                             ("schema_version", 2), ("extra", "field")):
            with self.subTest(field=field, value=value):
                changed = {**self.claims, field: value}
                with self.assertRaises(SystemExit):
                    ledger.validate_artifact_claims(changed)

    def test_duplicate_json_fields_are_rejected(self):
        self.reader.stop()
        for path in (ledger.ARTIFACT_CLAIMS, ledger.ARTIFACT_EVIDENCE, ledger.ROOT / ledger.ARTIFACT_REVIEW):
            with self.subTest(path=path), patch.object(Path, "open", return_value=io.StringIO(
                    '{"claims":{"duplicate":{},"duplicate":{}}}')), self.assertRaisesRegex(SystemExit, "duplicate"):
                ledger.read_json(path)

    def test_actual_inventory_and_archived_source_are_validated(self):
        row = next(row for row in self.inventory["records"]["files"] if row["source"] == "LICENSE")
        original = copy.deepcopy(row)
        for field, value in (("sha256", "0" * 64), ("bytes", 1), ("source", "test.go"),
                             ("rust_module", "sdk_cli"), ("id", "files:NOTICE.md")):
            with self.subTest(field=field):
                row.update(original)
                row[field] = value
                with self.assertRaises(SystemExit):
                    self.validate()
        row.update(original)
        source = next(row for row in self.sources["files"] if row["path"] == "LICENSE")
        source["content"] += "changed"
        with self.assertRaisesRegex(SystemExit, "archived LICENSE"):
            self.validate()

    def test_manifest_and_actual_immutable_bytes_are_validated(self):
        self.manifest["artifact_sha256"]["sources.json"] = "0" * 64
        with self.assertRaisesRegex(SystemExit, "manifest hash"):
            self.validate()

    def test_current_and_committed_bytes_and_preserved_inputs_cannot_rebaseline(self):
        real_read = Path.read_bytes
        real_git = ledger.artifact_git_bytes
        for path in (*ledger.ARTIFACT_PATHS, *ledger.PRESERVED_INPUTS,
                     "docs/migration/ledger/acceptance-contracts.json", ledger.ARTIFACT_REVIEW):
            with self.subTest(path=path, location="working tree"):
                def changed_read(target):
                    data = real_read(target)
                    return data + b"changed" if target == ledger.ROOT / path else data
                with patch.object(Path, "read_bytes", changed_read), self.assertRaises(SystemExit):
                    self.validate()
            with self.subTest(path=path, location="git"):
                def changed_git(target, revision=ledger.ARTIFACT_REVISION):
                    data = real_git(target, revision)
                    return data + b"changed" if target == path else data
                with patch.object(ledger, "artifact_git_bytes", side_effect=changed_git), self.assertRaises(SystemExit):
                    self.validate()

    def test_approval_is_explicit_complete_and_scoped(self):
        approvals = self.report["approvals"]
        original = copy.deepcopy(approvals["sdk-license-retention-v1"])
        for field in original:
            with self.subTest(field=field):
                approvals["sdk-license-retention-v1"] = {**original, field: ""}
                with self.assertRaises(SystemExit):
                    self.validate()
        approvals["sdk-license-retention-v1"] = {**original, "unexpected": True}
        with self.assertRaisesRegex(SystemExit, "schema"):
            self.validate()
        approvals.clear()
        with self.assertRaisesRegex(SystemExit, "schema"):
            self.validate()

    def test_review_reference_and_paths_are_safe_and_tracked(self):
        claim = next(iter(self.claims["claims"].values()))
        for reference in ("https://example.test/approval", "../approval.json#/ok",
                          ledger.ARTIFACT_REVIEW, ledger.ARTIFACT_REVIEW + "#/approvals/missing"):
            with self.subTest(reference=reference):
                claim["review_approval_reference"] = reference
                with self.assertRaises(SystemExit):
                    self.validate()
        for path in ("../LICENSE", "/LICENSE", "https://example.test/approval", "docs/../LICENSE"):
            with self.subTest(path=path), self.assertRaises(SystemExit):
                ledger.artifact_path(path)
        with patch.object(Path, "is_symlink", return_value=True), self.assertRaises(SystemExit):
            ledger.artifact_path(ledger.ARTIFACT_REVIEW, tracked=True)
        with patch.object(ledger.subprocess, "check_output", return_value=""), self.assertRaises(SystemExit):
            ledger.artifact_path(ledger.ARTIFACT_REVIEW, tracked=True)

    def test_evidence_requires_fixed_command_pin_profile_and_each_success(self):
        original = copy.deepcopy(self.evidence)
        for field, value in (("command", ["true"]), ("profile", "other"), ("exit_code", 1),
                             ("exit_code", False), ("schema_version", True),
                             ("baseline_revision", "wrong"), ("artifact_revision", "wrong"),
                             ("evidence_kind", "rust_test"), ("results", {}),
                             ("results", {key: "ignored" for key in ledger.ARTIFACT_ALLOWLIST}),
                             ("python", ""), ("platform", ""), ("extra", True)):
            with self.subTest(field=field, value=value):
                self.evidence.clear()
                self.evidence.update({**original, field: value})
                with self.assertRaises(SystemExit):
                    self.apply()

    def test_exact_input_set_and_freshness(self):
        original = copy.deepcopy(self.evidence["files"])
        for path in ledger.ARTIFACT_INPUTS:
            for change in ("missing", "stale"):
                with self.subTest(path=path, change=change):
                    self.evidence["files"] = original.copy()
                    if change == "missing":
                        del self.evidence["files"][path]
                    else:
                        self.evidence["files"][path] = "0" * 64
                    with self.assertRaisesRegex(SystemExit, "stale artifact evidence"):
                        self.apply()
        self.evidence["files"] = {**original, "extra": "0" * 64}
        with self.assertRaisesRegex(SystemExit, "stale artifact evidence"):
            self.apply()

    def test_overlap_excluded_and_already_verified_are_rejected(self):
        key = next(iter(ledger.ARTIFACT_ALLOWLIST))
        self.rust_claims["claims"][key] = {}
        with self.assertRaisesRegex(SystemExit, "overlap"):
            self.apply()
        with patch.object(Path, "write_text") as write, self.assertRaisesRegex(SystemExit, "overlap"):
            ledger.verify_artifacts()
        write.assert_not_called()
        del self.rust_claims["claims"][key]
        self.entries[key]["scope"] = "out_of_scope"
        with self.assertRaisesRegex(SystemExit, "cannot verify"):
            self.apply()
        self.entries[key] = {"scope": "in_scope", "disposition": "verified"}
        with self.assertRaisesRegex(SystemExit, "cannot verify"):
            self.apply()

    def test_generator_only_writes_after_success_and_never_snapshot(self):
        with patch.object(Path, "write_text") as write:
            ledger.verify_artifacts()
            generated = json.loads(write.call_args.args[0])
            self.assertEqual(generated["results"], self.evidence["results"])
            self.assertEqual(generated["files"], ledger.artifact_input_hashes())
        self.claims["baseline_revision"] = "wrong"
        with patch.object(Path, "write_text") as write, self.assertRaises(SystemExit):
            ledger.verify_artifacts()
        write.assert_not_called()
        with patch.object(ledger, "ARTIFACT_EVIDENCE", ledger.SNAPSHOT / "bad.json"), \
                patch.object(Path, "write_text") as write, self.assertRaisesRegex(SystemExit, "baseline snapshot"):
            ledger.verify_artifacts()
        write.assert_not_called()

    def test_inputs_changed_during_verification_prevent_write(self):
        with patch.object(ledger, "artifact_input_hashes", side_effect=[{"first": "hash"}, {"second": "hash"}]), \
                patch.object(Path, "write_text") as write, self.assertRaisesRegex(SystemExit, "changed during"):
            ledger.verify_artifacts()
        write.assert_not_called()

    def test_failed_rust_proof_does_not_fall_back_to_artifacts(self):
        self.reader.stop()
        with patch.object(ledger, "apply_rust_evidence", side_effect=SystemExit("failed Rust")), \
                patch.object(ledger, "apply_artifact_evidence") as artifact, self.assertRaisesRegex(SystemExit, "failed Rust"):
            ledger.overlay()
        artifact.assert_not_called()


if __name__ == "__main__":
    unittest.main()
