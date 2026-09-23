#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Run from the repository root. Set CARGO_NET_OFFLINE=true after cargo fetch.
set -eu
cargo test --locked -p adk --no-default-features --all-targets
for feature in compat runtime providers providers-runtime execution durable project-state tools mcp observability otel builder runtime,tools,providers-runtime; do
    cargo test --locked -p adk --no-default-features --features "$feature" --all-targets
done
cargo test --locked -p adk --all-features --all-targets
cargo run --locked --manifest-path fixtures/standalone-consumer/Cargo.toml
python3 scripts/check-purity.py
