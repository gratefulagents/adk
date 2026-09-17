use crate::{workspace::Workspace, write::resolve_existing};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Position {
    pub line: i64,
    pub character: i64,
}
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}
impl Range {
    fn stable(mut self) -> Result<Self, String> {
        for position in [&mut self.start, &mut self.end] {
            if position.line < 0 || position.character < 0 {
                return Err("negative LSP position".into());
            }
            position.line = position.line.checked_add(1).ok_or("LSP line overflow")?;
            position.character = position
                .character
                .checked_add(1)
                .ok_or("LSP character overflow")?;
        }
        Ok(self)
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub file_path: String,
    pub range: Range,
}
#[derive(Debug, Clone, Serialize)]
pub struct Hover {
    pub contents: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub name: String,
    pub kind: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_range: Option<Range>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Symbol>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub file_path: String,
    pub range: Range,
    #[serde(skip_serializing_if = "is_zero")]
    pub severity: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    pub message: String,
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct ResultData {
    pub operation: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover: Option<Hover>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<Symbol>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) fn file_uri(path: &Path) -> Result<String, String> {
    url::Url::from_file_path(path)
        .map(|uri| uri.to_string())
        .map_err(|_| "invalid file path".into())
}
pub(super) fn confined_uri(workspace: &Workspace, raw: &str) -> Option<String> {
    // URL parsers normalize localhost to an empty host; reject authorities before parsing.
    let rest = raw.strip_prefix("file://")?;
    if !rest.starts_with('/') {
        return None;
    }
    let uri = url::Url::parse(raw).ok()?;
    if uri.host_str().is_some() || uri.query().is_some() || uri.fragment().is_some() {
        return None;
    }
    let path = uri.to_file_path().ok()?;
    let relative = workspace.relative(path.to_str()?).ok()?;
    let resolved = resolve_existing(&workspace.root.join(relative)).ok()?;
    resolved
        .starts_with(&workspace.root)
        .then(|| resolved.to_string_lossy().into_owned())
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}
fn location(raw: &Value, workspace: &Workspace) -> Result<Option<Location>, String> {
    if !raw.is_object() {
        return Err("invalid LSP location".into());
    }
    let direct = raw
        .get("uri")
        .and_then(Value::as_str)
        .filter(|uri| !uri.is_empty());
    let uri = direct
        .or_else(|| raw.get("targetUri").and_then(Value::as_str))
        .unwrap_or("");
    let Some(path) = confined_uri(workspace, uri) else {
        return Ok(None);
    };
    let range = raw.get(if direct.is_some() {
        "range"
    } else {
        "targetSelectionRange"
    });
    Ok(Some(Location {
        file_path: path,
        range: range
            .map(decode::<Range>)
            .transpose()?
            .unwrap_or_default()
            .stable()?,
    }))
}
fn hover_contents(raw: &Value, depth: usize) -> Result<String, String> {
    if depth > 32 {
        return Err("LSP hover content nesting exceeds 32 levels".into());
    }
    match raw {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Object(map) => map
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or("invalid LSP hover contents".into()),
        Value::Array(items) => Ok(items
            .iter()
            .map(|item| hover_contents(item, depth + 1))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")),
        _ => Err("invalid LSP hover contents".into()),
    }
}
fn symbols(
    raw: &Value,
    workspace: &Workspace,
    workspace_symbols: bool,
    depth: usize,
    nodes: &mut usize,
) -> Result<Vec<Symbol>, String> {
    if depth > 64 {
        return Err("LSP symbol nesting exceeds 64 levels".into());
    }
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let items = raw.as_array().ok_or("invalid LSP symbols")?;
    *nodes += items.len();
    if *nodes > 10000 {
        return Err("LSP symbol result exceeds 10000 nodes".into());
    }
    let mut output = Vec::new();
    for item in items {
        #[derive(Default, Deserialize)]
        #[serde(default, rename_all = "camelCase")]
        struct Node {
            name: String,
            kind: i64,
            detail: String,
            container_name: String,
            range: Option<Range>,
            selection_range: Option<Range>,
        }
        let node: Node = decode(item)?;
        let mut symbol = Symbol {
            name: node.name,
            kind: node.kind,
            detail: node.detail,
            range: None,
            selection_range: None,
            location: None,
            children: Vec::new(),
        };
        if workspace_symbols || item.get("location").is_some() {
            let raw_location = item
                .get("location")
                .ok_or("workspace symbol has no location")?;
            let Some(location) = location(raw_location, workspace)? else {
                continue;
            };
            symbol.location = Some(location);
            symbol.detail = node.container_name;
        } else {
            symbol.range = Some(node.range.ok_or("document symbol has no range")?.stable()?);
            symbol.selection_range = Some(
                node.selection_range
                    .ok_or("document symbol has no selection range")?
                    .stable()?,
            );
            symbol.children = symbols(&item["children"], workspace, false, depth + 1, nodes)?;
        }
        output.push(symbol);
    }
    Ok(output)
}
pub(super) fn diagnostics(raw: &Value, path: &str) -> Result<Vec<Diagnostic>, String> {
    let raw = raw.get("items").unwrap_or(raw);
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let values = raw.as_array().ok_or("invalid LSP diagnostic report")?;
    values
        .iter()
        .map(|value| {
            #[derive(Default, Deserialize)]
            #[serde(default)]
            struct Item {
                range: Range,
                severity: i64,
                code: Value,
                source: String,
                message: String,
            }
            let item: Item = decode(value)?;
            Ok(Diagnostic {
                file_path: path.into(),
                range: item.range.stable()?,
                severity: item.severity,
                code: match item.code {
                    Value::String(code) => code,
                    Value::Number(code) => code.to_string(),
                    _ => String::new(),
                },
                source: item.source,
                message: item.message,
            })
        })
        .collect()
}
pub(super) fn parse(
    operation: &str,
    raw: &Value,
    workspace: &Workspace,
    path: &str,
) -> Result<ResultData, String> {
    let mut result = ResultData {
        operation: operation.into(),
        ..Default::default()
    };
    match operation {
        "definition" | "references" | "implementation" | "typeDefinition" => {
            if let Some(items) = raw.as_array() {
                for item in items {
                    if let Some(location) = location(item, workspace)? {
                        result.locations.push(location);
                    }
                }
            } else if !raw.is_null()
                && let Some(location) = location(raw, workspace)?
            {
                result.locations.push(location);
            }
        }
        "hover" if !raw.is_null() => {
            if !raw.is_object() {
                return Err("invalid LSP hover".into());
            }
            result.hover = Some(Hover {
                contents: hover_contents(&raw["contents"], 0)?,
                range: raw
                    .get("range")
                    .filter(|v| !v.is_null())
                    .map(decode::<Range>)
                    .transpose()?
                    .map(Range::stable)
                    .transpose()?,
            });
        }
        "documentSymbol" | "workspaceSymbol" => {
            result.symbols = symbols(raw, workspace, operation == "workspaceSymbol", 0, &mut 0)?
        }
        "diagnostics" => result.diagnostics = diagnostics(raw, path)?,
        _ => {}
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn nulls_symbols_and_workspace_uri_filtering() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(root.path()).unwrap();
        for operation in [
            "definition",
            "references",
            "hover",
            "documentSymbol",
            "workspaceSymbol",
            "implementation",
            "typeDefinition",
            "diagnostics",
        ] {
            assert_eq!(
                serde_json::to_value(parse(operation, &Value::Null, &workspace, "").unwrap())
                    .unwrap(),
                json!({"operation":operation})
            );
        }
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        assert!(
            confined_uri(
                &workspace,
                &file_uri(&root.path().join("escape/missing.txt")).unwrap()
            )
            .is_none()
        );
        let uri = file_uri(&root.path().join("missing.txt")).unwrap();
        assert!(confined_uri(&workspace, &uri).is_some());
        assert!(confined_uri(&workspace, "https://example.com/a").is_none());
        let range = json!({"start":{"line":0,"character":0},"end":{"line":2,"character":3}});
        let symbols = parse("documentSymbol", &json!([
            {"name":"parent","kind":12,"detail":"detail","range":range,"selectionRange":range,"children":[
                {"name":"child","kind":13,"range":range,"selectionRange":range}
            ]},
            {"name":"information","kind":1,"containerName":"container","location":{"uri":uri,"range":range}},
            {"name":"outside","kind":1,"location":{"uri":"file:///outside","range":range}}
        ]), &workspace, "").unwrap();
        assert_eq!(symbols.symbols.len(), 2);
        assert_eq!(
            symbols.symbols[0].children[0]
                .range
                .as_ref()
                .unwrap()
                .end
                .line,
            3
        );
        assert_eq!(symbols.symbols[1].detail, "container");
        assert!(
            parse(
                "documentSymbol",
                &json!([{"name":"no range"}]),
                &workspace,
                ""
            )
            .is_err()
        );
        assert!(
            parse(
                "workspaceSymbol",
                &json!([{"name":"no location"}]),
                &workspace,
                ""
            )
            .is_err()
        );
        assert!(
            Range {
                start: Position {
                    line: i64::MAX,
                    character: 0
                },
                ..Default::default()
            }
            .stable()
            .is_err()
        );
    }
    #[test]
    fn results_and_limits() {
        let root = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(root.path()).unwrap();
        let uri = file_uri(&root.path().join("a b.txt")).unwrap();
        let r = json!({"start":{"line":0,"character":3},"end":{"line":1,"character":0}});
        let result = parse("definition", &json!([{"targetUri":uri,"targetSelectionRange":r},{"uri":"file:///outside","range":r}]), &workspace, "").unwrap();
        assert_eq!(result.locations.len(), 1);
        assert_eq!(result.locations[0].range.start.character, 4);
        assert!(confined_uri(&workspace, "file://localhost/tmp/a").is_none());
        assert!(confined_uri(&workspace, &(uri.clone() + "?q")).is_none());
        assert!(confined_uri(&workspace, &(uri + "#f")).is_none());
        let hover = parse(
            "hover",
            &json!({"contents":["one",{"language":"rust","value":"two"}]}),
            &workspace,
            "",
        )
        .unwrap();
        assert_eq!(hover.hover.unwrap().contents, "one\n\ntwo");
        assert!(parse("diagnostics", &json!({"kind":"unchanged"}), &workspace, "").is_err());
        assert_eq!(
            diagnostics(&json!([{"range":r,"code":12,"message":"bad"}]), "a").unwrap()[0].code,
            "12"
        );
        let many = Value::Array(vec![json!({}); 10001]);
        assert!(
            symbols(&many, &workspace, false, 0, &mut 0)
                .unwrap_err()
                .contains("10000")
        );
        let mut nested = json!("text");
        for _ in 0..34 {
            nested = json!([nested]);
        }
        assert!(hover_contents(&nested, 0).is_err());
    }
}
