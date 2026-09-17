# Pinned SDK Git tools

## Delivered scope

`crates/adk-tools/src/git.rs` implements only `create_pull_request`,
`create_github_issue`, and `attach_repository`, with definitions read from the
existing manifest. No platform-only Git tools and no process-spawning fallback
are included. No new dependencies are needed: `adk-core`, `serde`, and
`serde_json` already exist.

Business workflows include branch guards before staging, dirty/untracked commits,
host attribution, push and explicit PR head, PR recovery/view/fallback, artifact
recording, normalized labels and missing-label creation, credential-free GitHub
URL validation, alias derivation, shallow HTTPS clones, missing-base fallback,
partial-clone cleanup, existing-origin verification, checkout and root excludes.

## Parent wiring required

1. Add `pub mod git;` to `crates/adk-tools/src/lib.rs` (outside this work's scope).
2. Construct `git::tools(runner, repositories, optional_sink, options)` and inject
   the returned tools into `Registry::build`. Do **not** automatically construct
   a default command runner or filesystem host.
3. Set `Options::git_remote_writes` from the same trusted owner setting as
   `Config::git_remote_writes`. Only PR creation is marked `writes_git_remote`
   by the SDK. Issue creation and attachment still obey mutation/name gates.
4. Implement `CommandRunner` with confined Git/GH executables, explicit argv/cwd,
   host context, approval enforcement, credential handling, and cancellation.
   Enforce the shorter of the operation deadline and 60-second local/GH or
   600-second network command timeout. Git hooks, config, credential helpers,
   executable resolution and transports must be confined, not merely the cwd.
   Successful Git output combines stdout/stderr; GH returns only stdout on
   success and stdout followed by stderr on failure. Ordinary exit failures
   belong in `CommandOutput.error`; authorization and infrastructure failures
   must remain `Err` and are not eligible for PR recovery or clone retries.
5. Implement mandatory `commit_message` using the host's attribution policy.
   Returning the original message gives pinned SDK behavior; the SDK itself
   does not mandate a platform co-author. Models cannot provide this policy.
6. Implement `RepositoryHost` using the host's confined filesystem/repository
   attachment facilities. `resolve` must resolve existing symlinks, including
   existing parents of missing paths, and enforce workspace containment. Every
   filesystem operation and command must remain confined at use time; an
   initial canonicalization check is not a sandbox. Stat `.git` as a directory
   (worktree `.git` files are not SDK repositories); unreadable tree inspection
   must not authorize incomplete-clone deletion. Exclude writes must append
   only missing exact lines. Operational filesystem errors use category `Tool`;
   security and infrastructure failures propagate. `remove_all` must support
   authorized best-effort clone cleanup even after operation cancellation,
   without granting new execution permissions.
7. Optional `ArtifactSink` records PR/issue URLs under the host session/run and
   reports recording errors through `warning`. Artifact-store errors cannot
   turn a successful external creation into a failed creation.

The integration test imports the owned source via `#[path]` because editing
`lib.rs` was prohibited. After the module is exported, the parent may replace
that shim with `use adk_tools::git::*`.

## Evidence

`git-cases.py` generates inputs. `git-generate.go` executes the **actual pinned
Go SDK tools** with a fake command runner and temporary repositories, producing
`git-expected.json`. No real Git/GH execution or network writes occur. The 118
fixtures contain exact result text, error flags, argv, cwd, command outcomes,
artifact calls, clone deletion and exclude contents.

Reproduce Go expectations from the workspace:

```sh
python3 fixtures/tools/git-cases.py
cd repos/sdk
GOROOT=/usr/local/go GOTELEMETRY=off go run ../../fixtures/tools/git-generate.go
GOROOT=/usr/local/go GOTELEMETRY=off go test ./pkg/agentsdk/tools/git
```

Rust verification: `cargo test -p adk-tools --test git` using the specified direct
1.97.1 toolchain and linker flags. Eight tests pass, including all 118 exact Go
fixture replays, owner authorization/remote gates, cancellation, in-flight clone
cleanup, symlink path denial, host attribution, nonfatal artifact failure,
missing-workspace creation, manifest registration and command timeouts.

Formatting checks pass. Clippy via the direct `clippy-driver` workspace wrapper
passes on the Git target with `#![deny(warnings)]`. Crate-wide `-D warnings` is
blocked by pre-existing `collapsible_if` findings in `network.rs:17` and
`web.rs:59`; those files were not edited. `cargo-clippy` itself cannot discover
its executable without `/proc/self/exe` in this environment, so the driver was
invoked through `RUSTC_WORKSPACE_WRAPPER`.

## Boundaries not exercised

Production sandbox/command/filesystem/artifact adapters and live GitHub calls are
host responsibilities and have not been exercised here. As in the other Rust
tools, input is already a `serde_json::Value`: malformed raw tool JSON and
original duplicate-key ordering cannot be reproduced at this boundary. Raw GH
label JSON error diagnostics have fixture coverage but are translated from
serde diagnostics rather than implemented with Go's complete JSON scanner.
No library registration or dependency files were edited and no commits made.
