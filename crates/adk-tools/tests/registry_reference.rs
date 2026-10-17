//! Runtime oracle for explicitly configured Go-equivalent selection, NOT constructor parity.
//! Rust Strict(empty) intentionally stays empty. Browser is selection-only here;
//! browser construction, Vision directories and duplicate Register are excluded.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use adk_core::{AccessMode, Tool, ToolDefinition};
use adk_tools::{Config, Features, Registry, lsp, select, shell};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
    contracts: BTreeMap<String, ToolDefinition>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    names: Vec<String>,
    tools: Vec<ObservedTool>,
    missing: bool,
    options: BTreeMap<String, bool>,
}
#[derive(Deserialize)]
struct ObservedTool {
    name: String,
    source_type: String,
    contract: String,
}
#[derive(Deserialize)]
struct Input {
    id: String,
    permission: String,
    remote_writes: bool,
    default_constructor: bool,
    signals: bool,
    browser: bool,
    without_web: bool,
    private_urls: bool,
    async_shell: bool,
    think: bool,
    terminal: bool,
    allow_mutating: BTreeSet<String>,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../fixtures/tools/registry-selection.json"
    ))
    .unwrap()
}

#[test]
fn explicit_option_values_match_actual_pinned_registry_state() {
    for case in fixture().cases {
        let config = equivalent(&case.input);
        let Features::Strict(features) = &config.features else {
            unreachable!()
        };
        let actual = BTreeMap::from([
            (
                "signals".into(),
                features.contains("Signals.AskUserQuestion")
                    && features.contains("Signals.PresentPlan"),
            ),
            ("browser".into(), features.contains("Browser")),
            ("without_web".into(), !features.contains("WebFetch")),
            ("private_urls".into(), config.allow_private_network_urls),
            ("async_shell".into(), features.contains("AsyncShell")),
            ("think".into(), features.contains("Think")),
            ("terminal".into(), features.contains("InteractiveTerminal")),
        ]);
        assert_eq!(actual, case.options, "{}", case.input.id);
    }
}

// This mapping is independent of the observed names and native capability catalog.
// Go NewRegistry always enables these defaults; native defaults do not.
fn equivalent(input: &Input) -> Config {
    let mut features: BTreeSet<String> = [
        "ListFiles",
        "ReadFile",
        "Glob",
        "Grep",
        "LSP",
        "Bash",
        "Write",
        "Edit",
        "ApplyPatch",
        "Move",
        "Delete",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for (enabled, feature) in [
        (!input.without_web, "WebFetch"),
        (input.signals, "Signals.AskUserQuestion"),
        (input.signals, "Signals.PresentPlan"),
        (input.browser, "Browser"),
        (input.async_shell, "AsyncShell"),
        (input.think, "Think"),
        (input.terminal, "InteractiveTerminal"),
    ] {
        if enabled {
            features.insert(feature.into());
        }
    }
    Config {
        features: Features::Strict(features),
        access: match input.permission.as_str() {
            "read-only" => AccessMode::ReadOnly,
            "workspace-write" => AccessMode::WorkspaceWrite,
            "danger-full-access" => AccessMode::FullAccess,
            other => panic!("unknown oracle permission {other}"),
        },
        git_remote_writes: input.remote_writes,
        allow_private_network_urls: input.private_urls,
        allowed_mutating_tools: input.allow_mutating.clone(),
        ..Default::default()
    }
}

#[test]
fn oracle_contains_default_and_thirteen_independent_profiles_across_all_six_gates() {
    let fixture = fixture();
    assert_eq!(fixture.cases.len(), 79);
    let ids: BTreeSet<_> = fixture.cases.iter().map(|c| c.input.id.as_str()).collect();
    assert_eq!(ids.len(), 79);
    assert_eq!(
        fixture
            .cases
            .iter()
            .filter(|c| c.input.default_constructor)
            .count(),
        1
    );
    for profile in [
        "baseline",
        "signals",
        "browser_public",
        "browser_private",
        "without_web",
        "private_urls",
        "async_shell",
        "think",
        "terminal",
        "all_options",
        "all_without_web",
        "allowlist_baseline",
        "allowlist_options",
    ] {
        for mode in ["read-only", "workspace-write", "danger-full-access"] {
            for remote in [false, true] {
                assert!(ids.contains(format!("{profile}/{mode}/remote_{remote}").as_str()));
            }
        }
    }
}

#[test]
fn explicit_selection_matches_every_runtime_observed_name_without_exceptions() {
    for case in fixture().cases {
        let names: Vec<_> = select(&equivalent(&case.input))
            .unwrap()
            .into_iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, case.names, "{}", case.input.id);
    }
}

#[test]
fn selected_modes_match_actual_go_constructor_types() {
    for case in fixture().cases {
        let selected = select(&equivalent(&case.input)).unwrap();
        let actual: Vec<_> = selected.iter().map(|c| (&c.name, &c.source_type)).collect();
        let expected: Vec<_> = case
            .tools
            .iter()
            .map(|t| (&t.name, &t.source_type))
            .collect();
        assert_eq!(actual, expected, "{}", case.input.id);
    }
}

#[test]
fn selected_contracts_match_runtime_go_definitions_including_browser_selection() {
    let fixture = fixture();
    for case in &fixture.cases {
        let config = equivalent(&case.input);
        for observed in &case.tools {
            let selected = select(&config)
                .unwrap()
                .into_iter()
                .find(|c| c.name == observed.name)
                .unwrap();
            let definition = selected
                .definition
                .clone()
                .or_else(|| {
                    shell::definition(&selected.name, config.access, &shell::Limits::default())
                })
                .unwrap();
            assert_eq!(
                definition, fixture.contracts[&observed.contract],
                "{}: {}",
                case.input.id, observed.name
            );
        }
    }
}

#[test]
fn native_safety_default_remains_empty_instead_of_claiming_constructor_parity() {
    assert_eq!(
        Registry::build(&Config::default(), [])
            .unwrap()
            .names()
            .count(),
        0
    );
    assert!(
        !fixture()
            .cases
            .iter()
            .find(|c| c.input.default_constructor)
            .unwrap()
            .names
            .is_empty()
    );
}

// Real shell/LSP implementations, not catalog-shaped stubs. No tools execute.
// Browser resource construction is deliberately outside this acceptance slice.
fn build(
    case: &Case,
) -> (
    tempfile::TempDir,
    shell::ShellBundle,
    Arc<lsp::LspTool>,
    Registry,
) {
    let root = tempfile::tempdir().unwrap();
    let mut sandbox = adk_sandbox::Config::new(root.path());
    sandbox.backend = adk_sandbox::Backend::Local;
    let mut config = equivalent(&case.input);
    if let Features::Strict(features) = &mut config.features {
        features.remove("Browser");
    }
    let shell = shell::ShellBundle::new(shell::Config {
        sandbox: sandbox.clone(),
        access: config.access,
        git_remote_writes: config.git_remote_writes,
        environment: Default::default(),
    })
    .unwrap();
    let lsp = lsp::tool(lsp::Config {
        executor: Arc::new(adk_sandbox::Executor::new(sandbox).unwrap()),
        servers: vec![],
        discoverer: None,
    });
    let mut implementations = shell.tools();
    implementations.push(lsp.clone() as Arc<dyn Tool>);
    let registry = Registry::build(&config, implementations).unwrap();
    (root, shell, lsp, registry)
}

#[tokio::test]
async fn built_registry_definitions_match_actual_go_objects_except_excluded_browser_construction() {
    let fixture = fixture();
    for case in &fixture.cases {
        let (_root, shell, lsp, registry) = build(case);
        for observed in case.tools.iter().filter(|t| t.name != "Browser") {
            assert_eq!(
                registry.get(&observed.name).unwrap().definition(),
                &fixture.contracts[&observed.contract],
                "{}: {}",
                case.input.id,
                observed.name
            );
        }
        shell.close().await;
        lsp.close().await.unwrap();
    }
}

#[tokio::test]
async fn built_registry_names_and_tools_are_sorted_and_match_observed_membership() {
    for case in fixture().cases {
        let (_root, shell, lsp, registry) = build(&case);
        let expected: Vec<_> = case
            .names
            .iter()
            .filter(|n| n.as_str() != "Browser")
            .map(String::as_str)
            .collect();
        let names: Vec<_> = registry.names().collect();
        let tools: Vec<_> = registry
            .tools()
            .map(|t| t.definition().name.as_str())
            .collect();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "{}", case.input.id);
        assert_eq!(names, expected, "{}", case.input.id);
        assert_eq!(tools, expected, "{}", case.input.id);
        shell.close().await;
        lsp.close().await.unwrap();
    }
}

#[tokio::test]
async fn get_returns_none_for_missing_name_even_when_allowlisted() {
    for case in fixture().cases {
        let (_root, shell, lsp, registry) = build(&case);
        assert!(case.missing);
        assert!(registry.get("missing_tool").is_none(), "{}", case.input.id);
        shell.close().await;
        lsp.close().await.unwrap();
    }
}
