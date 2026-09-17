# AnalyzeImage pinned contract and integration

Baseline: SDK v0.0.115, revision `1dc92b73900fac74dc357a938e4b5eee6392b418`, `pkg/agentsdk/tools/vision/vision.go` and its tests, manifest `AnalyzeImage` / `SDK-17BB44A3499484F4`.

## Important baseline correction

The prerequisite table in `repos/sdk/docs/tool-surface.md` still says an analyzer is required. Actual pinned source and `TestExecuteReturnsNativeImage` supersede that statement: the tool returns the prompt plus a base64 native image attachment and **must not invoke a separate analyzer**, even when configured. Rust `vision::Config::analyzer` retains a host-injection boundary, without a host-specific implementation or a call to it. `Content::Attachment` preserves media type, data, and normalized detail (`low`, otherwise `high`).

## Parent-owned wiring still required

No shared lib/Cargo/network/workspace files were intentionally edited by this work.

1. Export `pub mod vision;` in `crates/adk-tools/src/lib.rs`.
2. Add `.or_else(|| vision::builtin(capability, config.allow_private_network_urls))` to registry construction after supplied implementations. Unlike Browser, Vision must remain available with public-only URL policy; do not gate it on private-network opt-in.
3. Hosts needing managed Browser screenshots supply `vision::tool(vision::Config { allowed_image_dirs, allow_private_network_urls, analyzer, ..Default::default() })` through existing registry injection. Only host-provided absolute directories authorize paths outside the workspace; model arguments cannot modify either policy.
4. Tests currently compile the owned module plus existing network/workspace modules using `#[path]` so they run before shared wiring. After wiring, these can be replaced by `use adk_tools::vision;` and the test of crate-private `builtin` can construct `vision::tool(Default::default())`, or retain the isolated harness for implementation tests and add default-registry coverage in parent-owned registry tests.

Dependencies already present: `adk-core`, `serde`, `serde_json`, `base64`, `tokio` (including blocking tasks, time, net, test io-util), `reqwest`; shared `network::{resolve,client}` and `workspace::Workspace`. No new Cargo dependency. Native file confinement requires Linux openat2; URL-only use does not.

## Coverage

`vision.json` has 19 contract cases, executed twice with/without analyzer injection by Rust tests and `vision-verify.go` against actual pinned Go source (38 result comparisons). Covers prompt/source requirements, null/default handling, invalid types, case-insensitive input field names, file-over-URL precedence, detail normalization, MIME extension/magic precedence including dotfiles and space-bearing filenames, and exact attachment bytes.

Rust integration tests additionally cover host-managed absolute screenshots, rejected relative/outside/traversal/symlink/hardlink/nonregular/FIFO paths, missing files, exact 20 MiB acceptance and oversized stat preflight; URL Content-Type precedence/fallback, ignored URL extension, raw compressed bytes, bot User-Agent/no Accept-Encoding, all five redirect codes and redirect-body fallback, rejected embedded credentials/schemes/private addresses, model policy override attempts, HTTP errors, truncated bodies, exact and chunked oversized response limits, 15-second timeout, and cancellation/deadline interruption during headers and body with connection closure. Manifest contract and read-only Vision versus Browser selection are checked.

## Explicit gaps / deliberate confinement differences

- Shared descriptor-relative workspace reads reject **all** symlinks and hard links. Pinned vision resolves in-root symlinks and does not reject hard links. This implementation intentionally retains the stronger existing Rust confinement instead of canonicalize-then-open races. Absolute aliases through symlinked roots and paths that temporarily climb above a root and return may consequently fail closed.
- Unsupported filesystem platforms fail closed. Cross-platform loading, non-UTF-8 host directory names, malformed/non-ASCII HTTP header values, DNS rebinding against a live resolver and TLS redirects are not established by this fixture suite. Public/private DNS pinning is delegated to the shared network primitive; initial literal/local-host policy and redirect revalidation are tested.
- Operation cancellation drops network futures promptly. File I/O runs on a bounded blocking task so cancellation returns promptly, but already-running filesystem system calls cannot be forcibly interrupted. The detached read still obeys the 20 MiB + 1 read cap.
- OS/HTTP/parser error diagnostics retain SDK prefixes and result/error classification, not byte-identical platform-specific Go messages. Tool arguments are already a `serde_json::Value`; duplicate JSON field ordering is therefore unavailable. Native output, stable validation messages and size errors are exact in the fixtures.

## Verification commands

Use the supplied Rust toolchain environment (`PATH` starts with `/usr/local/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin`, `LD_LIBRARY_PATH` points to its `lib`, `RUSTFLAGS='-C linker-features=-lld -C link-arg=-fuse-ld=bfd'`):

- `cargo test -q -p adk-tools --test vision` — 11 passed (includes two shared network tests), including real 15-second request timeout.
- `rustfmt --edition 2024 --config skip_children=true --check crates/adk-tools/src/vision.rs crates/adk-tools/tests/vision.rs` — use `skip_children` to avoid formatting parent-owned modules through the test harness.
- From `repos/sdk`: `GOROOT=/usr/local/go GOTOOLCHAIN=local GOTELEMETRY=off /usr/local/go/bin/go run ../../fixtures/tools/vision-verify.go` — 38 pinned results verified. The host prints a benign telemetry `/proc/self/exe` warning before successful execution.

Normal `cargo clippy` cannot start in this host because `/proc/self/exe` is unavailable. A direct `RUSTC_WORKSPACE_WRAPPER=/usr/local/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/clippy-driver cargo rustc -p adk-tools --test vision -- --sysroot /usr/local/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu` runs the lints. Existing shared-module/workspace warnings prevent claiming globally warning-free Clippy; no suppressions were added to conceal them.
