#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
features=builder,providers-runtime,execution,tools,mcp,project-state,observability

rustfmt --edition 2024 --check crates/adk/examples/features.rs
cargo clippy --offline --locked -p adk --features "$features" --example features -- -D warnings
cargo build --offline --locked -p adk --features "$features" --example features
cargo run --offline --locked -p adk --features "$features" --example features -- "$@"
