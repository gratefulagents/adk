use adk::{
    core::{BoxFuture, Context, Error, Model, ModelRequest, ModelResponse},
    runtime::{AgentConfig, Handoff, ModelBinding},
    specialists::build_delegation_guide,
};
use std::{collections::BTreeMap, sync::Arc};

struct UnusedModel;
impl Model for UnusedModel {
    fn provider(&self) -> &str {
        "unused"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        panic!("formatting must not execute the model")
    }
}
fn agent(name: &str) -> AgentConfig {
    AgentConfig::new(
        name,
        ModelBinding::complete("unused", Arc::new(UnusedModel)),
    )
}

#[test]
fn delegation_guides_match_pinned_go_exactly() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/specialist-tools/observations.json"
    ))
    .unwrap();
    let cases = fixture["guideCases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let specialists: BTreeMap<_, _> = case["specialists"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, description)| {
                let mut specialist = agent(&format!("agent_{key}"));
                specialist.handoff_description = description.as_str().unwrap().into();
                (key.clone(), Arc::new(specialist))
            })
            .collect();
        let mut parent = agent("parent");
        for spec in case["handoffs"].as_array().into_iter().flatten() {
            let mut handoff = Handoff::new(Arc::new(agent(spec["Target"].as_str().unwrap())));
            handoff.definition.name = spec["Tool"].as_str().unwrap().into();
            handoff.definition.description = spec["Description"].as_str().unwrap().into();
            parent.handoffs.push(handoff);
        }
        assert_eq!(
            build_delegation_guide(&parent, &specialists),
            case["guide"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn original_compact_task_packet_guidance_regression() {
    let parent = agent("");
    let mut executor = agent("executor");
    executor.handoff_description = "Implement a bounded change".into();
    let guide = build_delegation_guide(
        &parent,
        &BTreeMap::from([("executor".into(), Arc::new(executor))]),
    );
    for expected in [
        "- executor: Implement a bounded change",
        "compact, self-contained task packet",
        "Do NOT send the same large background block to every task if only one sub-agent needs it.",
        "Files you own",
        "subagent tool",
        "mode=\"background\"",
        "tasks=[{key, message, depends_on:[keys]}",
        "subagent_status",
        "subagent_wait",
        "subagent_control",
    ] {
        assert!(guide.contains(expected), "{expected}");
    }
    assert_eq!(build_delegation_guide(&parent, &BTreeMap::new()), "");
}
