use adk_core::{AccessMode, Tool};
use adk_tools::{BuildError, Config, Features, LegacyFeatures, Registry, capabilities, select};
use std::{collections::BTreeSet, sync::Arc};

fn strict(features: &[&str], access: AccessMode) -> Config {
    Config {
        features: Features::Strict(features.iter().map(|s| s.to_string()).collect()),
        access,
        ..Default::default()
    }
}

fn names(config: &Config) -> Vec<&str> {
    select(config)
        .unwrap()
        .iter()
        .map(|c| c.name.as_str())
        .collect()
}

#[test]
fn all_features_are_independent_and_names_are_unique_in_each_access_mode() {
    let features: BTreeSet<_> = capabilities().iter().map(|c| c.feature.as_str()).collect();
    for feature in &features {
        for access in [
            AccessMode::ReadOnly,
            AccessMode::WorkspaceWrite,
            AccessMode::FullAccess,
        ] {
            let config = Config {
                git_remote_writes: true,
                allow_private_network_urls: true,
                ..strict(&[feature], access)
            };
            let selected = select(&config).unwrap();
            assert!(selected.iter().all(|c| c.feature == **feature));
            assert_eq!(
                selected
                    .iter()
                    .map(|c| &c.name)
                    .collect::<BTreeSet<_>>()
                    .len(),
                selected.len()
            );
        }
    }
    assert!(names(&Config::default()).is_empty());
    let config = strict(&["Browesr"], AccessMode::FullAccess);
    assert!(matches!(
        select(&config),
        Err(BuildError::UnknownFeature(_))
    ));
}

#[test]
fn baseline_legacy_and_strict_modes_do_not_enable_opt_in_families() {
    let config = Config {
        features: Features::Legacy(LegacyFeatures {
            enable_tools: true,
            ..Default::default()
        }),
        access: AccessMode::WorkspaceWrite,
        ..Default::default()
    };
    let runtime: Vec<_> = select(&config)
        .unwrap()
        .into_iter()
        .filter(|c| c.classification == "runtime-built-in")
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        runtime,
        [
            "ApplyPatch",
            "AskUserQuestion",
            "Bash",
            "Delete",
            "Edit",
            "LSP",
            "Move",
            "WebFetch",
            "Write",
            "finish",
            "glob",
            "grep",
            "list_files",
            "present_plan",
            "read_file"
        ]
    );
    let mut config = config;
    config.features = Features::Legacy(LegacyFeatures {
        enable_subagents: true,
        ..Default::default()
    });
    let runtime: Vec<_> = select(&config)
        .unwrap()
        .into_iter()
        .filter(|c| c.classification == "runtime-built-in")
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(runtime, ["AskUserQuestion", "finish", "present_plan"]);
}

#[test]
fn browser_and_web_network_gates_are_distinct() {
    let mut config = strict(&["Browser", "WebFetch"], AccessMode::ReadOnly);
    assert_eq!(names(&config), ["WebFetch"]);
    config.allow_private_network_urls = true;
    assert_eq!(names(&config), ["Browser", "WebFetch"]);
    let browser = select(&config).unwrap()[0];
    assert!(browser.read_only);
    assert_eq!(
        browser.definition.as_ref().unwrap().input_schema.as_value()["properties"]["action"]["enum"],
        serde_json::json!(["navigate", "get_text"])
    );
    config.access = AccessMode::WorkspaceWrite;
    assert!(!select(&config).unwrap()[0].read_only);
}

#[test]
fn terminal_remote_write_and_async_shell_gates_cannot_be_allowlisted_away() {
    let mut config = strict(
        &["InteractiveTerminal", "AsyncShell", "GitHubPullRequest"],
        AccessMode::ReadOnly,
    );
    config.allowed_mutating_tools.extend([
        "Terminal".into(),
        "BashStart".into(),
        "BashKill".into(),
        "create_pull_request".into(),
    ]);
    assert!(names(&config).is_empty());
    config.access = AccessMode::WorkspaceWrite;
    assert_eq!(names(&config), ["BashKill", "BashPoll", "BashStart"]);
    config.git_remote_writes = true;
    assert_eq!(
        names(&config),
        ["BashKill", "BashPoll", "BashStart", "create_pull_request"]
    );
    config.access = AccessMode::FullAccess;
    assert_eq!(
        names(&config),
        [
            "BashKill",
            "BashPoll",
            "BashStart",
            "Terminal",
            "create_pull_request"
        ]
    );
    config.allowed_names = Some(["BashPoll".into()].into());
    assert_eq!(names(&config), ["BashPoll"]);
}

#[test]
fn missing_implementations_are_reported_at_construction_not_tool_execution() {
    let config = strict(&["LSP", "Vision", "Think"], AccessMode::ReadOnly);
    assert!(
        matches!(Registry::build(&config, []), Err(BuildError::Unavailable(names)) if names == ["AnalyzeImage", "LSP"])
    );
    let empty = Registry::build(&Config::default(), []).unwrap();
    assert_eq!(empty.names().count(), 0);
}

#[test]
fn duplicates_are_not_silently_replaced() {
    let config = strict(&["Think"], AccessMode::ReadOnly);
    let registry = Registry::build(&config, []).unwrap();
    let tool = registry.get("think").unwrap().clone();
    assert!(
        matches!(Registry::build(&config, [tool.clone(), tool]), Err(BuildError::Duplicate(name)) if name == "think")
    );
}

#[test]
fn state_definitions_match_every_static_baseline_contract() {
    let definitions: Vec<adk_core::ToolDefinition> = serde_json::from_str(include_str!(
        "../../adk-project-state/src/tool-definitions.json"
    ))
    .unwrap();
    assert_eq!(definitions.len(), 15);
    for definition in definitions {
        let capability = capabilities()
            .iter()
            .find(|c| c.name == definition.name)
            .unwrap();
        assert_eq!(capability.definition.as_ref(), Some(&definition));
    }
}

struct Altered {
    tool: Arc<dyn Tool>,
    definition: adk_core::ToolDefinition,
}
impl Tool for Altered {
    fn definition(&self) -> &adk_core::ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        ctx: &'a adk_core::ToolContext,
        call: adk_core::ToolCall,
    ) -> adk_core::BoxFuture<'a, Result<adk_core::ToolOutput, adk_core::Error>> {
        self.tool.execute(ctx, call)
    }
}

#[test]
fn supplied_contract_drift_is_rejected() {
    let config = strict(&["Think"], AccessMode::ReadOnly);
    let registry = Registry::build(&config, []).unwrap();
    let tool = registry.get("think").unwrap().clone();
    let mut definition = tool.definition().clone();
    definition.description.push('!');
    let tool = Arc::new(Altered { tool, definition }) as Arc<dyn Tool>;
    assert!(
        matches!(Registry::build(&config, [tool]), Err(BuildError::Contract(name)) if name == "think")
    );
}
