use adk_codec::{approval::*, config::*, dto};
use adk_core::{Content, Message, Role, RunItem, ToolCall, ToolOutput};
use serde_json::json;
use std::time::Duration;

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "Edit".into(),
        arguments: json!({"text":"日本語", "n":9007199254740993_u64}),
    }
}
fn result(id: &str, error: bool) -> RunItem {
    RunItem::ToolResult {
        call_id: id.into(),
        output: ToolOutput {
            content: vec![Content::Text {
                text: if error { "host denied" } else { "done" }.into(),
            }],
            is_error: error,
            should_pause: false,
        },
    }
}
fn marker(id: &str, phase: ApprovalPhase, before_item: usize) -> ApprovalMarkerBoundary {
    ApprovalMarkerBoundary {
        before_item,
        marker: ApprovalMarker::from_call(&call(id), phase, None),
    }
}

#[test]
fn signed_sentinels_roundtrip_without_normalizing_original() {
    for n in [i64::MIN, -81, -1, 0, 1, 49, 100, u32::MAX as i64] {
        for untrusted in [None, Some(false), Some(true)] {
            let wire = RunConfigSentinels {
                max_turns: n,
                sub_agent_max_turns: n,
                max_concurrent_sub_agents: n,
                consecutive_tool_error_limit: n,
                stop_gate_max_blocks: n,
                max_tool_output_bytes: n,
                model_call_timeout: n,
                untrusted_tool_outputs: untrusted,
                tool_policy: Some(ToolPolicySentinels {
                    approval_required: true,
                    default_timeout: n,
                }),
            };
            let encoded = serde_json::to_value(&wire).unwrap();
            let restored: RunConfigSentinels = serde_json::from_value(encoded.clone()).unwrap();
            let effective = restored.resolve().unwrap();
            assert_eq!(serde_json::to_value(&restored).unwrap(), encoded);
            assert_eq!(
                effective.max_turns.get(),
                if n > 0 { n as u32 } else { 100 }
            );
            assert_eq!(
                effective.sub_agent_max_turns.get(),
                if n > 0 { n as u32 } else { 50 }
            );
            assert_eq!(
                effective.stop_gate_max_blocks,
                if n > 0 { n as usize } else { 8 }
            );
            assert_eq!(
                effective.max_concurrent_sub_agents,
                if n > 0 { Some(n as usize) } else { None }
            );
            assert_eq!(
                effective.consecutive_tool_error_limit,
                match n {
                    n if n < 0 => None,
                    0 => Some(3),
                    _ => Some(n as usize),
                }
            );
            assert_eq!(
                effective.max_tool_output_bytes,
                match n {
                    n if n < 0 => None,
                    0 => Some(16384),
                    _ => Some(n as usize),
                }
            );
            assert_eq!(
                effective.model_idle_timeout,
                match n {
                    n if n < 0 => None,
                    0 => Some(Duration::from_secs(300)),
                    _ => Some(Duration::from_nanos(n as u64)),
                }
            );
            assert_eq!(effective.untrusted_tool_outputs, untrusted.unwrap_or(true));
            assert_eq!(
                effective.tool_policy.as_ref().unwrap().default_timeout,
                if n > 0 {
                    Some(Duration::from_secs(n as u64))
                } else {
                    None
                }
            );
            assert_eq!(
                RunConfigSentinels::from_effective(&effective)
                    .unwrap()
                    .resolve()
                    .unwrap(),
                effective
            );
        }
    }
}

#[test]
fn zero_defaults_null_fields_and_absent_policy() {
    let wire: RunConfigSentinels = serde_json::from_value(
        json!({"MaxTurns":null,"ModelCallTimeout":null,"UntrustedToolOutputs":null}),
    )
    .unwrap();
    assert_eq!(wire, RunConfigSentinels::default());
    let effective = wire.resolve().unwrap();
    assert_eq!(effective.tool_policy, None);
    assert_eq!(
        RunConfigSentinels::from_effective(&effective)
            .unwrap()
            .resolve()
            .unwrap(),
        effective
    );
    assert!(serde_json::from_value::<RunConfigSentinels>(json!({"StopGate":true})).is_err());
    assert!(
        serde_json::from_value::<RunConfigSentinels>(
            json!({"ToolPolicy":{"ApprovalRequired":true,"Unknown":4}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<RunConfigSentinels>(json!({"MaxTurns":18446744073709551615_u64}))
            .is_err()
    );
}

#[test]
fn reject_unrepresentable_native_zero_precision_and_range() {
    let mut wire = RunConfigSentinels {
        max_turns: u32::MAX as i64 + 1,
        ..Default::default()
    };
    assert_eq!(wire.resolve().unwrap_err(), ConfigError("MaxTurns"));
    wire.max_turns = 0;
    wire.model_call_timeout = i64::MAX;
    assert_eq!(
        wire.resolve().unwrap().model_idle_timeout,
        Some(Duration::from_nanos(i64::MAX as u64))
    );
    wire.tool_policy = Some(ToolPolicySentinels {
        default_timeout: i64::MAX,
        ..Default::default()
    });
    assert_eq!(wire.resolve().unwrap_err(), ConfigError("DefaultTimeout"));
    let base = RunConfigSentinels::default().resolve().unwrap();
    for duration in [Duration::ZERO, Duration::from_secs(u64::MAX)] {
        let mut native = base.clone();
        native.model_idle_timeout = Some(duration);
        assert_eq!(
            RunConfigSentinels::from_effective(&native).unwrap_err(),
            ConfigError("ModelCallTimeout")
        );
    }
    let mut native = base.clone();
    native.max_tool_output_bytes = Some(0);
    assert_eq!(
        RunConfigSentinels::from_effective(&native).unwrap_err(),
        ConfigError("MaxToolOutputBytes")
    );
    for duration in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_secs(u64::MAX),
    ] {
        let mut native = base.clone();
        native.tool_policy = Some(EffectiveToolPolicy {
            approval_required: false,
            default_timeout: Some(duration),
        });
        assert_eq!(
            RunConfigSentinels::from_effective(&native).unwrap_err(),
            ConfigError("DefaultTimeout")
        );
    }
}

#[test]
fn parallel_pending_then_resolved_markers_preserve_exact_history_order() {
    let items = vec![
        RunItem::ToolCall { call: call("a") },
        RunItem::ToolCall { call: call("b") },
        RunItem::ToolCall { call: call("c") },
        result("b", false),
        result("a", false),
        result("c", true),
    ];
    let agents = vec![None; items.len()];
    let markers = vec![
        marker("a", ApprovalPhase::Pending, 3),
        marker("c", ApprovalPhase::Pending, 4),
        marker("a", ApprovalPhase::Approved, 4),
        marker("c", ApprovalPhase::Denied, 5),
    ];
    let wire = encode_history(&items, &agents, &markers).unwrap();
    assert_eq!(
        wire.iter().map(|v| v.kind.0).collect::<Vec<_>>(),
        [1, 1, 1, 6, 2, 6, 6, 2, 6, 2]
    );
    assert!(!wire[3].tool_approval.as_ref().unwrap().approved);
    assert!(wire[6].tool_approval.as_ref().unwrap().approved);
    assert!(!wire[8].tool_approval.as_ref().unwrap().approved);
    let phases = markers.iter().map(|v| v.marker.phase).collect::<Vec<_>>();
    let native = decode_history(&wire, &phases).unwrap();
    assert_eq!(
        native,
        NativeHistory {
            items,
            agents,
            markers
        }
    );
    assert_eq!(
        encode_history(&native.items, &native.agents, &native.markers).unwrap(),
        wire
    );
    let json_wire = serde_json::to_value(&wire).unwrap();
    let redecoded = serde_json::from_value::<Vec<dto::RunItem>>(json_wire.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(
            encode_history(&native.items, &native.agents, &native.markers).unwrap()
        )
        .unwrap(),
        json_wire
    );
    assert_eq!(decode_history(&redecoded, &phases).unwrap(), native);
}

#[test]
fn empty_and_end_boundaries_and_no_deduplication() {
    assert_eq!(encode_history(&[], &[], &[]).unwrap(), []);
    let m = marker("a", ApprovalPhase::Pending, 0);
    let markers = [m.clone(), m];
    let wire = encode_history(&[], &[], &markers).unwrap();
    assert_eq!(
        decode_history(&wire, &[ApprovalPhase::Pending; 2])
            .unwrap()
            .markers,
        markers
    );
    let future = marker("a", ApprovalPhase::Pending, 1);
    assert!(encode_history(&[], &[], std::slice::from_ref(&future)).is_err());
    assert!(encode_history(&[result("b", false)], &[None], &[future]).is_ok());
}

#[test]
fn reject_ambiguous_contradictory_or_unordered_markers() {
    let wire = vec![
        marker("a", ApprovalPhase::Pending, 0)
            .marker
            .to_wire()
            .unwrap(),
    ];
    assert!(decode_history(&wire, &[]).is_err());
    assert!(decode_history(&wire, &[ApprovalPhase::Approved]).is_err());
    assert!(decode_history(&wire, &[ApprovalPhase::Pending; 2]).is_err());
    assert!(decode_history(&wire, &[ApprovalPhase::Denied]).is_ok());
    assert!(
        encode_history(
            &[result("b", false)],
            &[None],
            &[
                marker("a", ApprovalPhase::Pending, 1),
                marker("b", ApprovalPhase::Pending, 0)
            ]
        )
        .is_err()
    );
    assert!(encode_history(&[result("b", false)], &[], &[]).is_err());
    let mut malformed = wire[0].clone();
    malformed.message = Some(dto::MessageOutput::default());
    assert!(decode_history(&[malformed], &[ApprovalPhase::Pending]).is_err());
}

#[test]
fn approval_native_call_bridge_requires_explicit_reason_and_preserves_null() {
    for arguments in [json!(null), json!({}), json!([1, "🙂"])] {
        let call = ToolCall {
            arguments,
            ..call("a")
        };
        for phase in [
            ApprovalPhase::Pending,
            ApprovalPhase::Approved,
            ApprovalPhase::Denied,
        ] {
            let marker = ApprovalMarker::from_call(
                &call,
                phase,
                Some(dto::AgentRef {
                    name: "worker".into(),
                }),
            );
            let request = marker.to_request("host supplied reason").unwrap();
            assert_eq!(request.call, call);
            assert_eq!(request.reason, "host supplied reason");
            let wire = marker.to_wire().unwrap();
            assert_eq!(
                decode_history(&[wire], &[phase]).unwrap().markers[0].marker,
                marker
            );
        }
    }
    let mut marker = marker("a", ApprovalPhase::Pending, 0).marker;
    marker.data.input = dto::RawJson::Missing;
    assert!(marker.to_request("").is_err());
    assert_eq!(
        decode_history(&[marker.to_wire().unwrap()], &[ApprovalPhase::Pending])
            .unwrap()
            .markers[0]
            .marker,
        marker
    );
}

#[test]
fn native_text_items_roundtrip_and_unsupported_fields_fail_closed() {
    for (role, agent) in [
        (Role::User, None),
        (
            Role::Assistant,
            Some(dto::AgentRef {
                name: "worker".into(),
            }),
        ),
    ] {
        let item = RunItem::Message {
            message: Message {
                role,
                content: vec![Content::Text {
                    text: "hello".into(),
                }],
            },
        };
        let wire = encode_item(&item, agent.as_ref()).unwrap();
        assert_eq!(decode_item(&wire).unwrap(), item);
        let mut phased = wire.clone();
        phased.message.as_mut().unwrap().phase = "analysis".into();
        assert!(decode_item(&phased).is_err());
        let mut media = wire;
        media
            .message
            .as_mut()
            .unwrap()
            .images
            .push(dto::ImageAttachment::default());
        assert!(decode_item(&media).is_err());
    }
    for role in [Role::System, Role::Developer, Role::Assistant] {
        assert!(
            encode_item(
                &RunItem::Message {
                    message: Message {
                        role,
                        content: vec![]
                    }
                },
                None
            )
            .is_err()
        );
    }
    for content in [
        vec![],
        vec![
            Content::Text { text: "a".into() },
            Content::Text { text: "b".into() },
        ],
        vec![Content::Image {
            uri: "x".into(),
            media_type: "image/png".into(),
        }],
    ] {
        assert!(
            encode_item(
                &RunItem::Message {
                    message: Message {
                        role: Role::User,
                        content
                    }
                },
                None
            )
            .is_err()
        );
    }
    assert!(
        encode_item(
            &RunItem::Handoff {
                call_id: "a".into(),
                agent: "b".into()
            },
            None
        )
        .is_err()
    );
    let mut paused = result("a", false);
    if let RunItem::ToolResult { output, .. } = &mut paused {
        output.should_pause = true;
    }
    assert!(encode_item(&paused, None).is_err());
    for kind in [3, 4, 5, 6, 7, 99] {
        assert!(
            decode_item(&dto::RunItem {
                kind: dto::RunItemType(kind),
                ..Default::default()
            })
            .is_err()
        );
    }
    let mut call_wire = encode_item(&RunItem::ToolCall { call: call("a") }, None).unwrap();
    call_wire.tool_call.as_mut().unwrap().input = dto::RawJson::Missing;
    assert!(decode_item(&call_wire).is_err());
}
