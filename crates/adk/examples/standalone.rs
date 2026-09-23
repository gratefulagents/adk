//! A native-contract-only application: no Tokio, provider, or platform dependency.
use adk::core::{Content, Message, Role, RunItem, RunPolicy, RunRequest};

fn main() {
    let request = RunRequest {
        input_provenance: Vec::new(),
        input: vec![RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text {
                    text: "Hello, 世界 🦀".into(),
                }],
            },
        }],
        policy: RunPolicy {
            max_turns: std::num::NonZeroU32::new(4).unwrap(),
            tools: Default::default(),
            tool_use: Default::default(),
        },
    };
    assert_eq!(request.input.len(), 1);
    println!("Standalone ADK request constructed (no model or runner executed): {request:?}");
}
