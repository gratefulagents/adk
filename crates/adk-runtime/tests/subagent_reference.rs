//! Actual pinned Go fixtures; regenerate with scripts/replay/subagent_export.py.
use adk_core::*;
use adk_runtime::{subagent::*, *};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/go-subagents.json")).unwrap()
}
fn context() -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "reference".into(),
            cancellation: Arc::new(CancellationToken::new()),
            deadline: None,
        },
        policy: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        work_dir: ".".into(),
        idempotency_key: None,
    }
}
#[derive(Default)]
struct TestHost;
impl Host for TestHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
struct MutationTool(ToolDefinition);
impl Tool for MutationTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async { panic!("the controlled model never invokes mutate") })
    }
}
struct ControlledModel {
    name: String,
    gate: Option<Semaphore>,
    started: Semaphore,
    calls: Mutex<Vec<Value>>,
}
impl Model for ControlledModel {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let input = serde_json::to_string(&request.input).unwrap();
            let mut tools: Vec<_> = request.tools.iter().map(|tool| &tool.name).collect();
            tools.sort();
            self.calls.lock().unwrap().push(json!({
                "tools":tools,
                "dependency_evidence":input.contains("evidence:"),
                "parent_secret":input.contains("parent-secret"),
            }));
            self.started.add_permits(1);
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            if self.name == "fail" {
                return Err(Error::new(ErrorCategory::Provider, "controlled failure"));
            }
            Ok(ModelResponse {
                snapshot_raw: None,
                raw: None,
                items: vec![RunItem::Message {
                    message: Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: format!("evidence:{}", self.name),
                        }],
                    },
                }],
                usage: Usage::default(),
                end_turn: None,
                response_id: None,
                metadata: Default::default(),
            })
        })
    }
}
struct Harness {
    owner: Scheduler,
    tools: Vec<Arc<dyn Tool>>,
    models: BTreeMap<String, Arc<ControlledModel>>,
}
fn harness(gates: &Value) -> Harness {
    let mut models = BTreeMap::new();
    let mut runners = std::collections::HashMap::new();
    let baseline = SecurityBaseline {
        tools: context().policy,
        ..Default::default()
    };
    let mut agents = BTreeMap::new();
    for name in ["worker", "fast", "slow", "fail"] {
        let model = Arc::new(ControlledModel {
            name: name.into(),
            gate: gates
                .as_array()
                .unwrap()
                .contains(&json!(name))
                .then(|| Semaphore::new(0)),
            started: Semaphore::new(0),
            calls: Mutex::new(vec![]),
        });
        let mut agent = AgentConfig::new(name, ModelBinding::complete(name, model.clone()));
        agent.tools.push(Arc::new(MutationTool(ToolDefinition {
            name: "mutate".into(),
            description: "mutation".into(),
            input_schema: schemars::schema_for!(Value),
            read_only: false,
            requires_approval: false,
        })));
        runners.insert(
            name.into(),
            Runner::new(agent, RunnerConfig::default()).unwrap(),
        );
        agents.insert(name.into(), baseline.clone());
        models.insert(name.into(), model);
    }
    let owner = Scheduler::new(
        context().operation,
        SchedulerConfig {
            security: baseline,
            agents,
            ..Default::default()
        },
        Arc::new(RunnerChildExecutor::new(runners, Arc::new(TestHost))),
        None,
    )
    .unwrap();
    let tools = build_subagent_task_tools(Arc::new(SubagentSession::new(owner.handle())), "worker");
    Harness {
        owner,
        tools,
        models,
    }
}
fn resolve(value: &mut Value, ids: &BTreeMap<String, String>) {
    match value {
        Value::String(s) if s.starts_with('$') => *s = ids[&s[1..]].clone(),
        Value::Array(items) => items.iter_mut().for_each(|item| resolve(item, ids)),
        Value::Object(map) => map.values_mut().for_each(|item| resolve(item, ids)),
        _ => {}
    }
}
fn normalize(value: &mut Value, ids: &BTreeMap<String, String>) {
    match value {
        Value::String(s) => {
            for (id, label) in ids {
                *s = s.replace(id, label);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| normalize(item, ids)),
        Value::Object(map) => {
            for (key, item) in map {
                if matches!(
                    key.as_str(),
                    "duration" | "started_at" | "timestamp" | "duration_ms"
                ) {
                    *item = json!("<time>");
                } else {
                    normalize(item, ids);
                }
            }
        }
        _ => {}
    }
}
async fn run_case(case: &Value) -> Value {
    let Harness {
        owner,
        tools,
        models,
    } = harness(&case["gates"]);
    let mut observations = serde_json::Map::new();
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    for step in case["steps"].as_array().unwrap() {
        if let Some(name) = step["release"].as_str() {
            models[name].gate.as_ref().unwrap().add_permits(1);
            continue;
        }
        if let Some(name) = step["started"].as_str() {
            models[name].started.acquire().await.unwrap().forget();
            continue;
        }
        if let Some(labels) = step["settle"].as_array() {
            let watched: Vec<_> = labels
                .iter()
                .map(|label| ids[label.as_str().unwrap()].clone())
                .collect();
            owner
                .handle()
                .wait(&watched, WaitMode::All, Some(Duration::from_secs(10)))
                .await
                .unwrap();
            continue;
        }
        let name = step["tool"].as_str().unwrap();
        let mut arguments = step["args"].clone();
        resolve(&mut arguments, &ids);
        let output = tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .unwrap()
            .execute(
                &context(),
                ToolCall {
                    id: step["name"].as_str().unwrap().into(),
                    name: name.into(),
                    arguments,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            output.content.len(),
            1,
            "unexpected extra content: {output:?}"
        );
        let Content::Text { text } = &output.content[0] else {
            panic!("expected text result")
        };
        let content: Value = serde_json::from_str(text).unwrap_or_else(|_| json!(text));
        observations.insert(
            step["name"].as_str().unwrap().into(),
            json!({"content":content,"is_error":output.is_error}),
        );
        for task in owner.handle().list() {
            ids.insert(task.message, task.id);
        }
    }
    let mut tasks = serde_json::Map::new();
    let mut replacements = BTreeMap::new();
    for task in owner.handle().list() {
        replacements.insert(task.id, format!("task:{}", task.message));
        tasks.insert(
            task.message,
            json!({
                "agent":task.agent_name,"status":task.status,"result":task.result,
                "has_error":task.error.is_some(),"depends_on":task.depends_on,
            }),
        );
    }
    let mut calls = serde_json::Map::new();
    for (name, model) in models {
        let observed = model.calls.lock().unwrap();
        if !observed.is_empty() {
            calls.insert(name, json!(*observed));
        }
    }
    let mut result = json!({"tools":observations,"scheduler":tasks,"model_calls":calls});
    normalize(&mut result, &replacements);
    owner.shutdown().await.unwrap();
    result
}

fn differences(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: std::collections::BTreeSet<_> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let at = format!("{path}/{key}");
                match (left.get(key), right.get(key)) {
                    (Some(l), Some(r)) => differences(&at, l, r, out),
                    (Some(l), None) => out.push(format!("{at}: missing Rust field; Go={l}")),
                    (None, Some(r)) => out.push(format!("{at}: extra Rust field={r}")),
                    _ => unreachable!(),
                }
            }
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
            for (index, (l, r)) in left.iter().zip(right).enumerate() {
                differences(&format!("{path}/{index}"), l, r, out);
            }
        }
        _ if expected != actual => out.push(format!("{path}: Go={expected}; Rust={actual}")),
        _ => {}
    }
}
#[test]
fn go_fixture_provenance_matches_generator() {
    use sha2::{Digest, Sha256};
    let fixture = fixture();
    assert_eq!(
        fixture["provenance"]["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["provenance"]["sdk_version"], "v0.0.115");
    assert_eq!(
        fixture["provenance"]["generator_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/subagents.go"))
        )
    );
    assert_eq!(
        fixture["provenance"]["exporter_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../../scripts/replay/subagent_export.py"))
        )
    );
}
#[tokio::test]
async fn model_facing_schemas_match_pinned_go() {
    let reference = fixture();
    let harness = harness(&json!([]));
    let mut drift = vec![];
    assert_eq!(
        harness.tools.len(),
        reference["schemas"].as_object().unwrap().len()
    );
    for tool in &harness.tools {
        let def = tool.definition();
        let actual = serde_json::to_value(&def.input_schema).unwrap();
        differences(
            &format!("{}/input_schema", def.name),
            &reference["schemas"][&def.name]["input_schema"],
            &actual,
            &mut drift,
        );
    }
    harness.owner.shutdown().await.unwrap();
    assert!(
        drift.is_empty(),
        "pinned Go schema drift:\n{}",
        drift.join("\n")
    );
}
#[tokio::test]
async fn tool_metadata_matches_pinned_go() {
    let reference = fixture();
    let harness = harness(&json!([]));
    let mut drift = vec![];
    for tool in &harness.tools {
        let def = tool.definition();
        let expected = &reference["schemas"][&def.name];
        differences(
            &format!("{}/read_only", def.name),
            &expected["read_only"],
            &json!(def.read_only),
            &mut drift,
        );
        differences(
            &format!("{}/requires_approval", def.name),
            &expected["requires_approval"],
            &json!(def.requires_approval),
            &mut drift,
        );
    }
    harness.owner.shutdown().await.unwrap();
    assert!(
        drift.is_empty(),
        "pinned Go metadata drift:\n{}",
        drift.join("\n")
    );
}
async fn compare(section: &str) {
    let mut drift = vec![];
    for case in fixture()["cases"].as_array().unwrap() {
        let observed = tokio::time::timeout(Duration::from_secs(20), run_case(&case["input"]))
            .await
            .expect("controlled scenario must terminate");
        differences(
            &format!("{}/{section}", case["input"]["name"].as_str().unwrap()),
            &case["expected"][section],
            &observed[section],
            &mut drift,
        );
    }
    assert!(
        drift.is_empty(),
        "pinned Go {section} drift:\n{}",
        drift.join("\n")
    );
}
#[tokio::test]
async fn tool_result_fields_match_pinned_go() {
    compare("tools").await;
}
#[tokio::test]
async fn scheduler_outcomes_match_pinned_go() {
    compare("scheduler").await;
}
#[tokio::test]
async fn actual_child_model_requests_match_pinned_go() {
    compare("model_calls").await;
}
