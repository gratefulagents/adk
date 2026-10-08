# User-input helpers

`adk::codec::userinput` (the `compat` feature) provides standalone counterparts
for SDK `pkg/agentsdk/userinput.go` and `ExtractAskUserQuestion`:

| SDK | Rust |
| --- | --- |
| `QuickAction` | `QuickAction` |
| `UserInputPause` | `UserInputPause` |
| `MarshalQuickActions` | `marshal_quick_actions` |
| `ExtractAskUserChoices` | `extract_ask_user_choices` |
| `ExtractPresentPlanData` | `extract_present_plan_data` |
| `ExtractAskUserQuestion` | `extract_ask_user_question` |
| `DetectUserInputPause` | `detect_user_input_pause` |
| `BuildAutoTurnCapPrompt` | `build_auto_turn_cap_prompt` |

These are inspection/formatting helpers, not a CLI, tool executor, or automatic
host pause policy. A host decides whether to pause after examining the result.
They require no platform dependency, runtime, provider, or credentials.

```rust
use adk::codec::userinput::{extract_present_plan_data, marshal_quick_actions};

let (summary, actions) = extract_present_plan_data(
    br#"{"summary":"Review changes","actions":[{"id":"approve","label":"Approve","mode":"build"}]}"#,
);
assert_eq!(summary, "Review changes");
assert_eq!(actions.unwrap(), br#"[{"id":"approve","label":"Approve"}]"#);
assert_eq!(marshal_quick_actions(None), b"null");
assert_eq!(marshal_quick_actions(Some(&[])), b"[]");
```

Actions use SDK JSON spelling, field order, HTML escaping and omission of empty
style. Plan extraction removes unknown fields (including legacy `mode`). An
absent action document (`None`) differs from a present JSON `null` (`Some(b"null")`).
Only the first matching tool-call item is inspected. Raw input bytes are retained
for the question fallback when using `RawJson::Encoded`; use that variant rather
than a parsed JSON value when original whitespace/key order matters.

The independent oracle covers 51 raw inputs and 306 pause observations, including
null array elements, invalid field types, structured question options, fallback
text, legacy action filtering and negative turn caps. It executes the pinned SDK
in a disposable archive:

```sh
python3 scripts/userinput-reference/run.py --check
cargo test --locked -p adk-codec --test userinput
```

Case-insensitive Go field matching (including Unicode simple-fold equivalents)
and repeated-key null/scalar/slice behavior have independent fixtures. Repeated nested arrays preserve Go backing-element reuse, including shortening
and later extending an array. Invalid UTF-8 and unpaired-surrogate replacement
are covered. Rust strings cannot retain invalid UTF-8 bytes: the unrecognized
question fallback uses the SDK JSON-visible replacement text, rather than an
invalid-byte Go string. Serde's nesting limit still applies; arbitrary-depth
Go JSON decoder parity is not claimed. Malformed raw JSON can
be supplied to extraction helpers but not constructed as `RawJson::Encoded`.
The Go warning-log side effect of unrecognized questions is not reproduced.

Source: [SDK at 1dc92b73900fac74dc357a938e4b5eee6392b418](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/userinput.go),
GPL-3.0-only. The oracle records source and harness hashes and the executed Go
version in `fixtures/userinput/observations.json`. Repository provenance and
license notices continue to apply.
