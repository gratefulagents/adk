//! Catalog tools install configuration only; they never start an MCP server.
use crate::{
    workspace::Workspace,
    write::{atomic_write, resolve_existing},
};
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: String,
    pub source: SkillSource,
    #[serde(rename = "mcpConfig")]
    pub mcp_config: CatalogServer,
    pub tags: Vec<String>,
    pub verified: bool,
    pub requires_env_vars: Vec<String>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SkillSource {
    pub repository: String,
    pub r#ref: String,
    pub url: String,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CatalogServer {
    pub r#type: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct Server {
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allow_env: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    trust_read_only_hint: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allowed_tools: Vec<String>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct Config {
    mcp_servers: BTreeMap<String, Server>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    name: String,
    query: String,
    category: String,
}

struct Shared {
    entries: Vec<SkillEntry>,
    fallback_work_dir: PathBuf,
    available_env: BTreeSet<String>,
}
struct SkillTool {
    definition: ToolDefinition,
    shared: Arc<Shared>,
}

/// The catalog, fallback workspace and available environment names are host-owned.
pub fn tools(
    entries: Vec<SkillEntry>,
    fallback_work_dir: PathBuf,
    available_env: BTreeSet<String>,
) -> Vec<Arc<dyn Tool>> {
    let shared = Arc::new(Shared {
        entries,
        fallback_work_dir,
        available_env,
    });
    ["skill_search", "skill_install", "skill_list_installed"]
        .into_iter()
        .map(|name| {
            let definition = crate::capabilities()
                .iter()
                .find(|c| c.name == name)
                .expect("skill capability")
                .definition
                .clone()
                .expect("skill definition");
            Arc::new(SkillTool {
                definition,
                shared: shared.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

fn go_lower(value: &str) -> String {
    value
        .chars()
        .map(|character| character.to_lowercase().next().expect("case mapping"))
        .collect()
}

fn equal_fold(left: &str, right: &str) -> bool {
    use regex_syntax::hir::{ClassUnicode, ClassUnicodeRange};
    let mut right = right.chars();
    for left in left.chars() {
        let Some(other) = right.next() else {
            return false;
        };
        if left == other
            || (left.is_ascii() && other.is_ascii() && left.eq_ignore_ascii_case(&other))
        {
            continue;
        }
        let mut class = ClassUnicode::new([ClassUnicodeRange::new(left, left)]);
        class.case_fold_simple();
        if !class
            .iter()
            .any(|range| range.start() <= other && other <= range.end())
        {
            return false;
        }
    }
    right.next().is_none()
}

fn normalize_fields(value: &mut Value) {
    if value.is_null() {
        *value = json!({});
    }
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|_, value| !value.is_null());
    }
}
fn load(workspace: &Workspace, path: &Path) -> Result<(Config, u32), String> {
    let mut file = match workspace.read_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Config::default(), 0o600));
        }
        Err(error) => return Err(error.to_string()),
    };
    let mode = file
        .metadata()
        .map_err(|e| e.to_string())?
        .permissions()
        .mode()
        & 0o600;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let mut value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    normalize_fields(&mut value);
    if let Some(servers) = value.get_mut("mcpServers").and_then(Value::as_object_mut) {
        for server in servers.values_mut() {
            normalize_fields(server);
            for key in ["args", "allowEnv", "allowedTools"] {
                if let Some(values) = server.get_mut(key).and_then(Value::as_array_mut) {
                    for value in values.iter_mut().filter(|v| v.is_null()) {
                        *value = json!("");
                    }
                }
            }
            if let Some(env) = server.get_mut("env").and_then(Value::as_object_mut) {
                for value in env.values_mut().filter(|v| v.is_null()) {
                    *value = json!("");
                }
            }
        }
    }
    Ok((
        serde_json::from_value(value).map_err(|e| e.to_string())?,
        mode,
    ))
}

impl SkillTool {
    fn invoke(&self, context: &ToolContext, mut arguments: Value) -> Result<String, String> {
        let action = self.definition.name.as_str();
        let input = if action == "skill_list_installed" {
            Input::default()
        } else {
            normalize_fields(&mut arguments);
            serde_json::from_value(arguments).map_err(|e| format!("Invalid input: {e}"))?
        };
        if action == "skill_search" {
            let query = go_lower(&input.query);
            let entries: Vec<_> = self
                .shared
                .entries
                .iter()
                .filter(|entry| {
                    if !input.query.is_empty() {
                        go_lower(&entry.name).contains(&query)
                            || go_lower(&entry.description).contains(&query)
                            || entry.tags.iter().any(|tag| go_lower(tag).contains(&query))
                    } else {
                        input.category.is_empty() || equal_fold(&entry.category, &input.category)
                    }
                })
                .collect();
            if entries.is_empty() {
                return Ok("No skills found matching your criteria.".into());
            }
            let mut text = format!("Found {} skill(s):\n\n", entries.len());
            for entry in entries {
                text.push_str(&format!(
                    "• **{}** (v{}, {}{})\n  {}\n  Tags: {}\n",
                    entry.name,
                    entry.version,
                    entry.category,
                    if entry.verified { " ✓" } else { "" },
                    entry.description,
                    entry.tags.join(", ")
                ));
                if !entry.requires_env_vars.is_empty() {
                    let vars: Vec<_> = entry
                        .requires_env_vars
                        .iter()
                        .map(|name| {
                            format!(
                                "{name} ({})",
                                if self.shared.available_env.contains(name) {
                                    "set"
                                } else {
                                    "NOT set"
                                }
                            )
                        })
                        .collect();
                    text.push_str(&format!("  Requires env: {}\n", vars.join(", ")));
                }
                text.push('\n');
            }
            return Ok(text);
        }
        let skill = if action == "skill_install" {
            Some(
                self.shared
                    .entries
                    .iter()
                    .find(|entry| entry.name == input.name)
                    .ok_or_else(|| {
                        format!(
                            "Failed to install skill {}: skill {} not found in registry",
                            serde_json::to_string(&input.name).expect("name"),
                            serde_json::to_string(&input.name).expect("name")
                        )
                    })?,
            )
        } else {
            None
        };
        let work_dir = if context.work_dir.as_os_str().is_empty() {
            &self.shared.fallback_work_dir
        } else {
            &context.work_dir
        };
        let prefix = if action == "skill_install" {
            format!(
                "Failed to install skill {}: ",
                serde_json::to_string(&input.name).expect("name")
            )
        } else {
            "Failed to list installed skills: ".into()
        };
        let workspace = Workspace::new(work_dir).map_err(|e| format!("{prefix}{e}"))?;
        let path = resolve_existing(&workspace.root.join(".mcp.json"))
            .map_err(|e| format!("{prefix}{e}"))?;
        let relative = path
            .strip_prefix(&workspace.root)
            .map_err(|_| format!("{prefix}path is outside the workspace root"))?;
        let (mut config, mode) = load(&workspace, relative).map_err(|e| format!("{prefix}{e}"))?;
        if let Some(skill) = skill {
            let mut entry = Server {
                kind: skill.mcp_config.r#type.clone(),
                command: skill.mcp_config.command.clone(),
                args: skill.mcp_config.args.clone(),
                env: skill.mcp_config.env.clone(),
                allow_env: skill.requires_env_vars.clone(),
                ..Default::default()
            };
            if let Some(existing) = config.mcp_servers.remove(&skill.name) {
                let mut seen = BTreeSet::new();
                entry.allow_env = existing
                    .allow_env
                    .into_iter()
                    .chain(entry.allow_env)
                    .filter(|name| !name.is_empty() && seen.insert(name.clone()))
                    .collect();
                entry.allowed_tools = existing.allowed_tools;
                entry.enabled = existing.enabled;
                for (name, value) in existing.env {
                    entry.env.entry(name).or_insert(value);
                }
            }
            config.mcp_servers.insert(skill.name.clone(), entry);
            let mut serialized = serde_json::to_string_pretty(&config)
                .map_err(|e| format!("{prefix}marshaling config: {e}"))?;
            serialized = serialized
                .replace('&', "\\u0026")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
                .replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029");
            serialized.push('\n');
            atomic_write(&workspace, relative, serialized.as_bytes(), Some(mode))
                .map_err(|e| format!("{prefix}saving config: {e}"))?;
            let mut text = format!(
                "Skill {} installed: its MCP server config was added to .mcp.json. MCP configs are loaded at session start, so its tools become available after the agent restarts — they are NOT available in this session.",
                serde_json::to_string(&skill.name).expect("name")
            );
            let missing: Vec<_> = skill
                .requires_env_vars
                .iter()
                .filter(|name| !self.shared.available_env.contains(*name))
                .cloned()
                .collect();
            if !missing.is_empty() {
                text.push_str(&format!("\n\nWARNING: required environment variable(s) not set: {}. The server will likely fail to authenticate until they are provided (e.g. via the host's secret mechanism or the entry's env map in .mcp.json; they are already listed in the entry's allowEnv).",missing.join(", ")));
            }
            Ok(text)
        } else {
            if config.mcp_servers.is_empty() {
                return Ok("No skills currently installed in .mcp.json.".into());
            }
            let (skills, others): (Vec<_>, Vec<_>) = config
                .mcp_servers
                .keys()
                .cloned()
                .partition(|name| self.shared.entries.iter().any(|entry| entry.name == *name));
            let mut text = if skills.is_empty() {
                "No skills from the catalog are installed in .mcp.json.".into()
            } else {
                format!("Installed skills: {}", skills.join(", "))
            };
            if !others.is_empty() {
                text.push_str(&format!(
                    "\nOther MCP servers in .mcp.json (not from the skill catalog): {}",
                    others.join(", ")
                ));
            }
            Ok(text)
        }
    }
}
impl Tool for SkillTool {
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
            let (text, is_error) = match self.invoke(context, call.arguments) {
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
