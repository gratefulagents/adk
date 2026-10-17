#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
features=builder,host,providers-runtime,execution,tools,mcp,project-state,observability

rustfmt --edition 2024 --check crates/adk/examples/features.rs crates/adk/examples/host_session.rs
cargo clippy --offline --locked -p adk --features "$features" --example features --example host_session -- -D warnings
cargo build --offline --locked -p adk --features "$features" --example features --example host_session
cargo run --offline --locked -p adk --features "$features" --example host_session
cargo run --offline --locked -p adk --features "$features" --example features -- "$@"
