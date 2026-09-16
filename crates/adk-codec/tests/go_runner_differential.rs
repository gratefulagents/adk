//! Executes extracted, unmodified baseline functions without the SDK's provider dependencies.
use adk_codec::{approval::*, config::*, dto};
use adk_core::{Content, RunItem, ToolCall, ToolOutput};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn declaration(source: &str, prefix: &str) -> String {
    let start = source
        .find(prefix)
        .unwrap_or_else(|| panic!("missing Go declaration {prefix}"));
    let end = source[start..].find("\n}").unwrap() + start + 2;
    source[start..end].to_owned() + "\n"
}

#[test]
#[ignore = "requires Go and the pinned repos/sdk checkout; run explicitly with --ignored"]
fn config_and_denied_history_match_executed_go_baseline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let baseline = root.join("repos/sdk");
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&baseline)
        .output()
        .unwrap();
    assert!(revision.status.success());
    assert_eq!(
        String::from_utf8(revision.stdout).unwrap().trim(),
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let config = fs::read_to_string(baseline.join("internal/agent/run_config.go")).unwrap();
    let chatloop = fs::read_to_string(baseline.join("pkg/agentsdk/chatloop.go")).unwrap();
    let items = fs::read_to_string(baseline.join("internal/agent/items.go")).unwrap();
    let mut source = String::from(
        r#"
package main
import ("encoding/json"; "os"; "time")
type RunConfig struct {
    MaxTurns, SubAgentMaxTurns, ConsecutiveToolErrorLimit, StopGateMaxBlocks, MaxToolOutputBytes int
    ModelCallTimeout time.Duration
    UntrustedToolOutputs *bool
}
type Interruption struct { ToolName string; ToolInput json.RawMessage; ToolCallID string }
type RunItem struct { Type int; ToolApproval *ToolApprovalData; ToolOutput *ToolOutputData }
type ImageAttachment struct { MediaType, Data, Detail string }
const ( RunItemToolApproval = 6; RunItemToolOutput = 2 )
"#,
    );
    for name in [
        "DefaultModelCallTimeout",
        "DefaultMaxTurns",
        "DefaultSubAgentMaxTurns",
        "DefaultMaxToolOutputBytes",
        "DefaultConsecutiveToolErrorLimit",
        "DefaultStopGateMaxBlocks",
    ] {
        let prefix = format!("const {name} =");
        source.push_str(
            config
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap(),
        );
        source.push('\n');
    }
    for name in [
        "EffectiveMaxTurns",
        "EffectiveSubAgentMaxTurns",
        "EffectiveConsecutiveToolErrorLimit",
        "EffectiveStopGateMaxBlocks",
        "EffectiveMaxToolOutputBytes",
        "ShouldTagUntrustedToolOutputs",
    ] {
        source.push_str(&declaration(
            &config,
            &format!("func (c *RunConfig) {name}("),
        ));
    }
    source.push_str(&declaration(&config, "func effectiveModelCallTimeout("));
    source.push_str(&declaration(&items, "type ToolApprovalData struct"));
    source.push_str(&declaration(&items, "type ToolOutputData struct"));
    source.push_str(&declaration(&chatloop, "func denyPendingInterruptions("));
    source.push_str(&declaration(&chatloop, "func cloneRawMessage("));
    source.push_str(r#"
func main() {
    var input struct { Configs []RunConfig; Pending []*Interruption }
    if err := json.NewDecoder(os.Stdin).Decode(&input); err != nil { panic(err) }
    configs := []map[string]any{}
    for _, c := range input.Configs {
        configs = append(configs, map[string]any{
            "turns":c.EffectiveMaxTurns(), "sub_turns":c.EffectiveSubAgentMaxTurns(),
            "errors":c.EffectiveConsecutiveToolErrorLimit(), "blocks":c.EffectiveStopGateMaxBlocks(),
            "cap":c.EffectiveMaxToolOutputBytes(), "timeout":effectiveModelCallTimeout(c.ModelCallTimeout),
            "untrusted":c.ShouldTagUntrustedToolOutputs(),
        })
    }
    if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
        "configs":configs, "denied":denyPendingInterruptions(input.Pending, "host denied"),
    }); err != nil { panic(err) }
}
"#);
    let cases: Vec<_> = [i64::MIN, -7, -1, 0, 1, 42, u32::MAX as i64]
        .into_iter()
        .flat_map(|n| {
            [None, Some(false), Some(true)]
                .into_iter()
                .map(move |flag| RunConfigSentinels {
                    max_turns: n,
                    sub_agent_max_turns: n,
                    consecutive_tool_error_limit: n,
                    stop_gate_max_blocks: n,
                    max_tool_output_bytes: n,
                    model_call_timeout: n,
                    untrusted_tool_outputs: flag,
                    ..Default::default()
                })
        })
        .collect();
    let input = json!({"Configs":cases, "Pending":[
        {"ToolName":"Edit","ToolInput":{"a":1},"ToolCallID":"a"}, null,
        {"ToolName":"Edit","ToolInput":null,"ToolCallID":"b"}
    ]});
    let dir = std::env::temp_dir().join(format!("adk-codec-go-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("main.go"), source).unwrap();
    let mut child = Command::new("go")
        .args(["run", "main.go"])
        .current_dir(&dir)
        .env("GOWORK", "off")
        .env("GO111MODULE", "off")
        .env("GOTOOLCHAIN", "local")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    for (case, go) in cases.iter().zip(actual["configs"].as_array().unwrap()) {
        let native = case.resolve().unwrap();
        assert_eq!(
            *go,
            json!({
                "turns":native.max_turns.get(), "sub_turns":native.sub_agent_max_turns.get(),
                "errors":native.consecutive_tool_error_limit.unwrap_or(0), "blocks":native.stop_gate_max_blocks,
                "cap":native.max_tool_output_bytes.unwrap_or(0),
                "timeout":native.model_idle_timeout.map(|d| d.as_nanos() as u64).unwrap_or(0),
                "untrusted":native.untrusted_tool_outputs,
            })
        );
    }
    let native: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| RunItem::ToolResult {
            call_id: id.into(),
            output: ToolOutput {
                content: vec![Content::Text {
                    text: "host denied".into(),
                }],
                is_error: true,
                should_pause: false,
            },
        })
        .collect();
    let markers: Vec<_> = ["a", "b"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| ApprovalMarkerBoundary {
            before_item: index,
            marker: ApprovalMarker::from_call(
                &ToolCall {
                    id: id.into(),
                    name: "Edit".into(),
                    arguments: if index == 0 {
                        json!({"a":1})
                    } else {
                        Value::Null
                    },
                },
                ApprovalPhase::Denied,
                None,
            ),
        })
        .collect();
    let wire = encode_history(&native, &[None, None], &markers).unwrap();
    let go_wire: Vec<dto::RunItem> = serde_json::from_value(actual["denied"].clone()).unwrap();
    assert_eq!(wire, go_wire);
    let decoded = decode_history(&go_wire, &[ApprovalPhase::Denied; 2]).unwrap();
    assert_eq!(decoded.items, native);
    assert_eq!(decoded.markers, markers);
}
