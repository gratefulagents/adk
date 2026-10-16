#!/usr/bin/env bash
# Regenerate only synthetic provider fixtures; never invokes a live provider.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
sdk=${1:?usage: bash scripts/providers/generate.sh /path/to/pinned/sdk}
sdk=$(cd "$sdk" && pwd)
mode=${2:-generate}
[[ "$mode" == generate || "$mode" == --check ]] || { echo 'Expected --check or no second argument' >&2; exit 1; }
expected=1dc92b73900fac74dc357a938e4b5eee6392b418
[[ $(git -C "$sdk" rev-parse HEAD) == "$expected" ]] || { echo 'SDK revision mismatch' >&2; exit 1; }
probe="$sdk/internal/openai/zz_adk_continuation_probe_test.go"
[[ ! -e "$probe" ]] || { echo 'Probe path already exists' >&2; exit 1; }
[[ -z $(git -C "$sdk" status --porcelain --untracked-files=no) ]] || { echo 'SDK tracked sources must be clean' >&2; exit 1; }
output=$(mktemp -d)
trap 'rm -f "$probe" "$output/continuation.json"; rmdir "$output"' EXIT
cp "$root/scripts/providers/continuation_probe.go.txt" "$probe"
cd "$sdk"
ADK_PROVIDER_FIXTURE_DIR="$output" go test ./internal/openai -run '^TestADKContinuationGolden$' -count=1
if [[ "$mode" == --check ]]; then
    cmp "$output/continuation.json" "$root/fixtures/providers/continuation.json"
    echo 'Pinned provider continuation observations reproduced exactly'
else
    cp "$output/continuation.json" "$root/fixtures/providers/continuation.json"
fi
