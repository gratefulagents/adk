#!/usr/bin/env python3
"""Generate or check the issue #11 ledger overlay without touching snapshots."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
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
ARTIFACT_CLAIMS = ROOT / "docs/migration/ledger/issue-11-artifact-claims.json"
ARTIFACT_EVIDENCE = ROOT / "docs/verification/issue-11-artifact-evidence.json"
ARTIFACT_REVIEW = "docs/verification/issue-11-license-review.json"
ARTIFACT_REVIEW_REFERENCE = ARTIFACT_REVIEW + "#/approvals/sdk-license-retention-v1"
ARTIFACT_REVISION = "b1a1a981ead06d9c69eae4aaa713bb373a6fe2c3"
ARTIFACT_SHA256 = "e3df40627909b6fb9d91898c54b8c56dd677b0b2919a2e584ff3dbe7ebeced27"
ARTIFACT_ALLOWLIST = {"SDK-B9810B40B3C11D74": "artifacts", "SDK-0B2DC29E8C9CA68D": "files"}
ARTIFACT_PATHS = {"LICENSE": ARTIFACT_SHA256, "docs/migration/licenses/sdk-LICENSE": ARTIFACT_SHA256}
PRESERVED_INPUTS = {
    "NOTICE.md": "02e24299af6e90c0eeeb4d461a32dea8b35dbb2eb02d5c88170b102b4d830d67",
    "docs/migration/source-lock.json": "f5ccbe03a730d24d8fe85570b1de8d0f5d7127390ec01710e2c20b88b76e5087",
}
ARTIFACT_COMMAND = ["python3", "scripts/issue11-ledger.py", "verify-artifacts"]
ARTIFACT_PROFILE = "sdk-license-only-v1"
ARTIFACT_INPUTS = [
    "docs/migration/ledger/issue-11-artifact-claims.json", ARTIFACT_REVIEW,
    "scripts/issue11-ledger.py", "scripts/test_issue11_ledger.py",
    "scripts/issue11-audit.py", "scripts/test_issue11_audit.py",
    "docs/migration/ledger/acceptance-contracts.json",
    "docs/migration/ledger/sdk-v0.0.115/inventory.json",
    "docs/migration/ledger/sdk-v0.0.115/sources.json",
    "docs/migration/ledger/sdk-v0.0.115/manifest.json",
    *ARTIFACT_PATHS, *PRESERVED_INPUTS,
]
SNAPSHOT = ROOT / "docs/migration/ledger/sdk-v0.0.115"

BASELINE_REVISION = "1dc92b73900fac74dc357a938e4b5eee6392b418"
AUDIT_RUST_REVISION = "489b1886aa760b6a2d4680d42bd3dce0a1397407"
AUDIT_SDK_CHECKOUT = "63afe2ed8cc5f13ca7469054f2c1cb812fcac801"
TOTAL = 9142
EXCLUDED = 251
RETAINED = 8891
RUST_INPUTS = ["crates/adk/tests/execution.rs", "scripts/sandbox-env-reference/environment_test.go", "crates/adk-codec/src/sandbox_env.rs", "crates/adk-codec/tests/sandbox_env.rs", "scripts/sandbox-env-reference/run.py", "scripts/sandbox-env-reference/reference_test.go", "scripts/sandbox-env-reference/README.md", "fixtures/sandbox-env/observations.json", "crates/adk-sandbox/src/lib.rs", "crates/adk-sandbox/src/backend.rs", "crates/adk-sandbox/src/policy.rs", "crates/adk-sandbox/tests/lifecycle.rs", 'crates/adk-codec/src/text.rs', 'crates/adk-codec/tests/mcp_display.rs', 'scripts/mcp-display-reference/run.py', 'scripts/mcp-display-reference/reference_test.go', 'scripts/mcp-display-reference/README.md', 'fixtures/mcp-display/observations.json', 'crates/adk-codec/src/retry.rs', 'crates/adk-codec/tests/retry.rs', 'scripts/retry-reference/run.py', 'scripts/retry-reference/reference_test.go', 'scripts/retry-reference/README.md', 'fixtures/retry/observations.json', 'crates/adk-core/src/authorization.rs', 'crates/adk-core/tests/authorization.rs', 'crates/adk-runtime/tests/runner/authorization.rs', 'crates/adk-runtime/tests/durable/authorization.rs', 'crates/adk/tests/builder_subagents/authorization.rs', 'scripts/action-authorization-reference/run.py', 'scripts/action-authorization-reference/reference_test.go', 'scripts/action-authorization-reference/README.md', 'fixtures/action-authorization/observations.json', "crates/adk-codec/src/policy.rs", "crates/adk-codec/tests/policy.rs", "scripts/policy-reference/run.py", "scripts/policy-reference/reference_test.go", "scripts/policy-reference/README.md", "fixtures/policy/observations.json", "docs/execution-security.md","docs/feature-examples.md","scripts/mode-routing-reference/run.py", "scripts/mode-routing-reference/reference_test.go",
               "scripts/mode-routing-reference/README.md", "fixtures/mode-routing/observations.json",
               "crates/adk-codec/tests/mode_routing.rs", "crates/adk-codec/src/mode.rs", "scripts/mode-codec-reference/run.py", "scripts/mode-codec-reference/reference_test.go",
               "scripts/mode-codec-reference/README.md", "fixtures/mode-codec/observations.json",
               "crates/adk-codec/tests/modes.rs", "scripts/role-catalog-reference/run.py", "scripts/role-catalog-reference/reference_test.go",
               "scripts/role-catalog-reference/README.md", "fixtures/role-catalog/observations.json",
               "crates/adk-codec/tests/roles.rs", "scripts/handoff-reference/regex_classes.go", "scripts/handoff-reference/prompt_cache.go", "fixtures/handoff/sdk-prompt-cache.json", "crates/adk/src/specialists/catalog.rs", "crates/adk/tests/specialists/catalog.rs", "scripts/specialist-tools-reference/catalog_test.go", "fixtures/specialist-catalog/observations.json", "crates/adk/src/guardrails/rules.rs", "scripts/handoff-reference/public_rules.go", "crates/adk-security/src/secrets.rs", "crates/adk-security/src/signatures.rs", "crates/adk-security/tests/fixtures/secret_obfuscation.txt", "crates/adk/src/specialists/delegation.rs", "crates/adk/src/specialists/delegation.txt", "crates/adk/tests/specialists/delegation.rs", "scripts/specialist-tools-reference/delegation_test.go", "crates/adk-providers/tests/contracts/compaction_origin.rs", "crates/adk-providers/src/anthropic.rs", "crates/adk-providers/src/snapshot/assembler.rs", "crates/adk-providers/tests/assembler.rs", "scripts/anthropic-assembler-reference/run.py", "scripts/anthropic-assembler-reference/reference_test.go", "fixtures/anthropic-assembler/observations.json", "crates/adk/tests/host/progress_boundary.rs", "crates/adk/tests/host/approval_regressions.rs", "scripts/providers/generate.sh", "scripts/providers/continuation_probe.go.txt", "scripts/providers/anthropic_schema_probe.go.txt", "scripts/providers/anthropic_usage_probe.go.txt", "fixtures/providers/anthropic-usage.json", "crates/adk-providers/tests/http/usage.rs", "fixtures/providers/continuation.json", "fixtures/providers/anthropic-schemas.json", "crates/adk-providers/tests/http/schemas.rs", "crates/adk-providers/src/factory.rs", "crates/adk-providers/src/auth.rs", "crates/adk-providers/src/cost.rs", "crates/adk-providers/src/openai.rs", "crates/adk-providers/tests/openai_cost_reference.rs", "crates/adk-runtime/src/compaction.rs", "crates/adk-runtime/tests/host_history.rs", "crates/adk-runtime/tests/subagent_reference.rs", "crates/adk-runtime/tests/subagent_activity.rs", "crates/adk-runtime/tests/subagent_tool_contracts.rs", "crates/adk-runtime/tests/fixtures/subagents.go", "crates/adk-runtime/tests/fixtures/go-subagents.json", "scripts/replay/subagent_export.py", "crates/adk/tests/payload_surface.rs", "crates/adk-codec/src/go_decode.rs", "crates/adk-codec/src/payload_decode.rs", "crates/adk-codec/tests/payload_decode.rs", "scripts/payload-reference/run.py", "scripts/payload-reference/reference_test.go", "fixtures/payloads/observations.json", "crates/adk-runtime/src/workspace.rs", "fixtures/usage/generate.go", "fixtures/usage/observations.json", "crates/adk/src/specialists.rs", "crates/adk/tests/specialists.rs", "scripts/specialist-tools-reference/run.py", "scripts/specialist-tools-reference/reference_test.go", "fixtures/specialist-tools/observations.json", "scripts/trace-reference/test-sdk.py", "crates/adk/src/content_stream/runtime.rs", "crates/adk/tests/content_stream/runtime.rs", "crates/adk-runtime/tests/generation_content.rs", "crates/adk-runtime/tests/generation_content/completion.rs", "scripts/content-stream-reference/thinking_reference_test.go", "scripts/trace-reference/finalization.go", "fixtures/tracestore/finalization.json", "crates/adk/src/content_stream/llm.rs", "crates/adk/tests/content_stream/llm.rs", "crates/adk/src/content_stream/subagent.rs", "crates/adk/tests/content_stream/subagent.rs", "crates/adk/src/agent_logger.rs", "crates/adk/tests/agent_logger.rs", "scripts/content-logger-reference/agent_reference_test.go", "crates/adk/src/content_logger.rs", "crates/adk-codec/src/printable.rs", "crates/adk/tests/content_logger.rs", "scripts/content-logger-reference/run.py", "scripts/content-logger-reference/reference_test.go", "fixtures/content-logger/observations.json", "docs/migration/licenses/go-LICENSE.txt", "crates/adk/src/content_stream.rs", "crates/adk/tests/content_stream.rs", "scripts/content-stream-reference/run.py", "scripts/content-stream-reference/reference_test.go", "fixtures/content-stream/observations.json", "crates/adk-codec/src/line_writer.rs", "crates/adk-codec/tests/line_writer.rs", "scripts/line-writer-reference/run.py", "scripts/line-writer-reference/reference_test.go", "fixtures/line-writer/observations.json", "crates/adk-codec/src/timestamp.rs", "Cargo.toml", "crates/adk-codec/src/events.rs", "crates/adk-codec/tests/content_events.rs", "fixtures/content-event/observations.json", "scripts/content-event-reference/run.py", "scripts/content-event-reference/reference_test.go", "crates/adk-runtime/src/autoloop.rs", "crates/adk-runtime/tests/autoloop.rs", "fixtures/autoloop/observations.json", "scripts/autoloop-reference/run.py", "scripts/autoloop-reference/reference_test.go", "fixtures/image-analysis/observations.json", "scripts/image-analysis-reference/run.py", "scripts/image-analysis-reference/reference_test.go", "fixtures/handoff-tracing/observations.json", "scripts/handoff-tracing-reference/run.py", "scripts/handoff-tracing-reference/reference_test.go", "fixtures/handoff-compaction/observations.json", "scripts/handoff-compaction-reference/run.py", "scripts/handoff-compaction-reference/reference_test.go", "scripts/handoff-compaction-reference/builder_test.go", "crates/adk-runtime/tests/local_compaction.rs", "fixtures/handoff-input-type/observations.json", "scripts/handoff-input-type-reference/run.py", "scripts/handoff-input-type-reference/reference_test.go", "fixtures/handoff-filter/observations.json", "scripts/handoff-filter-reference/run.py", "scripts/handoff-filter-reference/reference_test.go", "fixtures/handoff-enabled/observations.json", "scripts/handoff-enabled-reference/run.py", "scripts/handoff-enabled-reference/reference_test.go", "fixtures/handoff-callback/observations.json", "scripts/handoff-callback-reference/run.py", "scripts/handoff-callback-reference/reference_test.go", "fixtures/handoff-constructor/observations.json", "scripts/handoff-constructor-reference/run.py", "scripts/handoff-constructor-reference/reference_test.go", "fixtures/tool-stopping/observations.json", "scripts/tool-stopping-reference/run.py", "scripts/tool-stopping-reference/reference_test.go", "fixtures/dynamic-instructions/observations.json", "scripts/dynamic-instructions-reference/run.py", "scripts/dynamic-instructions-reference/reference_test.go", "crates/adk-runtime/src/verifier.rs", "fixtures/critic/observations.json", "scripts/critic-reference/run.py", "scripts/critic-reference/reference_test.go", "fixtures/verifier/observations.json", "scripts/verifier-reference/run.py",
               "scripts/verifier-reference/reference_test.go",
               "crates/adk-codec/src/config.rs", "crates/adk-codec/tests/runner_codecs.rs",
               "fixtures/confirmation/observations.json", "scripts/confirmation-reference/run.py",
               "scripts/confirmation-reference/reference_test.go",
               "crates/adk-codec/src/userinput.rs", "crates/adk-codec/tests/userinput.rs",
               "scripts/userinput-reference/run.py", "scripts/userinput-reference/reference_test.go",
               "fixtures/userinput/observations.json",
               "crates/adk-runtime/src/mcp_prompt.rs", "crates/adk-runtime/Cargo.toml",
               "fixtures/mcp-prompt/observations.json", "scripts/mcp-prompt-reference/run.py",
               "scripts/mcp-prompt-reference/reference_test.go","fixtures/run-instructions/observations.json", "scripts/run-instructions-reference/run.py",
               "scripts/run-instructions-reference/reference_test.go","crates/adk/src/tracing_runtime.rs", "crates/adk/tests/tracing_runtime.rs",
               "crates/adk/tests/tool_behavior.rs", "crates/adk/examples/features.rs", "crates/adk-runtime/src/guardrails.rs", "crates/adk-runtime/tests/guardrails.rs",
               "crates/adk/src/tracing.rs", "crates/adk/tests/tracing.rs",
               "crates/adk/tests/tracing_sinks.rs", "crates/adk/src/lib.rs", "Cargo.lock", "crates/adk/Cargo.toml", "crates/adk/src/tracestore.rs",
               "crates/adk/tests/tracestore.rs", "crates/adk/src/telemetry.rs",
               "crates/adk/src/telemetry_spans.rs", "crates/adk/src/tracewriter.rs",
               "crates/adk/tests/telemetry.rs", "crates/adk/tests/telemetry_spans.rs",
               "fixtures/tracestore/sdk-otel.json", "scripts/trace-reference/otel.go",
               "scripts/trace-reference/go.mod", "scripts/trace-reference/go.sum",
               "crates/adk-codec/Cargo.toml", "crates/adk-codec/src/snapshots.rs",
               "crates/adk-codec/src/dto.rs", "crates/adk-codec/src/lib.rs",
               "crates/adk-codec/src/items.rs", "crates/adk-codec/tests/items.rs",
               "crates/adk-codec/src/value.rs",
               "fixtures/item-helpers/observations.json", "scripts/item-helpers-reference/run.py",
               "scripts/item-helpers-reference/reference_test.go",
               "crates/adk-codec/src/modelsdev.rs", "crates/adk-providers/src/modelsdev.rs", "crates/adk-providers/tests/modelsdev.rs", "scripts/modelsdev-reference/run.py", "scripts/modelsdev-reference/reference_test.go", "fixtures/modelsdev/observations.json",
               "crates/adk-codec/src/agents.rs", "crates/adk-codec/tests/agents.rs", "scripts/agent-kinds-reference/run.py", "scripts/agent-kinds-reference/reference_test.go", "fixtures/agent-kinds/observations.json",
               "crates/adk-codec/src/cost.rs", "crates/adk-codec/tests/baseline.rs", "crates/adk-platform/src/codec.rs",
               "crates/adk-codec/tests/request_snapshot.rs", "crates/adk/tests/tracewriter.rs",
               "crates/adk-codec/src/request_native.rs", "crates/adk-codec/tests/native_request_snapshot.rs",
               "scripts/trace-reference/request.go", "scripts/trace-reference/nonfinite.go",
               "scripts/trace-reference/provider.go", "crates/adk-providers/src/snapshot.rs",
               "crates/adk-providers/src/snapshot/completion.rs",
               "crates/adk-providers/src/anthropic_compaction_union.json",
               "crates/adk-providers/src/client.rs", "crates/adk-providers/src/lib.rs",
               "crates/adk-providers/src/routing.rs", "crates/adk-providers/src/copilot.rs",
               "crates/adk-providers/tests/http.rs", "crates/adk-providers/tests/http/activity.rs", "crates/adk-providers/tests/http/reasoning.rs",
               "crates/adk-runtime/tests/runner/activity.rs",
               "crates/adk-providers/Cargo.toml", "crates/adk-providers/tests/snapshots.rs",
               "crates/adk-codec/tests/native_snapshot.rs", "crates/adk-codec/tests/projected_snapshot.rs",
               "crates/adk-codec/src/approval.rs",
               "crates/adk-runtime/src/durable.rs", "crates/adk-runtime/tests/durable.rs",
               "crates/adk-runtime/tests/fixtures/verify.go",
               "crates/adk-runtime/tests/fixtures/checkpoint.go",
               "crates/adk-runtime/tests/fixtures/go-checkpoint.json",
               "crates/adk-runtime/tests/fixtures/go-stop-gate.json",
               "crates/adk-runtime/tests/runner.rs", "crates/adk-runtime/tests/subagent.rs", "crates/adk-runtime/tests/subagent_integration.rs",
               "crates/adk/src/observability.rs",
               "crates/adk/tests/observability.rs",
               "crates/adk-runtime/src/tracing.rs", "crates/adk-runtime/src/runner.rs",
               "crates/adk-runtime/src/hooks.rs",
               "crates/adk-runtime/src/cost.rs", "fixtures/costs/sdk-result-cost.json",
               "scripts/cost-reference/run.py", "scripts/cost-reference/reference_test.go",
               "crates/adk/src/progress/mod.rs", "crates/adk/src/progress/state.rs",
               "crates/adk/src/progress/types.rs", "crates/adk/src/progress/record.rs",
               "crates/adk/src/progress/helpers.rs", "crates/adk/tests/progress_helpers.rs",
               "crates/adk/src/progress/runtime.rs", "crates/adk/tests/progress_runtime.rs",
               "crates/adk/tests/progress_pricing.rs", "crates/adk-runtime/src/progress.rs",
               "crates/adk-runtime/tests/progress.rs",
               "crates/adk/tests/progress.rs", "crates/adk/tests/progress_reference.rs",
               "scripts/progress-reference/run.py", "scripts/progress-reference/reference_test.go",
               "scripts/progress-reference/helper_reference_test.go", "scripts/progress-reference/cases.json",
               "fixtures/progress/observations.json", "fixtures/progress/helpers.json",
               "scripts/progress-reference/runtime_reference_test.go", "fixtures/progress/runtime.json",
               "scripts/registry-reference/run.py", "scripts/registry-reference/reference_test.go",
               "fixtures/tools/registry-selection.json", "crates/adk-tools/tests/registry.rs",
               "crates/adk-tools/tests/registry_reference.rs",
               "crates/adk-tools/tests/web.rs", "crates/adk-tools/src/web.rs",
               "crates/adk-tools/src/network.rs", "crates/adk-tools/src/html.rs",
               "crates/adk/tests/usage.rs",
               "crates/adk-core/src/contracts.rs", "crates/adk-core/src/types.rs",
               "crates/adk-core/tests/contracts.rs", "crates/adk-providers/src/wire.rs",
               "crates/adk-providers/tests/contracts.rs", "crates/adk-providers/tests/contracts/chat_stream_reference.rs", "crates/adk-providers/tests/http.rs",
               "crates/adk-runtime/Cargo.toml", "crates/adk-runtime/src/lib.rs",
               "crates/adk-runtime/tests/error_contracts.rs", "crates/adk-runtime/src/settings.rs",
               "crates/adk-runtime/tests/settings.rs", "crates/adk/src/builder.rs",
               "crates/adk/src/builder/subagents.rs", "crates/adk/tests/builder_subagents/mod.rs",
               "crates/adk/tests/builder_subagents/lifecycle.rs", "crates/adk/tests/builder_subagents/durable.rs",
               "crates/adk/tests/builder.rs", "crates/adk/tests/catalog_handoffs.rs",
               "crates/adk/src/builder/workspace.rs", "crates/adk/tests/builder_workspace/mod.rs",
               "crates/adk/tests/builder_compaction_resolver/mod.rs",
               "crates/adk/tests/builder_compaction_resolver/metadata.rs",
               "crates/adk-providers/src/runtime/metadata.rs",
               "crates/adk-providers/src/metadata.rs", "crates/adk-providers/src/metadata/input.rs", "crates/adk-providers/src/runtime.rs",
               "crates/adk-providers/tests/metadata_compaction/mod.rs",
               "crates/adk-providers/tests/routes_cost_parity.rs",
               "crates/adk-runtime/src/compaction.rs", "crates/adk-runtime/tests/followup.rs",
               "crates/adk-runtime/src/compaction/llm.rs", "crates/adk-runtime/src/compaction/llm/tests.rs",
               "crates/adk-runtime/tests/llm_compaction.rs", "crates/adk-runtime/tests/local_compaction.rs",
               "fixtures/llm-summary/observations.json", "scripts/llm-summary-reference/run.py",
               "scripts/llm-summary-reference/reference_test.go",
               "fixtures/compaction.json",
               "fixtures/metadata-compaction/observations.json",
               "scripts/metadata-compaction-reference/run.py",
               "scripts/metadata-compaction-reference/reference_test.go",
               "scripts/metadata-compaction-reference/endpoints_test.go",
               "fixtures/workspace-context/observations.json", "scripts/workspace-context-reference/run.py",
               "scripts/workspace-context-reference/reference_test.go",
               "crates/adk/src/builder/project_state.rs", "crates/adk/tests/builder_project_state/mod.rs",
               "crates/adk-project-state/src/resolution.rs", "crates/adk-project-state/tests/resolution.rs",
               "fixtures/project-state/runtime-observations.json", "scripts/project-state-runtime-reference/run.py",
               "scripts/project-state-runtime-reference/reference_test.go",
               "crates/adk/src/builder/mcp.rs", "crates/adk/tests/builder_mcp/mod.rs",
               "crates/adk-mcp/src/client.rs", "crates/adk-mcp/src/config.rs",
               "crates/adk-mcp/src/connection.rs", "crates/adk-mcp/src/session.rs",
               "crates/adk-mcp/src/session_budget_tests.rs",
               "crates/adk-mcp/tests/client.rs", "crates/adk-mcp/tests/session.rs",
               "crates/adk-mcp/tests/end_to_end.rs", "crates/adk-mcp/tests/diagnostics.rs",
               "crates/adk-mcp/src/transport.rs", "crates/adk-mcp/src/tools.rs",
               "fixtures/mcp/builder-peer.py", "fixtures/mcp/runtime-inputs.json",
               "fixtures/mcp/runtime-observations.json", "scripts/mcp-runtime-reference/run.py",
               "scripts/mcp-runtime-reference/reference_test.go",
               "scripts/trace-reference/settings.go",
               "fixtures/tracestore/sdk-writer.json", "scripts/trace-reference/writer.go",
               "fixtures/tracestore/sdk-store.json", "scripts/trace-reference/main.go",
               "scripts/trace-reference/check.py",
               "crates/adk/src/telemetry_stdout.rs", "crates/adk/tests/telemetry_stdout.rs",
               "crates/adk/tests/telemetry_defaults.rs", "scripts/trace-reference/stdout.go",
               "fixtures/tracestore/sdk-stdout.json",
               "scripts/check-record-fixtures.py",
               "fixtures/project-state/records/main.go",
               "fixtures/project-state/records.json",
               "fixtures/project-state/record-proof-map.json",
               "fixtures/durable/generate.go",
               "fixtures/durable/records.json",
               "crates/adk-durable/src/codec.rs",
               "crates/adk-durable/src/filesystem.rs",
               "crates/adk-durable/src/lib.rs",
               "crates/adk-durable/src/pg.rs",
               "crates/adk-durable/src/store.rs",
               "crates/adk-durable/src/types.rs",
               "crates/adk-project-state/src/contracts.rs",
               "crates/adk-project-state/src/engine.rs",
               "crates/adk-project-state/src/lib.rs",
               "crates/adk-project-state/src/memory.rs",
               "crates/adk-project-state/src/recall.rs",
               "crates/adk-project-state/src/storage.rs",
               "crates/adk-project-state/src/tools.rs",
               "crates/adk-project-state/src/types.rs",
               "crates/adk-durable/tests/record_codecs.rs",
               "crates/adk-project-state/tests/record_codecs.rs",
               "crates/adk-core/src/policy.rs", "crates/adk-runtime/src/compat.rs",
               "crates/adk-runtime/tests/compat.rs", "crates/adk-runtime/tests/host_config.rs",
               "crates/adk-runtime/src/subagent.rs", "crates/adk-runtime/src/subagent_tools.rs",
               "crates/adk/src/host.rs", "crates/adk/src/host/rules.rs", "crates/adk/src/host/go_unicode_names.txt", "crates/adk/tests/host.rs",
               "fixtures/host-loop/sdk-chatloop.json", "scripts/host-reference/check.py",
               "fixtures/host-loop/sdk-cursor.json", "scripts/host-reference/cursor.go",
               "crates/adk/tests/host_cursor.rs",
               "scripts/host-reference/main.go", "scripts/host-reference/scenarios.go",
               "scripts/host-reference/main_test.go", "scripts/host-reference/go.mod",
               "scripts/host-reference/go.sum", "crates/adk/examples/host_session.rs",
               "crates/adk-runtime/tests/host_history.rs", "crates/adk-security/src/policy.rs",
               "crates/adk-security/tests/security.rs", "crates/adk/tests/host_rules.rs",
               "crates/adk-core/tests/result_helpers.rs", "crates/adk/src/host/conversation.rs",
               "crates/adk/tests/host_conversation.rs", "scripts/host-reference/conversation.go",
               "fixtures/host-loop/sdk-conversation.json",
               "crates/adk/src/host/fileconfig.rs", "crates/adk/tests/fileconfig.rs",
               "crates/adk/tests/fileconfig_oracle.rs", "fixtures/fileconfig/sdk-fileconfig.json",
               "scripts/fileconfig-reference/check.py", "scripts/fileconfig-reference/main.go",
               "scripts/fileconfig-reference/scenarios.go", "scripts/fileconfig-reference/main_test.go",
               "scripts/fileconfig-reference/go.mod", "scripts/fileconfig-reference/go.sum",
               "crates/adk-tools/src/bundle.rs", "crates/adk-tools/tests/role_views.rs",
               "crates/adk-tools/src/fs_input.rs", "crates/adk-tools/tests/fs_input.rs",
               "crates/adk-tools/src/edit_diff.rs",
               "crates/adk-tools/Cargo.toml",
               "crates/adk-tools/src/workspace.rs", "crates/adk-tools/src/search.rs",
               "crates/adk-tools/src/patch.rs", "crates/adk-tools/src/patch_fs.rs",
               "crates/adk-tools/src/patch_parse.rs", "crates/adk-tools/src/patch_hunks.rs",
               "crates/adk-tools/tests/patch.rs", "crates/adk-tools/examples/patch_replay.rs",
               "fixtures/tools/patch.json", "fixtures/tools/verify-patch.go",
               "crates/adk-tools/src/manifest.json", "crates/adk-tools/tests/schema.rs",
               "crates/adk-tools/tests/edit.rs", "crates/adk-tools/tests/write.rs",
               "crates/adk-tools/tests/lifecycle.rs", "fixtures/tools/skills.json",
               "fixtures/tools/skill-catalog.json", "fixtures/tools/schema-definitions.json",
               "fixtures/tools/schema-generate.go",
               "crates/adk-tools/src/edit.rs", "crates/adk-tools/src/write.rs",
               "crates/adk-tools/src/lifecycle.rs", "crates/adk-tools/src/lib.rs", "crates/adk-tools/src/memory.rs",
               "crates/adk-tools/examples/lifecycle_replay.rs",
               "fixtures/tools/lifecycle.json", "fixtures/tools/verify-lifecycle.go",
               "crates/adk-tools/tests/bundle.rs", "crates/adk-tools/src/shell.rs",
               "crates/adk-tools/tests/shell.rs", "crates/adk-tools/tests/shell_security.rs",
               "scripts/handoff-reference/main.go", "scripts/handoff-reference/scenarios.go",
               "scripts/handoff-reference/main_test.go", "scripts/handoff-reference/check.py",
               "scripts/handoff-reference/go.mod", "scripts/handoff-reference/go.sum",
               "fixtures/handoff/sdk-handoff-filter.json",
               "scripts/handoff-reference/tripwire_errors.go", "fixtures/handoff/sdk-tripwire-errors.json",
               "scripts/handoff-reference/catalog.go", "scripts/handoff-reference/catalog_scenarios.go",
               "scripts/handoff-reference/catalog_test.go", "fixtures/handoff/sdk-catalog-handoffs.json",
               "scripts/handoff-reference/subagents.go", "scripts/handoff-reference/subagents_test.go",
               "fixtures/handoff/sdk-subagent-selection.json", "fixtures/handoff/sdk-final-summary.json",
               "scripts/handoff-reference/final_summary.go", "scripts/handoff-reference/final_summary_test.go",
               "scripts/handoff-reference/immediate_input.go", "scripts/handoff-reference/immediate_input_test.go",
               "fixtures/handoff/sdk-immediate-input.json",
               "scripts/handoff-reference/immediate_signal.go", "scripts/handoff-reference/immediate_signal_test.go",
               "fixtures/handoff/sdk-immediate-signal.json",
               "scripts/handoff-reference/builtin_guardrails.go", "scripts/handoff-reference/builtin_guardrails_test.go",
               "fixtures/handoff/sdk-builtin-guardrails.json",
               "crates/adk/src/guardrails.rs", "crates/adk/tests/builtin_guardrails.rs",
               "crates/adk/tests/builtin_guardrails_oracle.rs",
               "crates/adk-security/src/lib.rs", "crates/adk-security/src/secrets.rs",
               "crates/adk-security/src/signatures.rs", "crates/adk-security/src/shell.rs",
               "crates/adk-security/tests/builtin_guardrails.rs",
               "crates/adk-tools/src/shell_policy.rs",
               "crates/adk-runtime/tests/immediate_input_oracle/mod.rs",
               "crates/adk-runtime/tests/immediate_input_oracle/signal.rs",
               str(RUST_CLAIMS.relative_to(ROOT))]


def read_json(path: Path) -> object:
    with path.open(encoding="utf-8") as source:
        if path in (ARTIFACT_CLAIMS, ARTIFACT_EVIDENCE, ROOT / ARTIFACT_REVIEW):
            return json.load(source, object_pairs_hook=artifact_json_object)
        return json.load(source)


def artifact_json_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise SystemExit(f"duplicate artifact JSON field: {key}")
        result[key] = value
    return result


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
        "host": {
            "facade_feature": "host",
            "api_paths": ["crates/adk/src/host.rs", "crates/adk/src/host/rules.rs", "crates/adk/src/host/conversation.rs"],
            "test_paths": ["crates/adk/tests/host.rs", "crates/adk/tests/host_rules.rs", "crates/adk-runtime/tests/host_history.rs", "crates/adk/tests/host_conversation.rs"],
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


def artifact_object(value, fields, label):
    if not isinstance(value, dict) or set(value) != set(fields):
        raise SystemExit(f"invalid artifact {label} schema")


def artifact_text(value, label):
    if not isinstance(value, str) or not value.strip():
        raise SystemExit(f"artifact {label} must be nonempty text")


def artifact_path(path, tracked=False):
    relative = Path(path)
    if relative.is_absolute() or relative.as_posix() != path or any(
            part in {"..", "."} for part in relative.parts):
        raise SystemExit(f"unsafe artifact path: {path}")
    target = ROOT / relative
    if not target.is_file() or ROOT.resolve() not in target.resolve().parents or any(
            item.is_symlink() for item in (target, *target.parents) if item != ROOT.parent):
        raise SystemExit(f"artifact path must be a regular repository file: {path}")
    if tracked:
        listing = subprocess.check_output(["git", "ls-files", "--stage", "--", path], cwd=ROOT, text=True)
        if not listing.startswith(("100644 ", "100755 ")) or len(listing.splitlines()) != 1:
            raise SystemExit(f"artifact review must be tracked and regular: {path}")
    return target


def artifact_git_bytes(path, revision=ARTIFACT_REVISION):
    mode = subprocess.check_output(["git", "ls-tree", revision, "--", path], cwd=ROOT, text=True)
    if not mode.startswith(("100644 blob ", "100755 blob ")):
        raise SystemExit(f"artifact revision must contain a regular file: {path}")
    return subprocess.check_output(["git", "show", f"{revision}:{path}"], cwd=ROOT)


def validate_artifact_claims(claims, records=None):
    artifact_object(claims, {"schema_version", "baseline_revision", "claims"}, "claims envelope")
    if type(claims["schema_version"]) is not int or claims["schema_version"] != 1 or claims["baseline_revision"] != BASELINE_REVISION:
        raise SystemExit("artifact claims require schema 1 and authoritative baseline")
    artifact_object(claims["claims"], ARTIFACT_ALLOWLIST, "allowlist")
    inventory = read_json(LEDGER)
    sources_path = SNAPSHOT / "sources.json"
    sources = read_json(sources_path)
    manifest = read_json(MANIFEST)
    for document in (inventory, sources, manifest):
        if document["commit"] != BASELINE_REVISION or document["sdk_version"] != "v0.0.115":
            raise SystemExit("artifact source baseline mismatch")
    for path in (LEDGER, sources_path, MANIFEST, ROOT / "docs/migration/ledger/acceptance-contracts.json"):
        name = path.relative_to(ROOT).as_posix()
        current = artifact_path(name).read_bytes()
        if current != artifact_git_bytes(name):
            raise SystemExit(f"artifact immutable input changed: {name}")
        if path in (LEDGER, sources_path) and sha256(path) != manifest["artifact_sha256"].get(path.name):
            raise SystemExit(f"artifact manifest hash mismatch: {name}")
    archived = [row for row in sources["files"] if row["path"] == "LICENSE"]
    if len(archived) != 1:
        raise SystemExit("artifact archived LICENSE must be unique")
    source = archived[0]["content"].encode("utf-8")
    if len(source) != 35149 or hashlib.sha256(source).hexdigest() != ARTIFACT_SHA256 or archived[0]["sha256"] != ARTIFACT_SHA256:
        raise SystemExit("artifact archived LICENSE bytes mismatch")
    if records is not None and records != inventory["records"]:
        raise SystemExit("artifact records must match actual inventory")
    indexed = {}
    for category, rows in inventory["records"].items():
        for row in rows:
            key = row["acceptance_id"]
            if key in indexed:
                raise SystemExit(f"duplicate artifact inventory ID: {key}")
            indexed[key] = (category, row)
    fields = {"evidence_kind", "category", "ledger_record_id", "source", "rust_module", "source_sha256",
              "artifact_revision", "artifact_paths", "review_approval_reference", "rationale"}
    for key, category in ARTIFACT_ALLOWLIST.items():
        claim = claims["claims"][key]
        artifact_object(claim, fields, "claim")
        expected = {"evidence_kind": "artifact_review", "category": category,
                    "ledger_record_id": f"{category}:LICENSE", "source": "LICENSE",
                    "rust_module": "sdk::distribution", "source_sha256": ARTIFACT_SHA256,
                    "artifact_revision": ARTIFACT_REVISION, "artifact_paths": ARTIFACT_PATHS,
                    "review_approval_reference": ARTIFACT_REVIEW_REFERENCE}
        if any(claim[field] != value for field, value in expected.items()):
            raise SystemExit(f"artifact claim identity mismatch: {key}")
        artifact_text(claim["rationale"], "rationale")
        actual_category, row = indexed.get(key, (None, {}))
        if actual_category != category or any(row.get(field) != claim[claim_field] for field, claim_field in (
                ("id", "ledger_record_id"), ("source", "source"), ("rust_module", "rust_module"))) or excluded(row)[0]:
            raise SystemExit(f"artifact inventory identity mismatch: {key}")
        if category == "files" and (row.get("sha256") != ARTIFACT_SHA256 or row.get("bytes") != 35149):
            raise SystemExit("artifact inventory LICENSE hash/length mismatch")
    for path in ARTIFACT_PATHS:
        if artifact_path(path).read_bytes() != source or artifact_git_bytes(path) != source:
            raise SystemExit(f"artifact destination bytes mismatch: {path}")
    for path, expected in PRESERVED_INPUTS.items():
        if sha256(artifact_path(path)) != expected or hashlib.sha256(artifact_git_bytes(path)).hexdigest() != expected:
            raise SystemExit(f"artifact preserved input changed: {path}")
    review_path = artifact_path(ARTIFACT_REVIEW, tracked=True)
    if review_path.read_bytes() != artifact_git_bytes(ARTIFACT_REVIEW, "HEAD"):
        raise SystemExit("artifact review must be committed before verification")
    report = read_json(review_path)
    artifact_object(report, {"schema_version", "approvals"}, "review envelope")
    if type(report["schema_version"]) is not int or report["schema_version"] != 1:
        raise SystemExit("artifact review requires schema 1")
    artifact_object(report["approvals"], {"sdk-license-retention-v1"}, "review approvals")
    approval = report["approvals"]["sdk-license-retention-v1"]
    expected = {
        "decision": "approved_artifact_retention", "baseline_revision": BASELINE_REVISION,
        "artifact_revision": ARTIFACT_REVISION, "source_sha256": ARTIFACT_SHA256,
        "artifact_paths": ARTIFACT_PATHS, "preserved_inputs": PRESERVED_INPUTS,
        "scope": "SDK LICENSE byte retention and provenance preservation only",
        "exclusions": ["no legal-compliance conclusion", "no release-approval conclusion", "no relicensing conclusion"],
    }
    artifact_object(approval, {*expected, "reviewed_acceptance_ids", "responsible_agent", "complete_source_review", "justification"}, "approval")
    if any(approval[field] != value for field, value in expected.items()):
        raise SystemExit("artifact approval decision/provenance/scope mismatch")
    ids = approval["reviewed_acceptance_ids"]
    if not isinstance(ids, list) or len(ids) != 2 or any(not isinstance(key, str) for key in ids) or set(ids) != set(ARTIFACT_ALLOWLIST):
        raise SystemExit("artifact approval must review exactly both LICENSE IDs")
    for field in ("responsible_agent", "complete_source_review", "justification"):
        artifact_text(approval[field], field)
    return claims["claims"]


def artifact_input_hashes():
    return {path: sha256(artifact_path(path)) for path in ARTIFACT_INPUTS}


def verify_artifacts():
    reject_snapshot_path(ARTIFACT_EVIDENCE)
    before = artifact_input_hashes()
    claims = validate_artifact_claims(read_json(ARTIFACT_CLAIMS))
    if set(claims) & set(read_json(RUST_CLAIMS)["claims"]):
        raise SystemExit("Rust/artifact claim overlap")
    after = artifact_input_hashes()
    if before != after:
        raise SystemExit("artifact inputs changed during verification")
    evidence = {
        "schema_version": 1, "baseline_revision": BASELINE_REVISION,
        "artifact_revision": ARTIFACT_REVISION, "evidence_kind": "artifact_review",
        "command": ARTIFACT_COMMAND, "profile": ARTIFACT_PROFILE,
        "platform": platform.platform(), "python": platform.python_version(), "exit_code": 0,
        "results": {key: "pass" for key in claims}, "files": after,
    }
    ARTIFACT_EVIDENCE.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    for key in sorted(claims):
        print(f"{key}: artifact_review pass")
    print(f"Artifact review verified: {len(claims)} explicit SDK LICENSE IDs ({ARTIFACT_PROFILE})")


def apply_artifact_evidence(entries, records):
    claims = validate_artifact_claims(read_json(ARTIFACT_CLAIMS), records)
    if set(claims) & set(read_json(RUST_CLAIMS)["claims"]):
        raise SystemExit("Rust/artifact claim overlap")
    evidence = read_json(ARTIFACT_EVIDENCE)
    artifact_object(evidence, {"schema_version", "baseline_revision", "artifact_revision", "evidence_kind",
                              "command", "profile", "platform", "python", "exit_code", "results", "files"}, "evidence")
    expected = {"schema_version": 1, "baseline_revision": BASELINE_REVISION, "artifact_revision": ARTIFACT_REVISION,
                "evidence_kind": "artifact_review", "command": ARTIFACT_COMMAND, "profile": ARTIFACT_PROFILE,
                "exit_code": 0, "results": {key: "pass" for key in claims}}
    if type(evidence["schema_version"]) is not int or type(evidence["exit_code"]) is not int or any(evidence[field] != value for field, value in expected.items()):
        raise SystemExit("artifact evidence requires exact successful command/profile/per-ID results")
    for field in ("platform", "python"):
        artifact_text(evidence[field], field)
    if evidence["files"] != artifact_input_hashes():
        raise SystemExit("stale artifact evidence; run verify-artifacts again")
    for key in claims:
        if key not in entries or entries[key]["scope"] != "in_scope" or entries[key]["disposition"] != "unresolved":
            raise SystemExit(f"cannot verify unknown/excluded/already verified artifact ID: {key}")
    for key, claim in claims.items():
        entries[key].update({
            "disposition": "verified", "disposition_rationale": claim["rationale"],
            "evidence_kind": "artifact_review", "artifact_paths": claim["artifact_paths"],
            "review_approval_reference": claim["review_approval_reference"],
            "implementation_symbols": [], "rust_test_identifiers": [], "target_features": [],
            "command_log": [str(ARTIFACT_EVIDENCE.relative_to(ROOT))],
            "verification_status": "passed", "semantic_closure": "verified",
            "approved_divergence_reference": None,
        })


def verify_rust() -> None:
    claims = read_json(RUST_CLAIMS)
    subprocess.run(["go", "run", "../../fixtures/tools/schema-generate.go", "--check"],
                   cwd=ROOT / "repos/sdk", text=True, capture_output=True, check=True)
    subprocess.run([os.environ.get("CARGO", "cargo"), "build", "--locked", "-p", "adk-tools",
                    "--example", "lifecycle_replay", "--example", "patch_replay"], cwd=ROOT, check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    lifecycle_check = subprocess.run(["go", "run", "../../fixtures/tools/verify-lifecycle.go",
                                     str(target.resolve() / "debug/examples/lifecycle_replay")],
                                    cwd=ROOT / "repos/sdk", text=True, capture_output=True, check=True)
    patch_check = subprocess.run(["go", "run", "../../fixtures/tools/verify-patch.go",
                                 str(target.resolve() / "debug/examples/patch_replay")],
                                cwd=ROOT / "repos/sdk", text=True, capture_output=True, check=True)
    fixture_check = subprocess.run([sys.executable, str(ROOT / "scripts/trace-reference/check.py")],
                                   cwd=ROOT, text=True, capture_output=True, check=True)
    record_check = subprocess.run([sys.executable, str(ROOT / "scripts/check-record-fixtures.py")],
                                  cwd=ROOT, text=True, capture_output=True, check=True)
    host_check = subprocess.run([sys.executable, str(ROOT / "scripts/host-reference/check.py")],
                                cwd=ROOT, text=True, capture_output=True, check=True)
    fileconfig_check = subprocess.run([sys.executable, str(ROOT / "scripts/fileconfig-reference/check.py")],
                                      cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-reference/check.py")],
                                  cwd=ROOT, text=True, capture_output=True, check=True)
    mcp_check = subprocess.run([sys.executable, str(ROOT / "scripts/mcp-runtime-reference/run.py"), "--check"],
                              cwd=ROOT, text=True, capture_output=True, check=True)
    project_state_check = subprocess.run([sys.executable, str(ROOT / "scripts/project-state-runtime-reference/run.py"), "--check"],
                                        cwd=ROOT, text=True, capture_output=True, check=True)
    mcp_prompt_check = subprocess.run([sys.executable, str(ROOT / "scripts/mcp-prompt-reference/run.py"), "--check"],
                                    cwd=ROOT, text=True, capture_output=True, check=True)
    instructions_check = subprocess.run([sys.executable, str(ROOT / "scripts/run-instructions-reference/run.py"), "--check"],
                                    cwd=ROOT, text=True, capture_output=True, check=True)
    workspace_check = subprocess.run([sys.executable, str(ROOT / "scripts/workspace-context-reference/run.py"), "--check"],
                                    cwd=ROOT, text=True, capture_output=True, check=True)
    provider_check = subprocess.run(["bash", str(ROOT / "scripts/providers/generate.sh"), str(ROOT / "repos/sdk"), "--check"],
                                    cwd=ROOT, text=True, capture_output=True, check=True)
    assembler_check = subprocess.run([sys.executable, str(ROOT / "scripts/anthropic-assembler-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    metadata_check = subprocess.run([sys.executable, str(ROOT / "scripts/metadata-compaction-reference/run.py"), "--check"],
                                   cwd=ROOT, text=True, capture_output=True, check=True)
    item_helpers_check = subprocess.run([sys.executable, str(ROOT / "scripts/item-helpers-reference/run.py"), "--check"],
                                        cwd=ROOT, text=True, capture_output=True, check=True)
    llm_summary_check = subprocess.run([sys.executable, str(ROOT / "scripts/llm-summary-reference/run.py"), "--check"],
                                      cwd=ROOT, text=True, capture_output=True, check=True)
    content_logger_check = subprocess.run([sys.executable, str(ROOT / "scripts/content-logger-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    content_stream_check = subprocess.run([sys.executable, str(ROOT / "scripts/content-stream-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    line_writer_check = subprocess.run([sys.executable, str(ROOT / "scripts/line-writer-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    cost_check = subprocess.run([sys.executable, str(ROOT / "scripts/cost-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    progress_check = subprocess.run([sys.executable, str(ROOT / "scripts/progress-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    subagent_check = subprocess.run([sys.executable, str(ROOT / "scripts/replay/subagent_export.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    payload_check = subprocess.run([sys.executable, str(ROOT / "scripts/payload-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    mode_routing_check = subprocess.run([sys.executable, str(ROOT / "scripts/mode-routing-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    mode_codec_check = subprocess.run([sys.executable, str(ROOT / "scripts/mode-codec-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    role_catalog_check = subprocess.run([sys.executable, str(ROOT / "scripts/role-catalog-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    registry_check = subprocess.run([sys.executable, str(ROOT / "scripts/registry-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    content_event_check = subprocess.run([sys.executable, str(ROOT / "scripts/content-event-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    autoloop_check = subprocess.run([sys.executable, str(ROOT / "scripts/autoloop-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    modelsdev_check = subprocess.run([sys.executable, str(ROOT / "scripts/modelsdev-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    agent_kinds_check = subprocess.run([sys.executable, str(ROOT / "scripts/agent-kinds-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    specialist_tools_check = subprocess.run([sys.executable, str(ROOT / "scripts/specialist-tools-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    specialist_catalog_check = subprocess.run([sys.executable, str(ROOT / "scripts/specialist-tools-reference/run.py"), "--catalog", "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    image_analysis_check = subprocess.run([sys.executable, str(ROOT / "scripts/image-analysis-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_tracing_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-tracing-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_compaction_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-compaction-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_input_type_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-input-type-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_filter_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-filter-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_enabled_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-enabled-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_callback_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-callback-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    handoff_constructor_check = subprocess.run([sys.executable, str(ROOT / "scripts/handoff-constructor-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    tool_stopping_check = subprocess.run([sys.executable, str(ROOT / "scripts/tool-stopping-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    dynamic_instructions_check = subprocess.run([sys.executable, str(ROOT / "scripts/dynamic-instructions-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    critic_check = subprocess.run([sys.executable, str(ROOT / "scripts/critic-reference/run.py"), "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    verifier_check = subprocess.run([sys.executable, str(ROOT / "scripts/verifier-reference/run.py"), "--check"],
                                   cwd=ROOT, text=True, capture_output=True, check=True)
    confirmation_check = subprocess.run([sys.executable, str(ROOT / "scripts/confirmation-reference/run.py"), "--check"],
                                       cwd=ROOT, text=True, capture_output=True, check=True)
    userinput_check = subprocess.run([sys.executable, str(ROOT / "scripts/userinput-reference/run.py"), "--check"],
                                    cwd=ROOT, text=True, capture_output=True, check=True)
    action_authorization_check = subprocess.run([sys.executable, "scripts/action-authorization-reference/run.py", "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    expansion_check = subprocess.run([sys.executable, "scripts/sandbox-env-reference/run.py", "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    retry_check = subprocess.run([sys.executable, "scripts/retry-reference/run.py", "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    mcp_display_check = subprocess.run([sys.executable, "scripts/mcp-display-reference/run.py", "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    policy_check = subprocess.run([sys.executable, "scripts/policy-reference/run.py", "--check"], cwd=ROOT, text=True, capture_output=True, check=True)
    command = [os.environ.get("CARGO", "cargo"), "test", "--locked", "-p", "adk", "-p", "adk-codec", "--test", "sandbox_env", "--test", "mcp_display", "--test", "retry", "--test", "policy", "--test", "agents", "--test", "baseline", "--test", "roles", "--test", "modes", "--test", "mode_routing", "--test", "userinput", "--test", "runner_codecs", "-p", "adk-platform", "-p", "adk-runtime",
               "-p", "adk-providers", "--test", "assembler", "--test", "snapshots", "--test", "http", "--test", "routes_cost_parity", "--test", "openai_cost_reference", "--test", "modelsdev",
               "-p", "adk-durable", "-p", "adk-project-state", "--test", "record_codecs",
               "--features", "otel,builder,host,mcp,project-state,execution,observability", "--example", "features", "--test", "resolution", "--test", "host", "--test", "host_cursor", "--test", "host_config", "--test", "host_rules",
               "--test", "compat", "--test", "execution", "--test", "host_history", "-p", "adk-security", "--test", "security",
               "-p", "adk-core", "--test", "authorization", "--test", "contracts", "--test", "result_helpers", "--test", "host_conversation",
               "--test", "usage", "--test", "settings", "--test", "builder", "--test", "catalog_handoffs",
               "--test", "builtin_guardrails", "--test", "builtin_guardrails_oracle",
               "--test", "fileconfig", "--test", "fileconfig_oracle", "-p", "adk-tools", "-p", "adk-mcp",
               "--test", "client", "--test", "session", "--test", "end_to_end", "--test", "diagnostics",
               "--test", "role_views", "--test", "bundle", "--test", "shell", "--test", "shell_security",
               "--test", "fs_input", "--test", "edit", "--test", "write", "--test", "lifecycle",
               "--test", "schema", "--test", "patch",
               "--test", "observability", "--test", "tracestore", "--test", "telemetry", "--test", "telemetry_spans",
               "--test", "progress", "--test", "progress_reference", "--test", "progress_helpers",
               "--test", "progress_runtime", "--test", "progress_pricing",
               "--test", "subagent_reference", "--test", "subagent_activity", "--test", "subagent_tool_contracts", "--test", "payload_surface", "--test", "payload_decode", "--test", "items", "--test", "registry", "--test", "registry_reference",
               "--test", "web",
               "--test", "tracewriter", "--test", "request_snapshot", "--test", "native_snapshot", "--lib",
               "--test", "native_request_snapshot", "--test", "projected_snapshot", "--test", "durable",
               "--test", "telemetry_stdout", "--test", "telemetry_defaults",
               "--test", "tracing", "--test", "tracing_sinks",
               "--test", "tool_behavior", "--test", "guardrails", "--test", "runner", "--test", "subagent", "--test", "subagent_integration", "--test", "tracing_runtime", "--test", "followup", "--test", "llm_compaction", "--test", "local_compaction", "--test", "autoloop", "--test", "content_events", "--test", "line_writer", "--test", "specialists", "--test", "generation_content", "--test", "content_stream", "--test", "content_logger", "--test", "agent_logger"]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True,
                            env={**os.environ, "ADK_TEST_GO": "1"})
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
        "environment": {"ADK_TEST_GO": "1"},
        "exit_code": result.returncode,
        "reference_fixture_check": "Pinned tool schemas reproduced exactly\n" + lifecycle_check.stdout + patch_check.stdout + fixture_check.stdout + record_check.stdout + host_check.stdout + fileconfig_check.stdout + handoff_check.stdout + mcp_check.stdout + project_state_check.stdout + workspace_check.stdout + instructions_check.stdout + mcp_prompt_check.stdout + provider_check.stdout + assembler_check.stdout + metadata_check.stdout + item_helpers_check.stdout + llm_summary_check.stdout + userinput_check.stdout + confirmation_check.stdout + verifier_check.stdout + critic_check.stdout + dynamic_instructions_check.stdout + tool_stopping_check.stdout + handoff_constructor_check.stdout + handoff_callback_check.stdout + handoff_enabled_check.stdout + handoff_filter_check.stdout + handoff_input_type_check.stdout + handoff_compaction_check.stdout + handoff_tracing_check.stdout + modelsdev_check.stdout + action_authorization_check.stdout + expansion_check.stdout + retry_check.stdout + mcp_display_check.stdout + policy_check.stdout + agent_kinds_check.stdout + specialist_tools_check.stdout + specialist_catalog_check.stdout + image_analysis_check.stdout + autoloop_check.stdout + registry_check.stdout + payload_check.stdout + role_catalog_check.stdout + mode_codec_check.stdout + mode_routing_check.stdout + subagent_check.stdout + progress_check.stdout + cost_check.stdout + content_event_check.stdout + line_writer_check.stdout + content_stream_check.stdout + content_logger_check.stdout,
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
    if f"typed-record reference fixtures verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust evidence requires independently executed pinned record fixtures")
    if f"fileconfig reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust fileconfig evidence requires independently executed pinned fixtures")
    if f"handoff reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust handoff evidence requires independently executed pinned fixtures")
    if f"subagent selection reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust subagent selection evidence requires independently executed pinned fixtures")
    if f"final summary reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust final summary evidence requires independently executed pinned fixtures")
    if f"immediate input reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust immediate input evidence requires independently executed pinned fixtures")
    if f"immediate signal reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust immediate signal evidence requires independently executed pinned fixtures")
    if f"builtin guardrails reference verified at {BASELINE_REVISION}" not in evidence.get("reference_fixture_check", ""):
        raise SystemExit("Rust builtin guardrail evidence requires independently executed pinned fixtures")
    for marker in ("Pinned tool schemas reproduced exactly",
                   "Pinned policy observations reproduced exactly",
                   "Pinned MCP display observations reproduced exactly",
                   "Pinned sandbox expansion observations reproduced exactly",
                   "Pinned retry policy observations reproduced exactly",
                   "Pinned action authorization observations reproduced exactly",
                   "272 SDK lifecycle results and resulting filesystem trees verified",
                   "1176 SDK ApplyPatch results and resulting filesystem trees verified",
                   "Pinned agent-kind observations reproduced exactly",
                   "Pinned models.dev observations reproduced exactly",
                   "Pinned specialist tools observations reproduced exactly",
                   "Pinned specialist catalog observations reproduced exactly",
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
                   "Pinned role/catalog observations reproduced exactly",
                   "Pinned mode codec observations reproduced exactly",
                   "Pinned mode routing observations reproduced exactly",
                   "Go subagents: 6 scenarios verified unchanged",
                   "Pinned content event observations reproduced exactly",
                   "Pinned line writer observations reproduced exactly",
                   "Pinned content stream observations reproduced exactly",
                   "Pinned content logger observations and printable ranges reproduced exactly"):
        if marker not in evidence.get("reference_fixture_check", ""):
            raise SystemExit("Rust builder evidence requires independently executed pinned fixtures")
    if evidence.get("environment", {}).get("ADK_TEST_GO") != "1":
        raise SystemExit("Rust record evidence requires live Go record decoding")
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
            "evidence_kind": "rust_test",
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
    apply_artifact_evidence(entries, records)
    counts = Counter(entry["disposition"] for entry in entries.values())
    category_counts = {category: Counter() for category in records}
    for entry in entries.values():
        category_counts[entry["category"]][entry["disposition"]] += 1
    if (len(entries), counts["excluded"], counts["unresolved"] + counts["verified"]) != (TOTAL, EXCLUDED, RETAINED):
        raise SystemExit("ledger no longer has 9,142 IDs, 251 exclusions and 8,891 retained obligations")
    return {
        "schema_version": 1,
        "purpose": "Issue #11 current evidence overlay retaining its historical audit snapshot. Explicit Rust claims require passing command evidence and unchanged source/test hashes. Separately typed SDK LICENSE artifact claims require approved byte/provenance review and fresh per-ID evidence; the generated v0.0.115 baseline is immutable.",
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
            "Separately verify only the two allowlisted SDK LICENSE artifact IDs with approved byte/provenance review; no sibling or verification-group inheritance.",
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
    parser.add_argument("command", choices=("generate", "check", "verify-rust", "verify-artifacts"))
    args = parser.parse_args()
    if args.command == "verify-artifacts":
        verify_artifacts()
        return
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
