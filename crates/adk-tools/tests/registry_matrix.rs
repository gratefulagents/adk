use adk_core::AccessMode;
use adk_tools::{Config, Features, LegacyFeatures, select};
use serde::Deserialize;
#[derive(Deserialize, Debug)]
struct Case {
    features: Option<Vec<String>>,
    legacy: u8,
    access: String,
    remote: bool,
    private: bool,
    allowed: Vec<String>,
    names: Vec<String>,
}
#[test]
fn actual_sdk_runtime_feature_and_access_matrix() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../../../fixtures/tools/registry-matrix.json")).unwrap();
    for (index, case) in cases.into_iter().enumerate() {
        let features = match &case.features {
            Some(features) => Features::Strict(features.iter().cloned().collect()),
            None => Features::Legacy(LegacyFeatures {
                enable_tools: case.legacy & 1 != 0,
                enable_subagents: case.legacy & 2 != 0,
                disable_default_tools: case.legacy & 4 != 0,
                disable_signal_tools: case.legacy & 8 != 0,
                disable_web_tools: case.legacy & 16 != 0,
                enable_async_shell: case.legacy & 32 != 0,
                enable_project_state: case.legacy & 64 != 0,
            }),
        };
        let selected = select(&Config {
            features,
            access: match case.access.as_str() {
                "read_only" => AccessMode::ReadOnly,
                "workspace_write" => AccessMode::WorkspaceWrite,
                "full_access" => AccessMode::FullAccess,
                _ => panic!("access"),
            },
            git_remote_writes: case.remote,
            allow_private_network_urls: case.private,
            allowed_mutating_tools: case.allowed.iter().cloned().collect(),
            ..Default::default()
        })
        .unwrap();
        let mut names = selected
            .into_iter()
            .filter(|c| c.classification != "host-only")
            .map(|c| c.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, case.names, "SDK case {index}: {case:?}");
    }
}
