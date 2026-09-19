use crate::{Capability, json_text, workspace::Workspace};
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[path = "patch_fs.rs"]
mod fs;
#[path = "patch_hunks.rs"]
mod hunks;
#[path = "patch_parse.rs"]
mod parse;

const MAX_PATCH: usize = 1024 * 1024;
const MAX_FILE: usize = 5 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_AGGREGATE: usize = 64 * 1024 * 1024;
static MUTATION: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct PatchFile {
    old: String,
    new: String,
    old_mode: Option<u32>,
    new_mode: Option<u32>,
    delete_all: bool,
    hunks: Vec<Hunk>,
}
#[derive(Default)]
struct Hunk {
    old_start: i64,
    old_count: i64,
    new_start: i64,
    new_count: i64,
    range_less: bool,
    locator: String,
    eof: bool,
    lines: Vec<Line>,
}
struct Line {
    kind: u8,
    text: String,
    no_newline: bool,
}
#[derive(Clone, Default, PartialEq, Eq, Debug)]
struct State {
    exists: bool,
    data: String,
    mode: u32,
}
struct Plan {
    operation: &'static str,
    old: String,
    new: String,
    state: State,
}

pub(crate) fn builtin(capability: &Capability) -> Option<Arc<dyn Tool>> {
    (capability.name == "ApplyPatch").then(|| {
        Arc::new(ApplyPatch {
            definition: capability.definition.clone().expect("patch definition"),
        }) as Arc<dyn Tool>
    })
}
struct ApplyPatch {
    definition: ToolDefinition,
}

fn plan(
    workspace: &Workspace,
    files: Vec<PatchFile>,
) -> Result<(Vec<Plan>, BTreeMap<String, State>), String> {
    let mut sources = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for file in &files {
        if !file.old.is_empty() {
            if !sources.insert(file.old.clone()) {
                return Err(format!("multiple patch operations use source {}", file.old));
            }
            paths.insert(file.old.clone());
        }
        if !file.new.is_empty() {
            if file.new != file.old && !targets.insert(file.new.clone()) {
                return Err(format!(
                    "multiple patch operations use destination {}",
                    file.new
                ));
            }
            paths.insert(file.new.clone());
        }
    }
    for target in &targets {
        if sources.contains(target) {
            return Err(format!("conflicting patch paths include {target}"));
        }
    }
    let mut finals: Vec<&str> = Vec::new();
    for file in &files {
        if !file.new.is_empty() {
            for existing in &finals {
                if file.new.starts_with(&format!("{existing}/"))
                    || existing.starts_with(&format!("{}/", file.new))
                {
                    return Err(format!(
                        "conflicting ancestor patch paths {existing} and {}",
                        file.new
                    ));
                }
            }
            finals.push(&file.new);
        }
    }
    let mut states = BTreeMap::new();
    let mut total = 0;
    for path in paths {
        let state = fs::inspect(workspace, &path).map_err(|e| format!("{path}: {e}"))?;
        total += state.data.len();
        if total > MAX_AGGREGATE {
            return Err(format!(
                "patch source data exceeds aggregate limit of {MAX_AGGREGATE} bytes"
            ));
        }
        states.insert(path, state);
    }
    let mut plans = Vec::new();
    for file in files {
        let source = states.get(&file.old).cloned().unwrap_or_default();
        let operation = if file.old.is_empty() {
            "create"
        } else if file.new.is_empty() {
            "delete"
        } else if file.old == file.new {
            "modify"
        } else {
            "rename"
        };
        if operation != "create" && !source.exists {
            return Err(format!("{operation} source {} does not exist", file.old));
        }
        if matches!(operation, "create" | "rename") && states[&file.new].exists {
            return Err(format!(
                "{operation} destination {} already exists",
                file.new
            ));
        }
        if file
            .old_mode
            .is_some_and(|mode| !file.old.is_empty() && mode & 0o111 != source.mode & 0o111)
        {
            return Err(format!("mode conflict for {}", file.old));
        }
        if operation == "create" && file.old_mode.is_some() {
            return Err("create operation has an old mode".into());
        }
        if operation == "delete" && file.new_mode.is_some() {
            return Err("delete operation has a new mode".into());
        }
        let updated = if file.delete_all {
            String::new()
        } else {
            hunks::apply(&source.data, &file.hunks).map_err(|e| {
                format!(
                    "{}: {e}",
                    if file.old.is_empty() {
                        &file.new
                    } else {
                        &file.old
                    }
                )
            })?
        };
        if operation == "delete" && !updated.is_empty() {
            return Err(format!(
                "delete patch for {} does not remove all file content",
                file.old
            ));
        }
        if updated.len() > MAX_FILE {
            return Err(format!(
                "patched file {} is too large ({} bytes, limit {MAX_FILE})",
                file.new,
                updated.len()
            ));
        }
        let mut mode = if operation == "create" {
            0o644
        } else {
            source.mode
        };
        if let Some(new_mode) = file.new_mode {
            if operation == "create" {
                mode = new_mode;
            } else if operation != "delete" {
                mode = (mode & !0o111) | (new_mode & 0o111);
            }
        }
        plans.push(Plan {
            operation,
            old: file.old,
            new: file.new,
            state: State {
                exists: true,
                data: updated,
                mode,
            },
        });
    }
    Ok((plans, states))
}
#[derive(Serialize)]
struct Audit<'a> {
    operation: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    path: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    from_path: &'a str,
    #[serde(skip_serializing_if = "String::is_empty")]
    mode: String,
    #[serde(skip_serializing_if = "is_zero")]
    bytes: usize,
}
fn is_zero(n: &usize) -> bool {
    *n == 0
}
fn preview(patch: &str) -> String {
    if patch.len() <= 8192 {
        return json_text(&patch).expect("patch string");
    }
    let cut = patch.as_bytes()[..8192]
        .iter()
        .rposition(|b| *b == b'\n')
        .filter(|n| *n > 0)
        .unwrap_or(8192);
    let mut boundary = cut;
    while !patch.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut encoded = json_text(&&patch[..boundary]).expect("patch string");
    encoded.pop();
    // Go's byte cutoff produces one JSON replacement escape per incomplete UTF-8 byte.
    encoded.push_str(&"\\ufffd".repeat(cut - boundary));
    encoded.push_str("\\n... [diff truncated]\"");
    encoded
}
fn input(arguments: Value) -> Result<(String, bool), String> {
    let kind = |v: &Value| match v {
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        _ => "object",
    };
    if arguments.is_null() {
        return Ok((String::new(), false));
    }
    let Some(fields) = arguments.as_object() else {
        return Err(format!(
            "Invalid input: json: cannot unmarshal {} into Go value of type fs.applyPatchInput",
            kind(&arguments)
        ));
    };
    let mut patch = String::new();
    let mut dry_run = false;
    for (key, value) in fields {
        if value.is_null() {
            continue;
        }
        if key.eq_ignore_ascii_case("patch") {
            patch = value.as_str().ok_or_else(|| format!("Invalid input: json: cannot unmarshal {} into Go struct field applyPatchInput.patch of type string",kind(value)))?.into();
        } else if key.eq_ignore_ascii_case("dry_run") {
            dry_run = value.as_bool().ok_or_else(|| format!("Invalid input: json: cannot unmarshal {} into Go struct field applyPatchInput.dry_run of type bool",kind(value)))?;
        }
    }
    Ok((patch, dry_run))
}
impl ApplyPatch {
    fn run(&self, context: &ToolContext, arguments: Value) -> Result<String, String> {
        let (patch, dry_run) = input(arguments)?;
        let files = parse::parse(&patch).map_err(|e| format!("Invalid patch: {e}"))?;
        let _guard = if dry_run {
            None
        } else {
            Some(MUTATION.lock().unwrap_or_else(|e| e.into_inner()))
        };
        let workspace = Workspace::new(&context.work_dir)
            .map_err(|e| format!("Patch validation failed: {e}"))?;
        let (plans, states) =
            plan(&workspace, files).map_err(|e| format!("Patch validation failed: {e}"))?;
        let operations = plans
            .iter()
            .map(|p| Audit {
                operation: p.operation,
                path: if p.operation == "delete" {
                    &p.old
                } else {
                    &p.new
                },
                from_path: if p.operation == "rename" { &p.old } else { "" },
                mode: if p.operation == "delete" {
                    String::new()
                } else {
                    format!("{:04o}", p.state.mode)
                },
                bytes: if p.operation == "delete" {
                    0
                } else {
                    p.state.data.len()
                },
            })
            .collect::<Vec<_>>();
        let result = format!(
            "{{\"dry_run\":{dry_run},\"operations\":{},\"diff\":{}}}",
            json_text(&operations).expect("patch operations"),
            preview(&patch)
        );
        if result.len() > 65536 {
            return Err(format!(
                "Patch result is too large ({} bytes, limit 65536)",
                result.len()
            ));
        }
        if !dry_run {
            fs::apply(&workspace, &plans, &states)
                .map_err(|e| format!("Error applying patch: {e}"))?;
        }
        Ok(result)
    }
}
impl Tool for ApplyPatch {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let (text, is_error) = match self.run(context, call.arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            Ok(ToolOutput {
                content: vec![Content::Text { text }],
                is_error,
                should_pause: false,
            })
        })
    }
}
