# Issue #10 acceptance evidence map

This map records evidence relevant to [issue #10](https://github.com/gratefulagents/adk/issues/10), **not** a maintainer acceptance decision. Final local verification: 748 workspace tests passed, 0 failed, 30 existing opt-in tests ignored; all 68 subagent tests below passed without ignores. Actual Go regeneration `--check` was unchanged. Independent review and pushed-head CI are reported in the PR completion comment.

## Provenance and boundaries

- **Upstream SDK:** `v0.0.115`, commit `1dc92b73900fac74dc357a938e4b5eee6392b418`.
- **Go upstream evidence:** `go-upstream-tests.json` records **103 distinct, executed, passing genuine upstream Go test cases** from that revision, across `internal/agent`, `pkg/agentsdk`, and `pkg/agentsdk/runtime`. The manifest records its command, Go version, package outcomes, and raw JSON log checksum.
- **Rust evidence:** the source inventory below contains **68 Rust test functions**: **62 separate language-native tests** (33 scheduler, 21 runner/integration, 4 tool-contract, and 4 actual activity lifecycle) plus **6 differential reference-comparison tests**. The language-native tests are separate from the differential tests; neither set claims a one-to-one Go counterpart.
- **Differential evidence:** the generated Go fixture contains **6 scenarios and 44 actual Go model-facing tool calls**. Rust replays the same scenario inputs through its tools, scheduler, child executor, and runner. This is narrower than the native suites and does **not** establish blanket parity.
- **Parent-context limit:** the differential harness uses empty parent history. Its `parent_secret=false` observation does **not** test sharing or isolation against seeded parent history. Parent-history sharing is covered separately by language-native tests and upstream tests; it is not differential evidence.

## Acceptance-to-evidence map

| Issue #10 acceptance | Exact upstream Go cases in the manifest | Rust language-native evidence | Differential coverage and limits |
| --- | --- | --- | --- |
| Deterministic DAG/dependency success, failure, cancel, and concurrency | `TestSubagentToolBatchRejectsCyclesAndUnknownDeps`; `TestSubagentToolBatchSyncWaitsForWholeGraph`; `TestSubagentToolBatchWaitDeadlineLeavesTasksRunning`; `TestSubagentWaitForAnyDeliversEachResultOnce`; `TestSubAgentRegistryCancelMarksTaskCancelledImmediately`; `TestSubAgentRegistryWaitsForDependenciesAndInjectsResults`; `TestSubAgentRegistryConcurrentConfigureAndSpawn` | `atomic_dag_validation_and_dependency_forwarding`; `dependency_failure_policies_and_forwarding_opt_out`; `shared_concurrency_budget_and_incremental_delivery`; `cancel_running_and_waiting_does_not_cancel_parent`; `dag_keys_are_local_to_each_call_and_graph_retains_resolved_edges`; `malformed_batch_is_atomic_and_does_not_dispatch_valid_prefix` | `dag_sync_isolation`, `dependency_failure`, `background_wait_any`, `cancel`, and `dag_background` cover the defined fixture paths. They do not replace the separate native concurrency and cancellation suites. |
| Final join, incremental delivery, result reread, and event-driven wake | `TestRunnerAutoJoinsSubAgentResultsBeforeFinal`; `TestRunnerExtendsFinalTurnForSubAgentJoinItems`; `TestRunnerWaitsForSubAgentJoinAfterTurnBudget`; `TestSubagentResultsDeliveredIncrementallyAtTurnBoundary`; `TestSubagentStatusResultsRefetchesDeliveredResults`; `TestSubagentWaitBlocksUntilTaskFinishesAndDeliversResult`; `TestSubAgentRegistryBroadcastWakesAllWaiters` | `steering_ids_acknowledgements_notifications_and_finish_gate`; `wait_any_ignores_delivered_and_activity_timeout_does_not_cancel`; `background_final_answer_waits_and_delivers_exactly_once`; `failed_final_join_checkpoint_keeps_result_available`; `output_returning_stop_after_tool_cannot_bypass_child_final_join`; `final_join_includes_active_descendants_of_an_already_delivered_child` | All six scenarios read terminal views and reread results; `background_wait_any` exercises mixed completed/active wait-any and empty wait. It does not measure wake latency. |
| Child context/security isolation and no escalation | `TestApprovedSubagentSharesParentContext`; `TestSubagentToolShareParentContext/isolated_by_default`; `TestSubagentToolShareParentContext/opt_in`; `TestSubAgentRegistryAsyncSpawnInheritsCurrentTurnReadOnlyClamp`; `TestSubAgentRegistryTaskInheritsToolInputGuardrails`; `TestSubAgentRegistry_H4_ClampsChildAccessToParentReadOnly` | `fresh_context_explicit_history_and_common_sync_background_engine`; `every_security_dimension_is_monotone`; `sync_result_is_not_injected_again_and_readonly_clamps_live_policy`; `agent_as_tool_shares_child_engine_and_explicit_parent_context_is_paired`; `batch_access_cannot_override_call_read_only_and_dependency_forwarding_can_be_disabled` | `dag_sync_isolation` and `dag_background` observe tool narrowing and dependency-result visibility. Empty parent history means they do **not** differentially prove parent-history sharing or isolation. |
| Recovery separates safe resume from ambiguous effects; steering is retained exactly once | `TestRestoredActiveChildRequiresExplicitReconciliation`; `TestResumeRestoredCompletedChildDoesNotDispatch`; `TestResumeRestoredTaskRejectsUnreconciledOrWeakerSecurity`; `TestResumeRestoredTaskWithoutRunnerCheckpointRelaunchesSameID`; `TestReconcileRestoredTaskPersistsTerminalDecision`; `TestRestoreRequeuesInFlightSteeringBeforeQueued`; `TestSubAgentRegistrySendMessageInterruptsInFlightModelAttempt` | `restore_never_dispatches_queued_work_without_explicit_resume`; `restored_running_effect_and_steering_journal_require_reconciliation`; `accepted_but_unprocessed_steering_cannot_disappear_as_success`; `native_completed_checkpoint_resumes_without_replaying_provider`; `native_dispatched_effect_is_never_automatically_resumed`; `native_child_resume_preserves_tool_cursor_and_narrows_policy` | No differential scenario covers restored/reconciling state, checkpoint resume, or steering. This acceptance area relies on the separate Go and Rust native tests. |
| Session shutdown cancels/joins owned tasks and persists required state | `TestSessionStateCloseCancelsActiveSubAgentTasks`; `TestBuilderOwnedSessionStateIsRegisteredAsCloser`; `TestSubAgentSpawnFailsClosedWhenCheckpointFails`; `TestFailedSpawnDoesNotExposeProvisionalSnapshot`; `TestChildRunnerCheckpointIsPersistedWithSecurityBaseline` | `shutdown_joins_children_and_retained_handles_cannot_restart_owner`; `owner_drop_aborts_child_futures_and_closes_retained_handles`; `persistence_is_reentrant_committed_only_and_precedes_dispatch`; `failed_dispatch_persistence_never_executes_or_exposes_provisional_state` | No differential scenario closes a session or restores persisted state. This acceptance area relies on the separate Go and Rust native tests. |

## Differential fixture: actual cross-language comparison

The fixture at `crates/adk-runtime/tests/fixtures/go-subagents.json` is generated from the public Go SDK fixture program, not hand-authored as Rust expectations. Its provenance records the SDK revision, source hashes, generator/exporter checksums, and Go command.

| Scenario | Actual Go tool calls |
| --- | ---: |
| `single_sync` | 7 |
| `dag_sync_isolation` | 6 |
| `dependency_failure` | 6 |
| `background_wait_any` | 10 |
| `cancel` | 8 |
| `dag_background` | 7 |
| **Total** | **44** |

`subagent_reference.rs` compares generated IDs and existing timing values only as documented by the fixture. It preserves missing-versus-null-versus-empty distinctions; it does not erase differing response fields, flags, statuses, or diagnostic text. All six comparison gates passed in the final workspace run, with no ignored or relaxed assertions.

## Rust test inventory (68 functions: 62 language-native + 6 differential)

### `crates/adk-runtime/tests/subagent.rs` — 33 tests

Scheduler, ownership, security, and recovery.

- `fresh_context_explicit_history_and_common_sync_background_engine`
- `atomic_dag_validation_and_dependency_forwarding`
- `dependency_failure_policies_and_forwarding_opt_out`
- `shared_concurrency_budget_and_incremental_delivery`
- `wait_any_ignores_delivered_and_activity_timeout_does_not_cancel`
- `steering_ids_acknowledgements_notifications_and_finish_gate`
- `shutdown_joins_children_and_retained_handles_cannot_restart_owner`
- `cancel_running_and_waiting_does_not_cancel_parent`
- `scoped_delegation_enforces_depth_and_yields_slot_without_deadlock`
- `persistence_is_reentrant_committed_only_and_precedes_dispatch`
- `failed_dispatch_persistence_never_executes_or_exposes_provisional_state`
- `restore_never_dispatches_queued_work_without_explicit_resume`
- `restored_running_effect_and_steering_journal_require_reconciliation`
- `cancel_wins_while_resume_persistence_is_blocked`
- `failed_reconciliation_rolls_back_and_preserves_delivery`
- `restore_rejects_inconsistent_or_weakened_baselines_atomically`
- `every_security_dimension_is_monotone`
- `dropped_persistence_future_poison_is_latched_and_snapshot_fails_closed`
- `fenced_ledger_rejects_stale_parent_queued_snapshot`
- `last_shared_turn_is_usable_but_next_turn_is_denied`
- `accepted_but_unprocessed_steering_cannot_disappear_as_success`
- `child_panic_has_terminal_evidence_and_does_not_kill_scheduler`
- `native_completed_checkpoint_resumes_without_replaying_provider`
- `native_dispatched_effect_is_never_automatically_resumed`
- `stricter_security_resume_checkpoint_roundtrips_again`
- `delegated_non_tool_security_survives_root_restore`
- `owner_drop_aborts_child_futures_and_closes_retained_handles`
- `dropping_suspended_nested_wait_cancels_owned_subtree`
- `native_model_completed_and_tool_prepared_resume_only_remaining_work`
- `scoped_wait_timeout_reacquires_slot_without_cancelling_parent`
- `recorded_timing_and_steering_survive_checkpoints`
- `cancellation_and_failed_dependencies_record_terminal_metadata`
- `structured_activity_records_observed_tool_lifecycle`

### `crates/adk-runtime/tests/subagent_integration.rs` — 21 tests

Runner and managed-tool integration.

- `background_final_answer_waits_and_delivers_exactly_once`
- `sync_result_is_not_injected_again_and_readonly_clamps_live_policy`
- `sync_timeout_keeps_child_alive_across_runs_and_status_can_reread`
- `tool_policy_timeout_preserves_managed_pending_results`
- `nested_tool_policy_timeout_resumes_without_cancelling_children`
- `agent_as_tool_shares_child_engine_and_explicit_parent_context_is_paired`
- `durable_runner_binds_native_scheduler_without_dynamic_turn_context`
- `steering_interrupts_pending_model_and_counts_attempts_once`
- `denied_child_calls_do_not_consume_dispatch_budget`
- `failed_parent_checkpoint_does_not_consume_child_result`
- `native_durable_parent_refuses_unbacked_scheduler`
- `steering_after_visible_output_waits_for_safe_boundary`
- `ordinary_child_retains_retry_policy_and_attempt_accounting`
- `native_child_resume_preserves_tool_cursor_and_narrows_policy`
- `cost_is_reported_cumulatively_not_rounded_per_response`
- `managed_child_tools_rebind_to_actual_scheduler_and_release_single_slot`
- `failed_final_join_checkpoint_keeps_result_available`
- `steering_does_not_interrupt_started_tool`
- `failed_child_retains_bounded_assistant_progress_tail`
- `output_returning_stop_after_tool_cannot_bypass_child_final_join`
- `final_join_includes_active_descendants_of_an_already_delivered_child`

### `crates/adk-runtime/tests/subagent_activity.rs` — 4 tests

Observed activity through real RunnerChildExecutor, not manual helper calls.

- `native_runner_persists_observed_file_activity`
- `native_runner_preserves_overlapping_tool_activity`
- `native_runner_persists_failed_tool_activity`
- `native_runner_preserves_activity_after_parallel_tool_error`

### `crates/adk-runtime/tests/subagent_tool_contracts.rs` — 4 tests

Model-facing tool contracts.

- `dag_keys_are_local_to_each_call_and_graph_retains_resolved_edges`
- `malformed_batch_is_atomic_and_does_not_dispatch_valid_prefix`
- `batch_access_cannot_override_call_read_only_and_dependency_forwarding_can_be_disabled`
- `explicit_wait_any_returns_when_every_result_was_already_delivered`

### `crates/adk-runtime/tests/subagent_reference.rs` — 6 tests

Independent Go-fixture comparison.

- `go_fixture_provenance_matches_generator`
- `model_facing_schemas_match_pinned_go`
- `tool_metadata_matches_pinned_go`
- `tool_result_fields_match_pinned_go`
- `scheduler_outcomes_match_pinned_go`
- `actual_child_model_requests_match_pinned_go`

## Executed upstream Go inventory (103 genuine upstream cases)

The following names are copied from `docs/verification/subagents/go-upstream-tests.json`; each is an executed test case, including explicit Go subtests. The manifest is the authoritative execution record, not a statement of current Rust acceptance.

### `github.com/gratefulagents/sdk/internal/agent` — 75 cases

- `TestAgentToolUsesNestedSubAgentMaxTurns`
- `TestAsyncSubagentInheritsParentModelRetryAndTimeout`
- `TestBuildSubAgentBudgetContext`
- `TestCheckpointHookCanReenterSchedulerCheckpoint`
- `TestChildRunnerCheckpointIsPersistedWithSecurityBaseline`
- `TestDurableResumePreservesPriorUsage`
- `TestEventStreamSubagentCompletionIncludesCacheUsage`
- `TestEventStreamSubscribeReceivesChildSubagentEvents`
- `TestFailedSpawnDoesNotExposeProvisionalSnapshot`
- `TestProgressTracker_RecordSubagentLifecycle`
- `TestReconcileRestoredTaskPersistsTerminalDecision`
- `TestReconcileRestoredTaskRollsBackWhenCheckpointFails`
- `TestRecordSubagentCompletedDoesNotDoubleCountForwardedUsage`
- `TestRestoreRequeuesInFlightSteeringBeforeQueued`
- `TestRestoredActiveChildRequiresExplicitReconciliation`
- `TestResumeRestoredCompletedChildDoesNotDispatch`
- `TestResumeRestoredTaskCancellationDuringPersistenceDoesNotDispatch`
- `TestResumeRestoredTaskRejectsUnreconciledOrWeakerSecurity`
- `TestResumeRestoredTaskRejectsUnreconciledOrWeakerSecurity/approval_pending`
- `TestResumeRestoredTaskRejectsUnreconciledOrWeakerSecurity/unknown_boundary`
- `TestResumeRestoredTaskRejectsUnreconciledOrWeakerSecurity/unreconciled_model`
- `TestResumeRestoredTaskRejectsUnreconciledOrWeakerSecurity/weaker_policy`
- `TestResumeRestoredTaskRejectsUnsupportedSchemaWithSentinel`
- `TestResumeRestoredTaskWithoutRunnerCheckpointRelaunchesSameID`
- `TestRunConfigEffectiveSubAgentMaxTurns`
- `TestRunStreamedEmitsSubAgentStreamEvents`
- `TestRunSubAgentOnceFailureKeepsPartialUsage`
- `TestRunSubAgentOnceForcesStableTaskPromptCacheKey`
- `TestRunSubAgentOnceModelFailureSurfacesPartialProgress`
- `TestRunnerAutoJoinsSubAgentResultsBeforeFinal`
- `TestRunnerEmitsSpawnedSubagentLifecycleForAgentTool`
- `TestRunnerExtendsFinalTurnForSubAgentJoinItems`
- `TestRunnerWaitsForSubAgentJoinAfterTurnBudget`
- `TestSubAgentActivity_BriefStatus`
- `TestSubAgentActivity_ConcurrentAccess`
- `TestSubAgentActivity_CurrentTool`
- `TestSubAgentActivity_RecentToolsRingBuffer`
- `TestSubAgentActivity_RecordToolEnd_TracksReads`
- `TestSubAgentActivity_RecordToolEnd_TracksWrites`
- `TestSubAgentActivity_SnapshotIncludeRecent`
- `TestSubAgentActivity_StepInference`
- `TestSubAgentActivity_StepInference/Bash_git_add_.`
- `TestSubAgentActivity_StepInference/Bash_git_commit_-m_fix`
- `TestSubAgentActivity_StepInference/Bash_git_diff_HEAD`
- `TestSubAgentActivity_StepInference/Edit_file.go`
- `TestSubAgentActivity_StepInference/LSP_hover`
- `TestSubAgentActivity_StepInference/Write_file.go`
- `TestSubAgentRegistryAsyncSpawnInheritsCurrentTurnReadOnlyClamp`
- `TestSubAgentRegistryBroadcastWakesAllWaiters`
- `TestSubAgentRegistryCancelMarksTaskCancelledImmediately`
- `TestSubAgentRegistryConcurrentConfigureAndSpawn`
- `TestSubAgentRegistryConfigurePreservesTrackedTasks`
- `TestSubAgentRegistryPassesCompactionConfigToAsyncRuns`
- `TestSubAgentRegistryPassesMaxTurnsToAsyncRuns`
- `TestSubAgentRegistryRestoreSchedulerCheckpointRejectsNonFreshAndDuplicateIDs`
- `TestSubAgentRegistrySchedulerCheckpointRestore`
- `TestSubAgentRegistrySendMessageInterruptsInFlightModelAttempt`
- `TestSubAgentRegistrySendMessageQueuesParentMessage`
- `TestSubAgentRegistrySetStatusDoesNotReopenCancelledTask`
- `TestSubAgentRegistrySpawnUsesShortTaskIDs`
- `TestSubAgentRegistryTaskInheritsToolInputGuardrails`
- `TestSubAgentRegistryWaitsForDependenciesAndInjectsResults`
- `TestSubAgentRegistry_H4_AllowsDowngradeOverride`
- `TestSubAgentRegistry_H4_ClampsChildAccessToParentReadOnly`
- `TestSubAgentSpawnFailsClosedWhenCheckpointFails`
- `TestSubAgentTaskPanicFailsTaskInsteadOfCrashing`
- `TestSubagentCompletedWithoutStartFallsBack`
- `TestSubagentSpanPairing`
- `TestWireDurableChildrenAutoWiresCheckpointCallback`
- `TestWireDurableChildrenFailsClosedForActiveRecordsWithoutScheduler`
- `TestWireDurableChildrenIgnoresTerminalRecordsWithoutScheduler`
- `TestWireDurableChildrenKeepsHostProvidedCallback`
- `TestWireDurableChildrenPropagatesRestoreError`
- `TestWireDurableChildrenRestoresIntoEmptyScheduler`
- `TestWireDurableChildrenSkipsRestoreWhenHostAlreadyRestored`

### `github.com/gratefulagents/sdk/pkg/agentsdk` — 25 cases

- `TestApprovedSubagentSharesParentContext`
- `TestSubagentControlMessageAndCancel`
- `TestSubagentResultsDeliveredIncrementallyAtTurnBoundary`
- `TestSubagentStatusDetailsActivityAndGraph`
- `TestSubagentStatusResultsListsActiveTasks`
- `TestSubagentStatusResultsRefetchesDeliveredResults`
- `TestSubagentStatusSummaryReturnsReportableSnapshot`
- `TestSubagentToolBackgroundModeManagedJoinProviderReturnsResultOnce`
- `TestSubagentToolBackgroundModeManagedJoinProviderWaitsInSDKUntilResult`
- `TestSubagentToolBatchInheritsCallLevelAgentName`
- `TestSubagentToolBatchRejectsCyclesAndUnknownDeps`
- `TestSubagentToolBatchSyncWaitsForWholeGraph`
- `TestSubagentToolBatchTimeoutMarksCompletedResultsDelivered`
- `TestSubagentToolBatchWaitDeadlineLeavesTasksRunning`
- `TestSubagentToolPreservesPendingResultThroughRunnerToolPolicyTimeout`
- `TestSubagentToolRequiresExactlyOneOfMessageOrTasks`
- `TestSubagentToolShareParentContext`
- `TestSubagentToolShareParentContext/isolated_by_default`
- `TestSubagentToolShareParentContext/opt_in`
- `TestSubagentToolSyncModeWaitsAndReturnsResult`
- `TestSubagentToolSyncWaitDeadlineLeavesTaskRunning`
- `TestSubagentWaitBlocksUntilTaskFinishesAndDeliversResult`
- `TestSubagentWaitForAnyDeliversEachResultOnce`
- `TestSubagentWaitNoTasksInvalidInputAndUndelivered`
- `TestSubagentWaitTimeoutReturnsSnapshotNotError`

### `github.com/gratefulagents/sdk/pkg/agentsdk/runtime` — 3 cases

- `TestBuilderOwnedSessionStateIsRegisteredAsCloser`
- `TestBuilderReusesSessionStateSubAgentSchedulerAcrossBuilds`
- `TestSessionStateCloseCancelsActiveSubAgentTasks`

## Decision boundary

The evidence is intentionally partitioned:

1. The 103 Go cases establish what the pinned upstream revision executed.
2. The 57 scheduler, integration, and tool-contract Rust tests exercise native behavior.
3. The 6 reference tests exercise the generated, six-scenario/44-call differential path.

Neither the upstream run nor the differential fixture certifies all Rust behavior. In particular, the differential fixture must not be used to claim parent-history sharing parity, because its parent history is empty. The parent must rerun the relevant evidence after its changes and make the independent final acceptance decision.
