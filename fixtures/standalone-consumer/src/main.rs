//! External-workspace compile smoke test: every reusable facade feature enabled.
use adk::core::{Content, Message, Role, RunItem, RunPolicy, RunRequest};

fn main() {
    let request = RunRequest {
        input_provenance: Vec::new(),
        input: vec![RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text {
                    text: "embedded".into(),
                }],
            },
        }],
        policy: RunPolicy::default(),
    };
    assert_eq!(request.input.len(), 1);
    let config = adk::builder::Config {
        output_schema: Some(adk::core::JsonDocument::new("true".into()).unwrap()),
        ..Default::default()
    };
    assert!(config.output_schema.is_some());
    assert_eq!(config.output_schema_name, "final_output");
    assert!(config.output_schema_strict);
    assert!(config.output_parser.is_none());
    assert!(!adk::runtime::CancellationToken::new().is_cancelled());
    let cursor = adk::host::Cursor::default();
    assert_eq!(cursor.message_id, 0);
    assert!(cursor.token.is_empty());
    let template: adk::codec::dto::ModeTemplateSpec = adk::codec::from_go_json(
        br#"{"ModelRouting":{"ReasoningLevel":"high","RoleOverrides":{"worker":{"ReasoningLevel":"none"}}}}"#,
    )
    .unwrap();
    let routing = adk::codec::mode::mode_routing_from_template_spec(Some(&template));
    let resolved = adk::codec::mode::resolve_role_mode_routing(
        "medium",
        "anthropic",
        "worker",
        routing.as_ref(),
    );
    assert_eq!(resolved.model, "claude-sonnet-4-6");
    assert_eq!(resolved.model_settings.thinking_budget, 0);
    assert_eq!(adk::codec::to_go_json_bytes(&resolved).unwrap(), br#"{"Model":"claude-sonnet-4-6","FallbackModels":null,"ReasoningLevel":"none","TextVerbosity":"","ModelSettings":{"reasoning_effort":"none"}}"#);
    println!("External standalone consumer: all reusable facade features compiled");
}
