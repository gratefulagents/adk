#![cfg(feature = "compat")]

use adk::codec::{dto, from_go_json, to_go_json_bytes};

#[test]
fn cursor_codec_is_available_without_host_runtime() {
    let cursor: dto::Cursor =
        from_go_json(br#"{"MESSAGE_ID":-0,"to\u212aen":"next","token":null}"#).unwrap();
    assert_eq!(cursor.message_id, 0);
    assert_eq!(cursor.token, "next");
    assert_eq!(to_go_json_bytes(&cursor).unwrap(), br#"{"token":"next"}"#);
}

#[test]
fn payload_decoder_is_available_through_standalone_facade() {
    let message: dto::MessageOutput =
        from_go_json(br#"{"TEXT":"hello","text":null,"images":[null]}"#).unwrap();
    assert_eq!(message.text, "hello");
    assert_eq!(message.images, vec![dto::ImageAttachment::default()]);

    let call: dto::ToolCallData =
        from_go_json(br#"{"ID":"call","input":{"x":1e+02,"x":2}}"#).unwrap();
    assert_eq!(call.id, "call");
    assert_eq!(call.input.try_text().unwrap(), r#"{"x":1e+02,"x":2}"#);
    let unknown: dto::RunItemType = from_go_json(b"-42").unwrap();
    assert_eq!(unknown.0, -42);
    assert_eq!(to_go_json_bytes(&unknown).unwrap(), b"-42");
    let call: dto::ToolCallData = from_go_json(b"{\"input\":\"\xff\"}").unwrap();
    assert_eq!(call.input.bytes().as_ref(), b"\"\xff\"");
    assert!(call.input.try_text().is_err());
    assert_eq!(
        to_go_json_bytes(&call).unwrap(),
        b"{\"id\":\"\",\"name\":\"\",\"input\":\"\xff\"}"
    );
}

#[test]
fn item_helpers_are_available_through_standalone_facade() {
    use adk::codec::items::{extract_last_text, extract_text, extract_tool_calls};
    let items = [
        serde_json::from_slice(br#"{"Message":{"text":"hello"}}"#).unwrap(),
        serde_json::from_slice(br#"{"Type":1,"ToolCall":{"name":"bash","input":{"x":1e0}}}"#)
            .unwrap(),
        serde_json::from_slice(br#"{"Message":{"text":"world"}}"#).unwrap(),
    ];
    assert_eq!(extract_text(&items), "hello\nworld");
    assert_eq!(extract_last_text(&items), "world");
    let calls = extract_tool_calls(&items).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "bash");
    assert_eq!(calls[0].input.bytes().as_ref(), br#"{"x":1e0}"#);
    assert!(extract_tool_calls(&[]).is_none());
}

#[test]
fn untyped_value_decoder_is_available_through_standalone_facade() {
    let value = adk::codec::value::decode(b"[9007199254740993,-0]").unwrap();
    assert_eq!(
        adk::codec::snapshots::to_go_json(&value).unwrap(),
        b"[9007199254740992,-0]"
    );
}

#[cfg(feature = "observability")]
#[test]
fn logger_preserves_and_byte_truncates_invalid_raw_input() {
    use adk::agent_logger::{AgentLogger, LogLevel};
    use std::io::{Read, Seek};
    let mut file = tempfile::tempfile().unwrap();
    let logger = AgentLogger::new(LogLevel::DEBUG, file.try_clone().unwrap());
    let mut raw = vec![b'"'];
    raw.extend(std::iter::repeat_n(0xff, 310));
    raw.push(b'"');
    logger
        .input_items(&[dto::RunItem {
            kind: dto::RunItemType::TOOL_CALL,
            tool_call: Some(dto::ToolCallData {
                input: dto::RawJson::EncodedBytes(raw.clone()),
                ..Default::default()
            }),
            ..Default::default()
        }])
        .unwrap();
    logger.flush().unwrap();
    file.rewind().unwrap();
    let mut logged = Vec::new();
    file.read_to_end(&mut logged).unwrap();
    let mut expected = b" input=".to_vec();
    expected.extend_from_slice(&raw[..300]);
    expected.extend_from_slice("…\n".as_bytes());
    assert!(logged.ends_with(&expected));
}

#[test]
fn snapshot_schema_and_tool_codecs_are_available_through_facade() {
    use adk::codec::snapshots::{OutputSchema, ToolSnapshot};
    let schema: OutputSchema = from_go_json(
        b"{\"NAME\":\"schema\",\"name\":null,\"SCHEMA\":\"\xff\",\"STRICT\":true,\"strict\":null}",
    )
    .unwrap();
    assert_eq!(schema.name, "schema");
    assert!(schema.strict);
    assert_eq!(
        to_go_json_bytes(&schema).unwrap(),
        b"{\"name\":\"schema\",\"schema\":\"\xff\",\"strict\":true}"
    );
    let tool: ToolSnapshot = from_go_json(br#"{"NAME":"tool","READ_ONLY":true,"read_only":null,"needs_approval":true,"TIMEOUT_SECONDS":-0,"INPUT_SCHEMA":{"z":1,"z":2}}"#).unwrap();
    assert_eq!(
        to_go_json_bytes(&tool).unwrap(),
        br#"{"name":"tool","input_schema":{"z":1,"z":2},"read_only":true,"needs_approval":true}"#
    );
}

#[test]
fn snapshot_settings_codec_is_available_through_facade() {
    let settings: adk::codec::snapshots::ModelSettings = from_go_json(
        br#"{"MAX_TOKENS":7,"max_tokens":null,"temperature":-0,"stop_sequences":["a"],"STOP_SEQUENCES":[null]}"#,
    ).unwrap();
    assert_eq!(settings.max_tokens, 7);
    assert_eq!(
        settings.temperature.unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(settings.stop_sequences, ["a"]);
    assert_eq!(
        to_go_json_bytes(&settings).unwrap(),
        br#"{"temperature":-0,"max_tokens":7,"stop_sequences":["a"]}"#
    );
}

#[test]
fn signed_cost_projection_is_available_without_runtime_accounting() {
    use adk::codec::{
        cost::estimate_result_cost,
        dto::{ModelResponse, Usage},
    };
    let usage = Usage {
        input_tokens: 2,
        ..Default::default()
    };
    let responses = [ModelResponse {
        usage: Usage {
            input_tokens: -1,
            ..Default::default()
        },
        cost_usd: -0.25,
        cost_known: true,
        ..Default::default()
    }];
    let callback_usage = std::cell::RefCell::new(None);
    let price = |usage: &Usage| {
        *callback_usage.borrow_mut() = Some(usage.clone());
        (0.75, false)
    };
    assert_eq!(
        estimate_result_cost(Some((&usage, &responses)), Some(&price)),
        (0.5, false)
    );
    assert_eq!(callback_usage.borrow().as_ref().unwrap().input_tokens, 3);
    assert_eq!(estimate_result_cost(None, Some(&price)), (0.0, false));
}

#[test]
fn governed_kinds_are_available_through_compat_facade() {
    use adk::codec::agents::{
        Definition, Kind, built_in_kinds, definitions, is_allowed, parse_kind, resolve_kind,
    };
    assert_eq!(built_in_kinds().len(), 11);
    assert_eq!(definitions().len(), 11);
    assert_eq!(parse_kind("Plan"), Some(Kind::PLANNER));
    assert!(resolve_kind("Explore").unwrap().read_only);
    assert!(is_allowed(&Kind::EXECUTOR, &["general-purpose".into()]));
    let metadata: Definition =
        from_go_json(br#"{"KIND":"planner","READONLY":true,"ALIASES":["Plan"]}"#).unwrap();
    assert_eq!(
        to_go_json_bytes(&metadata).unwrap(),
        br#"{"kind":"planner","readOnly":true,"aliases":["Plan"]}"#
    );
}

#[test]
fn modelsdev_catalog_is_available_without_provider_transport() {
    let catalog = adk::codec::modelsdev::parse_catalog(
        br#"{"openai":{"models":{"m":{"limit":{"context":100,"output":20}}}}}"#,
    )
    .unwrap();
    assert_eq!(
        adk::codec::modelsdev::lookup(&catalog, " OPENAI ", "openai/m")
            .unwrap()
            .context_tokens,
        100
    );
}

#[cfg(feature = "providers")]
#[test]
fn modelsdev_resolver_options_are_available_through_facade() {
    use adk::providers::modelsdev::{CacheTtl, Options, Resolver};
    let options = Options {
        url: "not a URL".into(),
        cache_path: None,
        ttl: CacheTtl::nanoseconds(-1),
        ..Options::default()
    };
    assert!(Resolver::new(options).is_ok());
}

#[test]
fn legacy_policy_projections_are_available_without_authorization_changes() {
    use adk::codec::policy::{self, RuntimePolicy, ToolPolicy};
    let legacy = RuntimePolicy::default().normalize();
    assert_eq!(legacy.permission_mode, "workspace-write");
    assert_eq!(legacy.git_remote_writes, "enabled");
    assert!(!legacy.enable_mcp);
    assert!(ToolPolicy::new(&legacy).allows_write_tools());
    assert!(!policy::allows_mcp_tool("read-only", false));
    assert!(policy::allows_mcp_tool("danger-full-access", false));
    assert!(
        policy::tool_policy_from_permission_mode("read-only")
            .unwrap()
            .approval_required
    );
    assert!(policy::tool_policy_from_permission_mode("danger-full-access").is_none());
    assert_eq!(
        adk::core::ToolPolicy::default().access,
        adk::core::AccessMode::ReadOnly
    );
}

#[test]
fn sandbox_environment_projections_are_standalone_and_explicit() {
    use adk::codec::sandbox_env::{
        EnvironmentConfig, config_env_names, expand_safe, safe_environment_map_posix,
    };
    use std::collections::BTreeMap;
    let parent = BTreeMap::from([
        (b"PATH".to_vec(), b"/usr/bin".to_vec()),
        (b"OPENAI_API_KEY".to_vec(), b"dummy-secret".to_vec()),
    ]);
    let config = EnvironmentConfig {
        extra_env: BTreeMap::from([(b"CUSTOM".to_vec(), b"$PATH".to_vec())]),
        ..Default::default()
    };
    let environment = safe_environment_map_posix(&parent, &config);
    assert_eq!(
        expand_safe(b"$CUSTOM/${OPENAI_API_KEY}", &environment),
        b"/usr/bin/"
    );
    assert!(!environment.contains_key(b"OPENAI_API_KEY".as_slice()));
    assert_eq!(config_env_names().len(), 8);
}
