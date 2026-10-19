# Pinned payload reference

Run `python3 scripts/payload-reference/run.py` to regenerate, then
`python3 scripts/payload-reference/run.py --check` to reproduce byte-for-byte.
The runner archives immutable SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` from `repos/sdk` into a disposable
directory and injects only the oracle test. It never changes the SDK checkout.
Go dependencies must already be available or downloadable. `GOROOT` defaults to
`/usr/local/go`; `GOTOOLCHAIN=local`, `GOWORK=off`, `GOFLAGS=-mod=readonly`, and
`GOARCH=amd64` are fixed. The test asserts `strconv.IntSize == 64`.
`ADK_REFERENCE_TMPDIR`, `GOCACHE`, and `GOMODCACHE` can override temporary/cache
locations. The recorded Go version and source/harness SHA-256 hashes are part of
exact reproducibility; changing the toolchain intentionally changes provenance.

## Stable fixture schema (version 1)

The fixture includes 221 `LLMReasoning` and 153 `LLMCompaction` observations.
These are plain-string analysis records, distinct from the runtime item payloads.
Both are supported by the native sealed byte-codec API; the minimal compat-only
facade runs all 374 comparisons. This does not add whole request/response byte
decoding or infer nested/raw-field compatibility from the leaf records.

`LLMToolCall` and `LLMToolApproval` add 319 decode/encode cases and 14 direct
marshal cases. Their sealed native codecs preserve a single raw `input` field
without normalizing its original bytes. Raw omission, empty versus null values,
invalid UTF-8, duplicate keys, lexical numbers, malformed syntax and depth limits
are independently observed. The facade now checks 693 leaf decode/encode cases.
The leaf portion of the payload fixture contains 2,598 decode cases and 28 direct
snapshot marshal cases. Additional nested-document observations are described below.

`run_item_byte_cases` adds 40 whole run-item decode observations, including
original nested raw bytes and whole-object marshal bytes. `run_item_encode_cases`
adds 20 direct nested raw marshal probes. They distinguish whole-input decode
depth from independently validated raw fragments during marshal. The native
schema-bounded transport uses call-local tokens while the existing object
decoder handles repeated pointers and nulls; it neither traverses arbitrary raw
JSON as typed fields nor uses global state. Whole request/response codecs remain
outside the original bounded run-item slice.

`envelope_byte_cases` now adds 130 request/response decode observations;
`envelope_encode_cases` adds 80 direct marshal probes at eight raw-field locations.
The oracle additionally injects `envelope_test.go` into the disposable archive.
Comparisons cover original raw bytes, whole encoded bytes, typed-string
replacement, duplicate arrays with retained hidden slots, pointer resets,
malformed overwritten values, and both depth boundaries. The native schema
walker now traverses only the declared item/tool arrays and schema fields.
Byte-codec support does not imply automatic trace-writer integration or complete
in-memory slice-representation parity.

Output: `fixtures/payloads/observations.json`.

- `schema_version`: 1.
- `int_size`: 64 (bits).
- `enum_values`: Go constant names mapped to their observed numeric values.
- `provenance`: SDK repository, commit, license, complete archive SHA-256,
  source file hashes, harness hashes, Go version, GOOS, and GOARCH.
- `cases`: ordered records, each containing:
  - `id`: unique stable descriptive identifier.
  - `type`: an observed Go record name or `RunItemType`.
  - `input_hex`: original JSON input bytes, including malformed UTF-8.
  - `decode_ok`, `decode_error`: `json.Unmarshal` result; error is empty on success.
  - `decoded_fields`: fields keyed by JSON tag, including zero values and fields
    normally omitted when marshaling. Images recursively use this same shape;
    nil images are null and nonnil empty images are []. RawMessage `input` is
    deliberately excluded. For RunItemType this is `{"value": <integer>}`.
    Partial decoded state is captured even after a decoding failure.
  - `raw_input`: null for types without a RawMessage field; otherwise
    `{"present": <bool>, "hex": <original decoded RawMessage bytes>}`.
    `present` means the resulting Go RawMessage is nonnil, not merely that an
    input key occurred. Missing input is false/empty; explicit null is true/6e756c6c.
  - `marshal_ok`, `marshal_error`, `marshal_hex`: independent `json.Marshal`
    result on the decoded value, even when decoding failed. Error is empty on
    success. Hex is empty when marshaling fails.

Never parse/re-encode `raw_input.hex` when comparing fixtures: duplicate keys,
number spelling, whitespace, escape spelling, invalid UTF-8, and lone surrogate
escapes are intentionally preserved there. `marshal_hex` is separate because Go
compacts RawMessage whitespace and applies HTML escaping on marshal.

## Coverage

All nine structs: zero, populated, each field missing/null/zero/populated,
wrong JSON types, folded keys and duplicates (including invalid earlier values),
unknown fields, null/wrong roots, malformed JSON, Unicode replacement/escaping,
and depth boundary. Image arrays cover nil/empty, invalid elements, repeated
keys, reused slots, shrink/regrow, null elements, and []/null resets. Both raw
input types cover lexical numbers/objects/strings, missing/null/repeated keys,
huge numbers, surrogate escapes, malformed UTF-8, and nested depth limits.
RunItemType covers 0..7, unknown values, int64 extrema, overflow, -0,
fraction/exponent forms, null, strings, wrong roots, and malformed JSON.
The harness also constructs public RawMessage values directly and checks that
both raw-bearing payloads marshal depth 10,000 successfully and reject 10,001;
this is separate from the containing object's decode depth limit.

Scope excludes RunItem and Agent. The harness imports the actual pinned SDK
payload types; it does not duplicate their definitions or expected behavior.
