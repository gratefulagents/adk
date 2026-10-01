#![cfg(feature = "host")]

//! Differential integration test against the independent, pinned SDK observation.
//! Expected values are never synthesized from Rust or a second Go implementation.
//! Each query is compared independently; failures are collected so none are hidden
//! by an earlier mismatch. HOME is only changed on a child Command before startup.

use adk::{
    builder::{ModeSpec, RoleSpec},
    core::{Context, Error, ErrorCategory},
    host::{ConfigSource, PermissionMode, build_mode_directive, fileconfig::FileConfigSource},
    runtime::CancellationToken,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command, sync::Arc};

const TEST: &str = "matches_every_unexcluded_pinned_go_query";
const CHILD_CASE: &str = "ADK_FILECONFIG_ORACLE_CASE";
const CHILD_ROOT: &str = "ADK_FILECONFIG_ORACLE_ROOT";
const PIN: &str = "1dc92b73900fac74dc357a938e4b5eee6392b418";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u32,
    sdk_revision: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    source_only: Vec<String>,
    input: Input,
    output: Vec<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    files: BTreeMap<String, String>,
    active_mode: String,
    root_style: String,
    home_unset: bool,
    queries: Vec<Query>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    operation: String,
    lookup: String,
    cancelled: bool,
    template: Value,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../fixtures/fileconfig/sdk-fileconfig.json"
    ))
    .unwrap()
}

// This is a DTO projection, not a loader. No trimming, sorting, deduplication,
// access normalization, or expected-dependent changes are permitted here.
fn mode_value(mode: &ModeSpec) -> Value {
    let routing = mode.model_routing.as_ref().map(|routing| {
        assert!(routing.settings.is_empty(), "SDK has no routing settings field");
        let overrides: serde_json::Map<String, Value> = routing
            .role_overrides
            .iter()
            .map(|(name, role)| {
                assert!(role.settings.is_empty(), "SDK has no role routing settings field");
                (name.clone(), json!({
                    "Model": role.model,
                    // Go cloneTrimmedStrings always allocates, even for absent input.
                    "FallbackModels": role.fallback_models.as_deref().unwrap_or_default(),
                    "ReasoningLevel": role.reasoning_level,
                    "TextVerbosity": role.text_verbosity,
                }))
            })
            .collect();
        json!({
            "DefaultModel": routing.default_model,
            "FallbackModels": routing.fallback_models.as_deref().unwrap_or_default(),
            "ReasoningLevel": routing.reasoning_level,
            "TextVerbosity": routing.text_verbosity,
            // Go modelRoutingSpec.toSDK explicitly nils an empty override map.
            "RoleOverrides": if overrides.is_empty() { Value::Null } else { Value::Object(overrides) },
        })
    });
    let constraints = mode.constraints.as_ref().map(|limits| {
        json!({
            "MaxTurns": limits.max_turns.map_or(0, |n| n.get()),
            "SubAgentMaxTurns": limits.subagent_max_turns.map_or(0, |n| n.get()),
            "MaxConcurrentSubAgents": limits.max_concurrent_subagents.map_or(0, |n| n.get()),
            "MaxRetries": limits.max_retries.unwrap_or(0),
            "MaxRuntimeMinutes": limits.max_runtime_minutes.map_or(0, |n| n.get()),
        })
    });
    json!({
        "Name": mode.name, "Version": mode.version,
        "DisplayName": mode.display_name, "Description": mode.description,
        "Category": mode.category, "Autonomous": mode.autonomous,
        "ToolAccess": mode.tool_access, "Instructions": mode.instructions,
        "ModelRouting": routing, "Constraints": constraints,
    })
}

fn role_value(role: &RoleSpec) -> Value {
    json!({
        "Name": role.name, "Description": role.description,
        "Instructions": role.instructions, "ToolAccess": role.tool_access,
        "ModelOverride": role.model_override,
        // SDK parseRoleFile never assigns this slice; Rust RoleSpec has no field.
        "FallbackModels": null,
    })
}

fn formatter_input(template: &Value) -> Option<ModeSpec> {
    if template.is_null() {
        return None;
    }
    let text = |key: &str| template[key].as_str().unwrap().to_owned();
    let mode = ModeSpec {
        name: text("Name"),
        version: text("Version"),
        display_name: text("DisplayName"),
        description: text("Description"),
        category: text("Category"),
        autonomous: template["Autonomous"].as_bool().unwrap(),
        tool_access: text("ToolAccess"),
        instructions: text("Instructions"),
        ..Default::default()
    };
    // All pinned direct formatter inputs have null routing/constraints. Refuse
    // future non-null or extra fields rather than silently dropping input data.
    assert_eq!(
        mode_value(&mode),
        *template,
        "formatter input must be lossless"
    );
    Some(mode)
}

fn observation(result: Result<Value, Error>, error_result: Value) -> Value {
    match result {
        Ok(value) => json!({"result": value, "error": null}),
        Err(error) => json!({
            // Rust Result carries no partial value. SDK methods in this fixture
            // return nil for object/collection errors and "" for string errors.
            "result": error_result,
            "error": {
                "category": if error.info.category == ErrorCategory::Cancelled {
                    "cancelled"
                } else { "fileconfig" },
                "message": error.to_string(),
            },
        }),
    }
}

async fn execute(case: &Case, root: &Path) -> Vec<Value> {
    let root_text = root.to_str().unwrap();
    let root_arg = match case.input.root_style.as_str() {
        "" | "literal" => root_text.to_owned(),
        "padded" => format!(" \t{root_text} \n"),
        "default" => " \t".into(),
        "tilde" => "~".into(),
        "tilde-child" => "~/config".into(),
        other => panic!("unknown root style {other}"),
    };
    // Pass the actual string root, including whitespace. In particular, do not
    // hide constructor differences by always substituting the temporary root.
    let source =
        FileConfigSource::from_config_root(&root_arg).with_active_mode(&case.input.active_mode);
    let mut observations = Vec::new();
    for query in &case.input.queries {
        let cancellation = Arc::new(CancellationToken::new());
        if query.cancelled {
            cancellation.cancel();
        }
        let context = Context {
            run_id: "fileconfig-oracle".into(),
            cancellation,
            deadline: None,
        };
        let result = match query.operation.as_str() {
            "BuiltinModes" => Ok(Value::Array(
                adk::builder::builtin_modes().iter().map(mode_value).collect(),
            )),
            "GuardrailRules" => source.guardrail_rules(&context).await.map(|rules| {
                assert!(rules.is_empty(), "fileconfig must not supply guardrail rules");
                Value::Null
            }),
            "HandoffHistory" => source.handoff_history(&context).await.map(|history| {
                assert_eq!(history, adk::host::RunBatch::default());
                Value::Null
            }),
            "ListModes" => source
                .list_modes(&context)
                .map(|modes| Value::Array(modes.iter().map(mode_value).collect())),
            "GetMode" => source
                .get_mode(&context, &query.lookup)
                .map(|mode| mode_value(&mode)),
            "RoleCatalog" => source.role_catalog(&context).await.map(|roles| {
                // Go LoadRoleCatalog starts with a nil slice, including when
                // the directory exists but contains no roles. Only this result
                // type uses nil for an empty successful Vec.
                if roles.is_empty() {
                    Value::Null
                } else {
                    Value::Array(roles.iter().map(role_value).collect())
                }
            }),
            "PermissionMode" => source.permission_mode(&context).await.map(|permission| {
                json!(match permission {
                    PermissionMode::ReadOnly => "read-only",
                    PermissionMode::WorkspaceWrite => "workspace-write",
                    PermissionMode::DangerFullAccess => "danger-full-access",
                })
            }),
            "ModeSnapshot" => source
                .mode_snapshot(&context)
                .await
                .map(|mode| mode.as_ref().map_or(Value::Null, mode_value)),
            "ModeDirective" => source.mode_directive(&context).await.map(Value::String),
            "BuildModeDirective" => Ok(json!(build_mode_directive(
                formatter_input(&query.template).as_ref()
            ))),
            "dirs" => {
                // The only string substitution matches the oracle's temporary
                // root replacement. Whitespace, separators, etc. remain intact.
                let path = |p: &Path| p.to_str().unwrap().replace(root_text, "<root>");
                Ok(json!({
                    "RootDir": path(source.root()),
                    "ModeDir": path(&source.mode_dir()),
                    "AgentDir": path(&source.agent_dir()),
                    "DefaultRootDir": path(FileConfigSource::default().root()),
                }))
            }
            operation => panic!("unhandled oracle operation {operation}"),
        };
        let error_result = match query.operation.as_str() {
            "PermissionMode" | "ModeDirective" | "BuildModeDirective" => json!(""),
            _ => Value::Null,
        };
        observations.push(observation(result, error_result));
    }
    observations
}

// Report leaf paths while preserving array order and exact null/empty semantics.
fn differences(path: &str, actual: &Value, expected: &Value, out: &mut Vec<String>) {
    if actual == expected {
        return;
    }
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) if a.keys().eq(e.keys()) => {
            for (key, value) in e {
                differences(&format!("{path}.{key}"), &a[key], value, out);
            }
        }
        (Value::Array(a), Value::Array(e)) if a.len() == e.len() => {
            for (index, (actual, expected)) in a.iter().zip(e).enumerate() {
                differences(&format!("{path}[{index}]"), actual, expected, out);
            }
        }
        _ => out.push(format!("{path}: Rust={actual}; Go={expected}")),
    }
}

fn assert_inventory(fixture: &Fixture) {
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.sdk_revision, PIN);
    assert_eq!(fixture.cases.len(), 51);
    assert_eq!(
        fixture
            .cases
            .iter()
            .map(|c| c.input.queries.len())
            .sum::<usize>(),
        190
    );
    let exclusions = BTreeMap::from([
        ("duplicate-declared-mode-names", "duplicate-name-policy"),
        ("duplicate-declared-role-names", "duplicate-name-policy"),
        ("unknown-yaml-fields", "unknown-field-policy"),
        (
            "zero-and-negative-constraints",
            "zero-negative-limit-policy",
        ),
        (
            "role-unclosed-frontmatter",
            "unterminated-frontmatter-policy",
        ),
        ("access- MYSTERY ", "unknown-access-policy"),
        ("dirs-home-unset-default", "home-policy"),
        ("dirs-home-unset-tilde", "home-policy"),
        ("dirs-home-unset-tilde-child", "home-policy"),
    ]);
    let mut actual_exclusions = BTreeMap::new();
    let mut operations = BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    let mut excluded_queries = 0;
    for case in &fixture.cases {
        assert!(names.insert(&case.name), "duplicate fixture case");
        assert_eq!(case.input.queries.len(), case.output.len(), "{}", case.name);
        if !case.source_only.is_empty() {
            assert_eq!(case.source_only.len(), 1);
            actual_exclusions.insert(case.name.as_str(), case.source_only[0].as_str());
            excluded_queries += case.input.queries.len();
        } else {
            for query in &case.input.queries {
                *operations.entry(query.operation.as_str()).or_insert(0) += 1;
            }
        }
    }
    assert_eq!(
        actual_exclusions, exclusions,
        "source-only exclusions require explicit review"
    );
    assert_eq!(excluded_queries, 19);
    assert_eq!(
        operations,
        BTreeMap::from([
            ("BuiltinModes", 2),
            ("GuardrailRules", 2),
            ("HandoffHistory", 2),
            ("ListModes", 13),
            ("GetMode", 53),
            ("RoleCatalog", 21),
            ("PermissionMode", 23),
            ("ModeSnapshot", 17),
            ("ModeDirective", 23),
            ("dirs", 5),
            ("BuildModeDirective", 10),
        ])
    );
}

#[tokio::test]
async fn matches_every_unexcluded_pinned_go_query() {
    let fixture = fixture();
    assert_inventory(&fixture);
    if let Ok(name) = std::env::var(CHILD_CASE) {
        let root = std::env::var_os(CHILD_ROOT).expect("child root");
        let root = Path::new(&root);
        let case = fixture
            .cases
            .iter()
            .find(|case| case.name == name)
            .expect("selected case");
        assert!(
            case.source_only.is_empty(),
            "excluded cases are not parity passes"
        );
        let output = execute(case, root).await;
        fs::write(
            root.join("oracle-observations.json"),
            serde_json::to_vec(&output).unwrap(),
        )
        .unwrap();
        return;
    }

    // current_exe may require unavailable /proc access. The test harness argv[0]
    // is the actual test binary path, and works in this sandbox as well.
    let binary = std::env::args_os().next().expect("test binary argv[0]");
    let mut passed = 0;
    let mut checked = 0;
    let mut checked_cases = 0;
    let mut failures = Vec::new();
    for case in &fixture.cases {
        if !case.source_only.is_empty() {
            eprintln!(
                "EXCLUDED {} ({} queries): {}",
                case.name,
                case.input.queries.len(),
                case.source_only.join(", ")
            );
            continue;
        }
        checked_cases += 1;
        let root = tempfile::tempdir().unwrap();
        // As in Go execute(): no precreated modes/agents/home directories.
        for (path, text) in &case.input.files {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let mut command = Command::new(&binary);
        command
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD_CASE, &case.name)
            .env(CHILD_ROOT, root.path());
        if case.input.home_unset {
            command.env_remove("HOME");
        } else {
            command.env("HOME", root.path().join("home"));
        }
        let child = command.output().expect("start isolated oracle case");
        if !child.status.success() {
            failures.push(format!(
                "{}: child failed ({}): {}\n{}",
                case.name,
                child.status,
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            ));
            continue;
        }
        let actual: Vec<Value> = serde_json::from_slice(
            &fs::read(root.path().join("oracle-observations.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            actual.len(),
            case.output.len(),
            "{}: child must execute every query",
            case.name
        );
        for (index, ((query, actual), expected)) in case
            .input
            .queries
            .iter()
            .zip(&actual)
            .zip(&case.output)
            .enumerate()
        {
            checked += 1;
            let mut diffs = Vec::new();
            differences("result", &actual["result"], &expected["result"], &mut diffs);
            // Error presence and cancellation classification are API behavior;
            // parser and OS message spelling is deliberately not compared.
            differences(
                "error.present",
                &json!(!actual["error"].is_null()),
                &json!(!expected["error"].is_null()),
                &mut diffs,
            );
            differences(
                "error.category",
                &actual["error"]["category"],
                &expected["error"]["category"],
                &mut diffs,
            );
            if diffs.is_empty() {
                passed += 1;
            } else {
                failures.push(format!(
                    "{} query[{index}] {} lookup={:?} cancelled={}:\n  {}",
                    case.name,
                    query.operation,
                    query.lookup,
                    query.cancelled,
                    diffs.join("\n  ")
                ));
            }
        }
    }
    eprintln!(
        "Go fileconfig oracle: {passed}/{checked} queries passed; {checked_cases}/42 cases executed; 9 cases / 19 queries EXCLUDED (not passes); {} mismatches/infrastructure failures",
        failures.len()
    );
    assert_eq!(checked_cases, 42);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(checked, 171, "all non-source-only queries must execute");
}
