#[cfg(feature = "runtime")]
#[path = "specialists/catalog.rs"]
mod catalog;

#[cfg(feature = "runtime")]
#[path = "specialists/delegation.rs"]
mod delegation;

use adk::{core::*, specialists::*};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct TestTool {
    definition: ToolDefinition,
    adapted: Option<Arc<dyn Tool>>,
    adapter_calls: AtomicUsize,
}

impl TestTool {
    fn new(name: &str, read_only: bool, adapted: Option<Arc<dyn Tool>>) -> Arc<Self> {
        Arc::new(Self {
            definition: ToolDefinition {
                name: name.into(),
                description: String::new(),
                input_schema: true.into(),
                read_only,
                requires_approval: false,
            },
            adapted,
            adapter_calls: AtomicUsize::new(0),
        })
    }
}

impl Tool for TestTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
        assert_eq!(access, AccessMode::ReadOnly);
        self.adapter_calls.fetch_add(1, Ordering::SeqCst);
        self.adapted.clone()
    }

    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        panic!("tool-list helpers must not execute tools")
    }
}

fn assert_same_handles(actual: &[Arc<dyn Tool>], expected: &[Arc<dyn Tool>]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(Arc::ptr_eq(actual, expected));
    }
}

#[test]
fn normalization_matches_sdk_two_tiers() {
    for value in [
        "",
        "full",
        "write",
        "workspace-write",
        "workspace_write",
        "execution",
        "danger-full-access",
        "danger_full_access",
        " FULL ",
        "WrItE",
        "WRİTE",
        "worKspace-wrİte",
        "EXECUTİON",
        "DANGER_FULL_ACCESS",
    ] {
        assert_eq!(
            normalize_tool_access(value),
            ToolAccessLevel::Full,
            "{value:?}"
        );
    }
    for value in [
        "read-only",
        "read_only",
        "Readonly",
        "analysis",
        "unknown",
        "ful",
        "full_access",
        "workspace write",
        "wri\u{0307}te",
        "worſpace-write",
        "\u{200b}full",
        "full\u{feff}",
        "\u{001c}",
        "\u{180e}",
        "full\0",
        "full\nwrite",
    ] {
        assert_eq!(
            normalize_tool_access(value),
            ToolAccessLevel::ReadOnly,
            "{value:?}"
        );
    }
    let whitespace = "\t\n\u{000b}\u{000c}\r \u{0085}\u{00a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";
    for c in whitespace.chars() {
        assert_eq!(normalize_tool_access(&c.to_string()), ToolAccessLevel::Full);
        assert_eq!(
            normalize_tool_access(&format!("{c}FULL{c}")),
            ToolAccessLevel::Full
        );
    }
}

#[test]
fn full_access_preserves_handles_duplicates_and_never_calls_adapters() {
    let replacement = TestTool::new("renamed", true, None);
    let source = TestTool::new("source", false, Some(replacement.clone()));
    let reader = TestTool::new("reader", true, None);
    let tools: Vec<Arc<dyn Tool>> = vec![source.clone(), reader.clone(), source.clone()];
    for access in ["", "full", "WRITE", "workspace_write", "execution"] {
        let mut selected = filter_tools_by_access(&tools, access);
        assert_same_handles(&selected, &tools);
        selected.clear();
        assert_eq!(tools.len(), 3);
    }
    assert_eq!(source.adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(reader.adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(replacement.adapter_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn read_only_adapts_first_allows_renaming_and_filters_the_result() {
    let renamed = TestTool::new("renamed", true, None);
    let mutating = TestTool::new("mutating", false, None);
    let source = TestTool::new("source", false, Some(renamed.clone()));
    let unsafe_adapter = TestTool::new("originally-safe", true, Some(mutating.clone()));
    let reader = TestTool::new("reader", true, None);
    let writer = TestTool::new("writer", false, None);
    let tools: Vec<Arc<dyn Tool>> = vec![
        source.clone(),
        unsafe_adapter.clone(),
        reader.clone(),
        writer.clone(),
        source.clone(),
    ];
    let before = tools.clone();
    let definitions: Vec<_> = tools.iter().map(|tool| tool.definition().clone()).collect();
    for access in ["read-only", "read_only", "Readonly", "analysis", "typo"] {
        let selected = filter_tools_by_access(&tools, access);
        assert_same_handles(
            &selected,
            &[renamed.clone(), reader.clone(), renamed.clone()],
        );
        assert_same_handles(&tools, &before);
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.definition().clone())
                .collect::<Vec<_>>(),
            definitions
        );
    }
    assert_eq!(source.adapter_calls.load(Ordering::SeqCst), 10);
    for tool in [&unsafe_adapter, &reader, &writer] {
        assert_eq!(tool.adapter_calls.load(Ordering::SeqCst), 5);
    }
    for tool in [&renamed, &mutating] {
        assert_eq!(tool.adapter_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn default_signals_are_exact_and_each_map_is_independent() {
    let expected = BTreeMap::from([
        ("finish".into(), true),
        ("present_plan".into(), true),
        ("AskUserQuestion".into(), true),
    ]);
    let mut changed = default_signal_tool_names();
    assert_eq!(changed, expected);
    changed.insert("finish".into(), false);
    changed.insert("custom".into(), true);
    assert_eq!(default_signal_tool_names(), expected);
    let tools: Vec<Arc<dyn Tool>> = [
        "finish",
        "Finish",
        "present_plan",
        "AskUserQuestion",
        "askuserquestion",
        "present_Plan",
        " finish",
        "finish ",
        "ordinary",
        "ordinary",
    ]
    .into_iter()
    .map(|name| TestTool::new(name, false, None) as Arc<dyn Tool>)
    .collect();
    let before = tools.clone();
    assert_same_handles(
        &strip_signal_tools(&tools),
        &[
            tools[1].clone(),
            tools[4].clone(),
            tools[5].clone(),
            tools[6].clone(),
            tools[7].clone(),
            tools[8].clone(),
            tools[9].clone(),
        ],
    );
    assert_same_handles(&tools, &before);
}

#[test]
fn custom_signals_only_strip_true_entries_without_adaptation() {
    let finish = TestTool::new("finish", false, None);
    let custom = TestTool::new("custom", true, Some(finish.clone()));
    let missing = TestTool::new("missing", true, None);
    let tools: Vec<Arc<dyn Tool>> = vec![
        finish.clone(),
        custom.clone(),
        missing.clone(),
        finish.clone(),
    ];
    let before = tools.clone();
    let signals = BTreeMap::from([("finish".into(), false), ("custom".into(), true)]);
    let signals_before = signals.clone();
    assert_same_handles(
        &strip_named_signal_tools(&tools, &signals),
        &[finish.clone(), missing.clone(), finish.clone()],
    );
    assert_same_handles(&strip_named_signal_tools(&tools, &BTreeMap::new()), &tools);
    assert_same_handles(&tools, &before);
    assert_eq!(signals, signals_before);
    for tool in [&finish, &custom, &missing] {
        assert_eq!(tool.adapter_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn empty_results_are_empty_vectors() {
    for access in ["", "read-only", "typo"] {
        assert!(filter_tools_by_access(&[], access).is_empty());
    }
    assert!(strip_signal_tools(&[]).is_empty());
    assert!(strip_named_signal_tools(&[], &BTreeMap::new()).is_empty());
    assert!(
        filter_tools_by_access(&[TestTool::new("writer", false, None)], "read-only").is_empty()
    );
    assert!(strip_signal_tools(&[TestTool::new("finish", true, None)]).is_empty());
}

#[cfg(feature = "runtime")]
mod oracle {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::Mutex;

    struct OracleTool {
        definition: ToolDefinition,
        id: String,
        adapter: bool,
        replacement: Option<Arc<dyn Tool>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Tool for OracleTool {
        fn definition(&self) -> &ToolDefinition {
            &self.definition
        }

        fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
            assert_eq!(access, AccessMode::ReadOnly);
            if self.adapter {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("{}:read-only", self.id));
            }
            self.replacement.clone()
        }

        fn execute<'a>(
            &'a self,
            _: &'a ToolContext,
            _: ToolCall,
        ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
            panic!("fixture comparisons must not execute tools")
        }
    }

    #[test]
    fn pinned_go_tool_list_observations() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../fixtures/specialist-tools/observations.json"
        ))
        .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut by_id: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
        for adapter in [false, true] {
            for spec in fixture["specs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|spec| spec["adapter"] == adapter)
            {
                let replacement = spec["replacement"].as_str().unwrap();
                let tool = OracleTool {
                    definition: TestTool::new(
                        spec["name"].as_str().unwrap(),
                        spec["readOnly"].as_bool().unwrap(),
                        None,
                    )
                    .definition
                    .clone(),
                    id: spec["id"].as_str().unwrap().into(),
                    adapter,
                    replacement: if replacement.is_empty() {
                        None
                    } else {
                        Some(by_id[replacement].clone())
                    },
                    calls: calls.clone(),
                };
                by_id.insert(tool.id.clone(), Arc::new(tool));
            }
        }
        let handles = |ids: &Value| -> Vec<Arc<dyn Tool>> {
            ids.as_array()
                .unwrap()
                .iter()
                .map(|id| by_id[id.as_str().unwrap()].clone())
                .collect()
        };
        let tools = handles(&fixture["inputIDs"]);
        let before = tools.clone();
        for case in fixture["accessCases"].as_array().unwrap() {
            let access = case["access"].as_str().unwrap();
            let normalized = match normalize_tool_access(access) {
                ToolAccessLevel::Full => "full",
                ToolAccessLevel::ReadOnly => "read-only",
            };
            assert_eq!(normalized, case["normalized"], "{access:?}");
            assert_eq!(normalized, case["role"], "{access:?}");
            calls.lock().unwrap().clear();
            assert_same_handles(
                &filter_tools_by_access(&tools, access),
                &handles(&case["outputIDs"]),
            );
            assert_eq!(
                json!(*calls.lock().unwrap()),
                case["adapterCalls"],
                "{access:?}"
            );
            assert_same_handles(&tools, &before);
        }
        calls.lock().unwrap().clear();
        assert_eq!(json!(default_signal_tool_names()), fixture["defaultNames"]);
        assert_same_handles(
            &strip_signal_tools(&tools),
            &handles(&fixture["defaultStrip"]),
        );
        for case in fixture["stripCases"].as_array().unwrap() {
            let names: BTreeMap<String, bool> = if case["names"].is_null() {
                BTreeMap::new()
            } else {
                serde_json::from_value(case["names"].clone()).unwrap()
            };
            assert_same_handles(
                &strip_named_signal_tools(&tools, &names),
                &handles(&case["outputIDs"]),
            );
            assert_same_handles(&tools, &before);
        }
        assert!(calls.lock().unwrap().is_empty());
    }
}
