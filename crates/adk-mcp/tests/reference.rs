#![cfg(unix)]

use adk_mcp::{
    Limits, Transport,
    client::normalize_input_schema,
    config::ConfigSnapshot,
    tools::{format_call_result, format_resource_result},
    transport::{HttpTransport, RemoteOptions},
};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

const INPUTS: &str = include_str!("../../../fixtures/mcp/reference/inputs.json");
const OBSERVATIONS: &str = include_str!("../../../fixtures/mcp/reference/observations.json");

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn corpus() -> (Value, Value) {
    (
        serde_json::from_str(INPUTS).unwrap(),
        serde_json::from_str(OBSERVATIONS).unwrap(),
    )
}

#[test]
fn reference_provenance_pins_source_and_harness() {
    let (_, observed) = corpus();
    let provenance = &observed["provenance"];
    let baseline: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mcp/reference/baseline-tests.json"
    ))
    .unwrap();
    assert_eq!(baseline["commit"], provenance["commit"]);
    assert_eq!(baseline["goVersion"], provenance["goVersion"]);
    assert_eq!(
        baseline["observationsSHA256"],
        hash(OBSERVATIONS.as_bytes())
    );
    assert_eq!(baseline["exitCode"], 0);
    for (name, outcome) in baseline["tests"].as_object().unwrap() {
        assert_eq!(
            outcome,
            if name == "TestHelperMCPServer" {
                "skip"
            } else {
                "pass"
            },
            "{name}"
        );
    }
    assert_eq!(
        provenance["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(
        provenance["protocolModule"],
        "github.com/modelcontextprotocol/go-sdk v1.4.1"
    );
    for (name, bytes) in [
        ("fixtures/mcp/reference/inputs.json", INPUTS.as_bytes()),
        (
            "scripts/mcp-reference/run.py",
            include_bytes!("../../../scripts/mcp-reference/run.py").as_slice(),
        ),
        (
            "scripts/mcp-reference/reference_test.go",
            include_bytes!("../../../scripts/mcp-reference/reference_test.go").as_slice(),
        ),
    ] {
        assert_eq!(provenance["harnessSHA256"][name], hash(bytes), "{name}");
    }
    for name in [
        "go.mod",
        "go.sum",
        "pkg/agentsdk/mcp/manager.go",
        "pkg/agentsdk/mcp/tools.go",
        "pkg/agentsdk/mcp/config.go",
        "pkg/agentsdk/mcp/remote.go",
        "pkg/agentsdk/mcp/server_mode.go",
    ] {
        assert_eq!(provenance["sourceSHA256"][name].as_str().unwrap().len(), 64);
    }
}

#[test]
fn schemas_match_running_pinned_go() {
    let (inputs, observed) = corpus();
    let cases = inputs["schemas"].as_array().unwrap();
    let observations = observed["schemas"].as_array().unwrap();
    assert_eq!(cases.len(), observations.len());
    for (case, observation) in cases.iter().zip(observations) {
        assert_eq!(case["name"], observation["name"]);
        assert_eq!(
            normalize_input_schema(case["input"].clone()),
            observation["normalized"],
            "{}",
            case["name"]
        );
    }
}

fn result_input(case: &Value) -> Value {
    let Some(generate) = case["generate"].as_str() else {
        return case["input"].clone();
    };
    let mut block = match generate {
        "long-text" => json!({"type":"text","text":"界".repeat(100000)}),
        "oversized-blob" => {
            json!({"type":"image","mimeType":"application/octet-stream","data":base64::engine::general_purpose::STANDARD.encode(vec![0; 10*1024*1024+1])})
        }
        _ => panic!("unknown recipe"),
    };
    if case["kind"] == "resource" {
        let block = block.as_object_mut().unwrap();
        block.remove("type");
        block.insert("uri".into(), "test://item".into());
        if let Some(data) = block.remove("data") {
            block.insert("blob".into(), data);
        }
        json!({"contents":[block]})
    } else {
        json!({"content":[block]})
    }
}

fn normalize_rendered(
    value: &mut Value,
    workspace: &Path,
    blobs: &mut Vec<Value>,
    errors: &mut usize,
) {
    match value {
        Value::Object(object) => {
            if let Some(path) = object.get("blobSavedTo").and_then(Value::as_str) {
                let path = path.to_owned();
                let data = fs::read(&path).unwrap();
                let canonical = fs::canonicalize(&path).unwrap();
                blobs.push(json!({
                    "bytes":data.len(), "sha256":hash(&data),
                    "mode":format!("{:03o}",fs::metadata(&path).unwrap().permissions().mode() & 0o777),
                    "confined":canonical.starts_with(workspace.join(".mcp/blobs"))
                }));
                for value in object.values_mut() {
                    if let Value::String(text) = value {
                        *text = text.replace(&path, "<blob>");
                    }
                }
            }
            for (key, value) in object.iter_mut() {
                if let Value::String(text) = value {
                    if key == "error"
                        || (key == "text" && text.starts_with("Binary content could not be saved"))
                    {
                        *errors += 1;
                        *text = "<blob-error>".into();
                    }
                }
                normalize_rendered(value, workspace, blobs, errors);
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_rendered(value, workspace, blobs, errors);
            }
        }
        Value::String(text) if text.len() > 1024 => {
            *value = json!({"textBytes":text.len(), "textSHA256":hash(text.as_bytes())});
        }
        _ => {}
    }
}

#[test]
fn results_match_running_go_including_blob_effects_and_explicit_decode_delta() {
    let (inputs, observed) = corpus();
    let cases = inputs["results"].as_array().unwrap();
    let observations = observed["results"].as_array().unwrap();
    assert_eq!(cases.len(), observations.len());
    for (case, observation) in cases.iter().zip(observations) {
        assert_eq!(case["name"], observation["name"]);
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        if case["setup"] == "symlink" {
            symlink(outside.path(), workspace.path().join(".mcp")).unwrap();
        }
        let input = result_input(case);
        let rendered = if case["kind"] == "resource" {
            format_resource_result(workspace.path(), "server-resource", &input)
        } else {
            format_call_result(workspace.path(), "server-tool", &input)
        }
        .unwrap();
        let mut rendered = serde_json::from_str(&rendered).unwrap_or(Value::String(rendered));
        let mut blobs = Vec::new();
        let mut errors = 0;
        normalize_rendered(&mut rendered, workspace.path(), &mut blobs, &mut errors);
        assert_eq!(json!(blobs), observation["blobs"], "{}", case["name"]);
        if observation.get("decodeError").is_some() {
            assert_eq!(case["name"], "invalid-base64");
            assert!(case["delta"].as_str().unwrap().contains("decoder rejects"));
            assert_eq!(
                observation["decodeError"],
                "illegal base64 data at input byte 0"
            );
            assert_eq!(rendered["content"][0]["error"], "<blob-error>");
            assert_eq!(errors, 1);
        } else {
            assert_eq!(rendered, observation["rendered"], "{}", case["name"]);
            assert_eq!(errors, observation["diagnostics"].as_array().unwrap().len());
        }
        assert_eq!(
            input["isError"].as_bool().unwrap_or(false),
            observation["isError"].as_bool().unwrap()
        );
        if case["setup"] == "symlink" {
            assert_eq!(observation["outsideEmpty"], true);
            assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn config_observations_distinguish_parse_policy_and_intentional_fail_closed_deltas() {
    let (inputs, observed) = corpus();
    let cases = inputs["configs"].as_array().unwrap();
    let observations = observed["configs"].as_array().unwrap();
    assert_eq!(cases.len(), observations.len());
    for (case, observation) in cases.iter().zip(observations) {
        assert_eq!(case["name"], observation["name"]);
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join(".mcp.json");
        let mut data = serde_json::to_vec(&case["config"]).unwrap();
        if case["setup"] == "oversized" {
            data.extend(vec![b' '; 1024 * 1024]);
        }
        fs::write(&path, data).unwrap();
        if case["setup"] == "symlink" {
            let real = workspace.path().join("real.json");
            fs::rename(&path, &real).unwrap();
            symlink(real, &path).unwrap();
        }
        let snapshot = ConfigSnapshot::load(workspace.path());
        assert_eq!(
            snapshot.is_ok(),
            case["rustAccept"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(observation["exists"], true);
        assert_eq!(observation["loadAccept"], case["name"] != "wrong-tool-list");
        if observation["loadAccept"] != case["rustAccept"] {
            assert!(!case["delta"].as_str().unwrap().is_empty());
        }
        if case["layer"] == "remote" {
            assert!(
                observation["hostDenied"]
                    .as_str()
                    .unwrap()
                    .contains("not enabled by host policy")
            );
            let url = case["config"]["mcpServers"]["bad"]["url"].as_str().unwrap();
            for (allow_private_network, key) in
                [(false, "endpointPublic"), (true, "endpointPrivateOptIn")]
            {
                // Streamable construction checks the URL/IP but sends no request.
                let result = HttpTransport::connect(
                    "bad",
                    url,
                    false,
                    RemoteOptions {
                        tenant_id: "reference-tenant".into(),
                        allow_private_network,
                        ..RemoteOptions::default()
                    },
                    Limits::default(),
                )
                .await;
                assert_eq!(
                    result.is_ok(),
                    observation[key]["accept"].as_bool().unwrap(),
                    "{} {key}",
                    case["name"]
                );
                if let Ok(mut transport) = result {
                    transport.close().await.unwrap();
                }
            }
        }
        if case["layer"] == "dispatch" {
            assert!(observation["dispatchError"].as_str().is_some());
            assert!(
                snapshot.is_err(),
                "{} must match Go dispatch rejection",
                case["name"]
            );
        }
        if case["layer"] == "env" {
            let snapshot = snapshot.unwrap();
            let server = snapshot.config().server("bad").unwrap();
            assert_eq!(
                observation["filteredEnv"]["GITHUB_TOKEN"],
                "synthetic-secret"
            );
            let denied = server.filtered_env(&BTreeMap::new(), &BTreeSet::new());
            assert!(!denied.contains_key("GITHUB_TOKEN"));
            assert_eq!(denied["NORMAL"], "value");
            assert_eq!(
                serde_json::to_value(
                    server.filtered_env(&BTreeMap::new(), &BTreeSet::from(["GITHUB_TOKEN".into()]))
                )
                .unwrap(),
                observation["filteredEnv"]
            );
        }
    }
}
