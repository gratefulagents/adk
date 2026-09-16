#!/usr/bin/env bash
# Credential-free dependency capability probe; isolated from the workspace graph.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
probe=$(mktemp -d)
trap 'rm -rf "$probe"' EXIT
mkdir "$probe/src"
cp "$root/scripts/providers/client_probe.rs" "$probe/src/main.rs"
cat > "$probe/Cargo.toml" <<'MANIFEST'
[package]
name = "provider-client-probe"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
async-openai = { version = "=0.42.0", default-features = false, features = ["response-types"] }
serde_json = "1"
MANIFEST
cargo run --manifest-path "$probe/Cargo.toml"
