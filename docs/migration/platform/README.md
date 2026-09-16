# Platform handoff artifacts

These are source-derived migration contracts, **not an implemented Rust worker**. See [baseline.md](baseline.md) for launch, Postgres, durable state, encrypted S3, CLI, Slack, desktop and capability boundaries; [../baseline/README.md](../baseline/README.md) for executed Go/replay evidence.

## Reproduce and verify

Run from workspace root (Python 3 and Go standard library only; no platform/SDK dependency build):

```sh
python3 scripts/platform/closure_inventory.py > docs/migration/platform/closure.json
env GOROOT=/usr/local/go GOTOOLCHAIN=local go run scripts/platform/environment_inventory.go > docs/migration/platform/environment.json
env GOROOT=/usr/local/go GOTOOLCHAIN=local go test -v scripts/platform/environment_inventory.go scripts/platform/environment_inventory_test.go
env GOROOT=/usr/local/go GOTOOLCHAIN=local go vet scripts/platform/environment_inventory.go scripts/platform/environment_inventory_test.go
python3 scripts/platform/verify.py
```

Both generators reject a changed source pin or dirty source repository. They read sources and emit stdout; verification regenerates in memory. Explicit GOROOT works around this host's launcher configuration. A telemetry-sidecar `/proc/self/exe` warning may appear but is nonfatal. No inventory tests access databases, providers, Kubernetes or external services.

## Closure and package acceptance

`closure.json` contains all 62 local packages reached from platform `cmd/agent` and `internal/tools`, file hashes, import witnesses, build constraints and embed declarations. This is the **union of all build variants**, not a target-specific `go list` or reachable-symbol graph. Tests add zero packages. Embed patterns are recorded, not expanded. The extracted architect generator uses a regex import scanner appropriate to these pinned files, not a general Go parser; verification independently compares every closure file's imports against Go AST results.

There are 470 production files and 373 test files; sorted production paths, separated and terminated by newline, hash to:

```
b42f09f901303e913d871b439ea457d83e6907daf77482e742120c481b6fbcca
```

Every package has a proposed `rust_module`, `role_owner`, unique `acceptance_id`, `capability_acceptance_id`, `acceptance_contract`, `disposition`, `source_test_files`, `implementation_status` and `verification_status`. Owners are responsibility roles, not assigned people. IDs are `PLAT-PKG-` plus 16 uppercase SHA-256 hex characters of the import path. Capability IDs link baseline §8; SDK packages delegate finer acceptance/routing to the [SDK ledger](../ledger/README.md). Test associations are candidate regressions, not measured coverage. Control-plane inclusion does not require porting controller implementations.

All **278 platform production entries** include exact UTF-8 source `content`, including all **50 internal/tools files**. This preserves complete schemas, defaults, dynamic wrappers and feature gates. SDK exact source is archived separately in its ledger. `environment.json.registration_expressions` contains 606 registration/composition expressions, including 46 worker-loop calls. These are **not tool counts**; dynamic names and configuration-dependent registration require behavioral replay.

## Environment producer/consumer expressions

`environment.json` scans all **1,178 tracked Go files**, all build tags and tests, deliberately wider than the closure to capture controller/Slack/SandboxClaim producers. Tests are marked, not interpreted as deployment requirements.

- `records`: 1,979 exact expressions, line ranges, test flags and classifications: direct process consumers/producers; Env-named helpers, definitions, fields, assignments and forwarding loops; explicit/inferred Kubernetes EnvVar literals.
- `contexts`: 606 exact enclosing declarations, retaining defaults, conditions, provider switches and dynamic-name helper bodies.
- `literal_key_index`: 293 uppercase **candidate** literals linked to expression IDs. Nested definitions intentionally over-approximate; not every literal is an environment key.
- `constant_key_candidates`: directory-qualified constant/variable definitions, including SDK sandbox constants. Consumers may use identifiers rather than literals.
- `scanned_files`, `source_sha256`, `go_imports`: exact scanned scope, digests and independent AST imports.
- `deployment_shell_candidates`: 6,564 textual witnesses from tracked YAML, shell and Dockerfiles, including CI/test fixtures. These are supplementary candidates, not parsed Kubernetes environment rows.

“Exhaustive” means expression coverage for the stated syntactic extraction classes, **not whole-program dataflow resolution, a finite enumeration of dynamic names, or third-party implicit reads**. Env-name matching deliberately admits false positives (including envelope-related functions). Exact context/source makes unresolved dynamic expressions auditable. No secret values are resolved.

### Important dynamic producer → consumer relationships

| Family | Producer / definition | Consumer / forwarding |
|---|---|---|
| Required worker identity/model, repository/workspace, features | Controller `pod_support.go:buildCommonPodSpec` and common SandboxClaim launch helpers | Worker `config.go:loadRunConfig`, `loop.go`, `project_state.go`; required/default behavior in baseline §3 |
| Infra credentials | `workerInfraSecretEnv`; Secret references to `gratefulagents-worker-infra` | DATABASE_URL session/durable stores; S3/AWS workspace/content stores |
| Provider keys | `pod_support.go:providerEnvVarName` / `providerAPIKeyEnvs` | `config.go:providerAPIKeyEnvName` / `providerAPIKeysFromEnv`; preserve both switches and selected-provider fallback expressions |
| Ordered OAuth fallback paths | `openAIOAuthEnvs`, `providerOAuthEnvs`, `providerOAuthFallbackEnvs` | `oauthFallbacksFromEnv`; OpenAI auth/account paths are index-aligned |
| MCP generated keys | `internal/mcpattach/secret_env.go:SecretEnvPodName`, controller `resolveMCPServerSecretEnvs` | Worker MCP config resolves generated pod key into original server key. Formula: GRATEFULAGENTS_MCP_ + uppercase hex of first six SHA-256 bytes of trim(serverName) + NUL + trim(envName) + underscore + sanitized uppercase key (fallback SECRET) |
| Sandbox constants/child env | SDK `SandboxConfigEnvNames`, controller `commandSandboxConfigEnvs`, RuntimeProfile forwarding, worker toolkit setup | SDK `ConfigFromEnv`, `SafeEnv*`, subprocess backends. PATH replace/prepend/append, RO/RW roots, extra env, Kubernetes exposure remain distinct; inheritance is allowlisted plus LC_* |
| Mode constraints | `modeConstraintEnvs`: MODE_MAX_TURNS, MODE_SUBAGENT_MAX_TURNS, MODE_MAX_CONCURRENT_CHILDREN, MODE_NAME | Worker mode overrides and SDK configuration; preserve helper defaults |
| Run identities | `runtimeScopeEnvs`, `mode_overrides.go:setRuntimeParentMetadataEnv` | AGENTRUN_CURRENT_*, AGENTRUN_PARENT_*, supervised/maintained identities; RUN_* means parent, not current |
| Tool narrowing/output | `toolPolicyEnvs` | AGENTRUN_ALLOWED_TOOLS, AGENTRUN_DENIED_TOOLS, AGENTRUN_TASK_OUTPUT_SCHEMA; deny wins; control-flow exemptions retained |
| Slack | Trigger controller `slackagent_helpers.go`, worker secret helpers | Worker `slack.go`, `slack_workspace.go`, Slack tools; connector and run identity differ |

Controller helpers above are under `repos/gratefulagents/internal/controller/platform`, worker helpers under `repos/gratefulagents/cmd/agent`, SDK sandbox under `repos/sdk/pkg/agentsdk/sandbox`. Exact declarations and line witnesses are in the inventory. This table highlights relationships; it does not replace the full ledger.

## Annotation/label and generated-trigger metadata

`metadata_key_and_access_expressions` contains 1,249 qualified metadata literal, Annotation/Label declaration, and Annotations/Labels index-access witnesses, including tests. Constant-key accesses are preserved. Other qualified-domain keys and finalizers are intentionally included as candidates.

Coverage includes canonical AgentRun annotations plus trigger/maintainer runtime-trigger-name, generated-runtime, project-name/uid, project-trigger-name/type, PR-loop/review-round and maintainer state. This is a syntactic inventory, not proof that all consumers have been ported. Exact platform source retains additional indirection and mutation logic.

## Provenance and licensing

Platform revision `08e65c970830f05042c251bcbb46ec6a9e3719b9` is AGPLv3; SDK v0.0.115 revision `1dc92b73900fac74dc357a938e4b5eee6392b418` is GPLv3. See [source lock](../source-lock.json), [platform license](../licenses/platform-LICENSE), [SDK license](../licenses/sdk-LICENSE). Archived source remains subject to those licenses; this handoff does not authorize relicensing. Neither source repository was changed or committed.
