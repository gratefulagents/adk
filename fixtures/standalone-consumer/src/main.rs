//! External-workspace compile smoke test: every reusable facade feature enabled.
use adk::core::{Content, Message, Role, RunItem, RunPolicy, RunRequest};

fn main() {
    let request = RunRequest {
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
    assert!(!adk::runtime::CancellationToken::new().is_cancelled());
    println!("External standalone consumer: all reusable facade features compiled");
}
