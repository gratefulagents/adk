use super::*;
use adk_core::{Compaction, Content, Message, Role, RunItem};

#[test]
fn anthropic_skips_foreign_compaction_and_keeps_native_and_legacy() {
    let mut input = request();
    input.input.push(RunItem::Message {
        message: Message {
            role: Role::User,
            content: vec![Content::Text {
                text: "task".into(),
            }],
        },
    });
    for (id, encrypted_content, created_by) in [
        ("cmp_openai", "openai-blob", "openai"),
        ("cmp_native", "anthropic-blob", "anthropic"),
        ("cmp_legacy", "unstamped-blob", ""),
    ] {
        input.input.push(RunItem::Compaction {
            compaction: Compaction {
                id: id.into(),
                encrypted_content: encrypted_content.into(),
                created_by: created_by.into(),
                ..Default::default()
            },
        });
    }
    let body = wire::request(&input, Protocol::Anthropic, false).unwrap();
    let kept: Vec<_> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().unwrap())
        .filter(|block| block["type"] == "compaction")
        .map(|block| block["encrypted_content"].as_str().unwrap())
        .collect();
    assert_eq!(kept, ["anthropic-blob", "unstamped-blob"]);
}

#[test]
fn anthropic_downconverts_foreign_summary_without_forwarding_ciphertext() {
    let mut input = request();
    for (id, encrypted_content, content) in [
        (
            "cmp_openai",
            "openai-blob",
            "user asked for X; agent finished steps A and B",
        ),
        ("cmp_openai_opaque", "openai-blob-2", ""),
    ] {
        input.input.push(RunItem::Compaction {
            compaction: Compaction {
                id: id.into(),
                encrypted_content: encrypted_content.into(),
                created_by: "openai".into(),
                content: content.into(),
            },
        });
    }
    let body = wire::request(&input, Protocol::Anthropic, false).unwrap();
    let blocks: Vec<_> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().unwrap())
        .collect();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    let text = blocks[0]["text"].as_str().unwrap();
    assert!(text.contains("steps A and B"));
    assert!(text.contains("[CONTEXT SUMMARY"));
    assert!(!body.to_string().contains("openai-blob"));
}
