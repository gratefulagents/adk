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
        output_schema: Some(true.into()),
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
    println!("External standalone consumer: all reusable facade features compiled");
}
