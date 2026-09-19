use crate::{
    Capability, json_text,
    search_pattern::{Pattern, go_regex},
    workspace::{Entry, Workspace},
};
use adk_core::{
    BoxFuture, Content, Context, Error, ErrorCategory, Tool, ToolCall, ToolContext, ToolDefinition,
    ToolOutput,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufRead, BufReader, Read},
    path::Path,
    sync::Arc,
};

pub(crate) fn builtin(capability: &Capability) -> Option<Arc<dyn Tool>> {
    (cfg!(any(target_os = "linux", target_os = "macos")) && capability.family == "workspace-search")
        .then(|| {
            Arc::new(Search {
                definition: capability.definition.clone().expect("search definition"),
            }) as Arc<dyn Tool>
        })
}
struct Search {
    definition: ToolDefinition,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    path: String,
    limit: i64,
    start_line: i64,
    end_line: i64,
    pattern: String,
    cursor: String,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    respect_gitignore: bool,
    skip_default_dirs: Option<bool>,
    output_format: String,
    glob: String,
    ignore_case: bool,
    before_context: i64,
    after_context: i64,
    mode: String,
}

fn failure(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::Tool, message)
}
fn io_failure(error: io::Error) -> Error {
    failure(error.to_string()).with_source(error)
}
fn output(text: String, is_error: bool) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error,
        should_pause: false,
    }
}

pub(crate) fn go_string(bytes: &[u8]) -> String {
    let mut rest = bytes;
    let mut result = String::new();
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                result.push_str(valid);
                break;
            }
            Err(error) => {
                result.push_str(
                    std::str::from_utf8(&rest[..error.valid_up_to()]).expect("valid prefix"),
                );
                result.push('�');
                rest = &rest[error.valid_up_to() + 1..];
            }
        }
    }
    result
}
fn bound_line(line: &[u8]) -> String {
    if line.len() > 400 {
        format!("{}...", go_string(&line[..400]))
    } else {
        go_string(line)
    }
}
fn skip_directory(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "vendor" | ".venv" | "target"
    )
}

fn walk(
    ws: &Workspace,
    root: &Path,
    skip: bool,
    context: &Context,
    mut visitor: impl FnMut(&Path, &Entry) -> Result<bool, Error>,
) -> Result<(), Error> {
    let meta = ws
        .open(root)
        .and_then(|file| file.metadata())
        .map_err(io_failure)?;
    let mut stack = vec![(
        root.to_path_buf(),
        Entry {
            name: root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            directory: meta.is_dir(),
            symlink: false,
        },
    )];
    while let Some((path, entry)) = stack.pop() {
        context.check_active()?;
        if !visitor(&path, &entry)? {
            break;
        }
        if entry.directory {
            let entries = ws.entries(&path).map_err(io_failure)?;
            for entry in entries.into_iter().rev() {
                if skip && entry.directory && skip_directory(&entry.name) {
                    continue;
                }
                stack.push((path.join(&entry.name), entry));
            }
        }
    }
    Ok(())
}
fn relative_string(path: &Path) -> String {
    path.strip_prefix(".")
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

struct Ignore {
    base: String,
    pattern: Option<Pattern>,
    negate: bool,
    directory: bool,
    anchored: bool,
}
struct Filters {
    include: Vec<Pattern>,
    exclude: Vec<Pattern>,
    ignores: Vec<Ignore>,
}
impl Filters {
    fn new(ws: &Workspace, input: &Input, context: &Context) -> Result<Self, Error> {
        let patterns = |items: &Option<Vec<String>>, label: &str| -> Result<Vec<Pattern>, Error> {
            items
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|p| {
                    Pattern::new(p)
                        .map_err(|error| failure(format!("invalid {label} pattern {p:?}: {error}")))
                })
                .collect()
        };
        let mut result = Self {
            include: patterns(&input.include, "include")?,
            exclude: patterns(&input.exclude, "exclude")?,
            ignores: Vec::new(),
        };
        if input.respect_gitignore {
            walk(
                ws,
                Path::new("."),
                input.skip_default_dirs.unwrap_or(true),
                context,
                |path, entry| {
                    if entry.directory || entry.name != ".gitignore" {
                        return Ok(true);
                    }
                    let Ok(file) = ws.read_file(path) else {
                        return Ok(true);
                    };
                    if file.metadata().map_err(io_failure)?.len() > 1 << 20 {
                        return Ok(true);
                    }
                    let mut bytes = Vec::new();
                    if file.take((1 << 20) + 1).read_to_end(&mut bytes).is_err()
                        || bytes.len() > 1 << 20
                    {
                        return Ok(true);
                    }
                    for line in go_string(&bytes).lines() {
                        let mut line = line.trim();
                        if line.is_empty() || line.len() > 4096 || line.starts_with('#') {
                            continue;
                        }
                        if result.ignores.len() >= 10000 {
                            return Err(failure(
                                "respect_gitignore cannot load more than 10000 rules",
                            ));
                        }
                        let negate = line.starts_with('!');
                        if negate {
                            line = &line[1..];
                        }
                        let anchored = line.starts_with('/');
                        line = line.strip_prefix('/').unwrap_or(line);
                        let directory = line.ends_with('/');
                        line = line.strip_suffix('/').unwrap_or(line);
                        if !line.is_empty() {
                            result.ignores.push(Ignore {
                                base: relative_string(path.parent().unwrap_or(Path::new("."))),
                                pattern: Pattern::new(line).ok(),
                                negate,
                                directory,
                                anchored: anchored || line.contains('/'),
                            });
                        }
                    }
                    Ok(true)
                },
            )?;
        }
        Ok(result)
    }
    fn ignored_direct(&self, path: &str, directory: bool) -> bool {
        let mut ignored = false;
        for rule in &self.ignores {
            let candidate = if rule.base.is_empty() || rule.base == "." {
                path
            } else {
                let Some(candidate) = path.strip_prefix(&format!("{}/", rule.base)) else {
                    continue;
                };
                candidate
            };
            let Some(pattern) = &rule.pattern else {
                continue;
            };
            let components: Vec<_> = candidate.split('/').collect();
            let count = if directory || !rule.directory {
                components.len()
            } else {
                components.len().saturating_sub(1)
            };
            let matched = (1..=count).any(|i| {
                if rule.anchored {
                    pattern.matches(&components[..i].join("/"))
                } else {
                    components[..i]
                        .iter()
                        .any(|component| pattern.matches(component))
                }
            });
            if matched {
                ignored = !rule.negate;
            }
        }
        ignored
    }
    fn includes(&self, path: &str) -> bool {
        let parts: Vec<_> = path.split('/').collect();
        if (1..parts.len()).any(|i| self.ignored_direct(&parts[..i].join("/"), true))
            || self.ignored_direct(path, false)
        {
            return false;
        }
        (self.include.is_empty()
            || self
                .include
                .iter()
                .any(|p| p.matches_path_or_basename(path)))
            && !self
                .exclude
                .iter()
                .any(|p| p.matches_path_or_basename(path))
    }
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    v: u32,
    q: String,
    o: i64,
}
fn decode_cursor(raw: &str, query: &str) -> Result<usize, String> {
    if raw.is_empty() {
        return Ok(0);
    }
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| "invalid cursor")?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| "invalid cursor")?;
    if cursor.v != 1 || cursor.o < 0 {
        return Err("invalid cursor".into());
    }
    if cursor.q != query {
        return Err("cursor does not match this search".into());
    }
    usize::try_from(cursor.o).map_err(|_| "invalid cursor".into())
}
fn encode_cursor(query: &str, offset: usize) -> String {
    URL_SAFE_NO_PAD.encode(
        json_text(&Cursor {
            v: 1,
            q: query.into(),
            o: offset as i64,
        })
        .expect("cursor JSON"),
    )
}
fn fingerprint(kind: &str, ws: &Workspace, input: &Input) -> String {
    #[derive(Serialize)]
    struct Query<'a, T> {
        kind: &'a str,
        value: T,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct GlobQuery<'a> {
        workspace: String,
        pattern: &'a str,
        path: &'a str,
        include: &'a Option<Vec<String>>,
        exclude: &'a Option<Vec<String>>,
        respect_gitignore: bool,
        skip_default_dirs: bool,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct GrepQuery<'a> {
        workspace: String,
        pattern: &'a str,
        path: &'a str,
        mode: &'a str,
        include: &'a Option<Vec<String>>,
        exclude: &'a Option<Vec<String>>,
        ignore_case: bool,
        before: i64,
        after: i64,
        respect_gitignore: bool,
        skip_default_dirs: bool,
    }
    let data = if kind == "glob" {
        json_text(&Query {
            kind,
            value: GlobQuery {
                workspace: ws.root.to_string_lossy().into_owned(),
                pattern: &input.pattern,
                path: &input.path,
                include: &input.include,
                exclude: &input.exclude,
                respect_gitignore: input.respect_gitignore,
                skip_default_dirs: input.skip_default_dirs.unwrap_or(true),
            },
        })
    } else {
        json_text(&Query {
            kind,
            value: GrepQuery {
                workspace: ws.root.to_string_lossy().into_owned(),
                pattern: &input.pattern,
                path: &input.path,
                mode: &input.mode,
                include: &input.include,
                exclude: &input.exclude,
                ignore_case: input.ignore_case,
                before: input.before_context,
                after: input.after_context,
                respect_gitignore: input.respect_gitignore,
                skip_default_dirs: input.skip_default_dirs.unwrap_or(true),
            },
        })
    }
    .expect("query JSON");
    URL_SAFE_NO_PAD.encode(Sha256::digest(data.as_bytes()))
}
#[derive(Serialize)]
struct Match {
    path: String,
    line: usize,
    text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    before: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    after: Vec<String>,
}
#[derive(Serialize)]
struct Count {
    path: String,
    count: usize,
}
#[derive(Serialize)]
struct Page<T> {
    matches: T,
    truncated: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    next_cursor: String,
    incomplete: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    omitted_files: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    omitted_files_truncated: bool,
}
fn text_metadata(mut text: String, count: usize, truncated: bool, next: &str) -> String {
    if truncated {
        #[derive(Serialize)]
        struct Metadata<'a> {
            matches: usize,
            truncated: bool,
            next_cursor: &'a str,
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "[search_metadata {}]",
            json_text(&Metadata {
                matches: count,
                truncated: true,
                next_cursor: next
            })
            .expect("metadata JSON")
        ));
    }
    text
}

impl Search {
    fn read(
        &self,
        ws: &Workspace,
        path: &Path,
        input: &Input,
        context: &Context,
    ) -> Result<ToolOutput, Error> {
        let file = match ws.read_file(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let basename = Path::new(&input.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let mut exact = Vec::new();
                let mut partial = Vec::new();
                let mut visited = 0;
                let _ = walk(ws, Path::new("."), true, context, |path, entry| {
                    visited += 1;
                    if visited > 20000 {
                        return Ok(false);
                    }
                    if !entry.directory {
                        if entry.name == basename {
                            exact.push(relative_string(path));
                        } else if partial.len() < 5
                            && entry.name.to_lowercase().contains(&basename.to_lowercase())
                        {
                            partial.push(relative_string(path));
                        }
                    }
                    Ok(exact.len() < 5)
                });
                let suggestions = if exact.is_empty() { partial } else { exact };
                return Ok(output(
                    if suggestions.is_empty() {
                        format!("{}: no such file — use glob to locate the file", input.path)
                    } else {
                        format!(
                            "{}: no such file — did you mean one of: {}",
                            input.path,
                            suggestions.join(", ")
                        )
                    },
                    true,
                ));
            }
            Err(error) => return Err(io_failure(error)),
        };
        if input.start_line <= 0 && input.end_line <= 0 {
            let mut bytes = Vec::new();
            file.take(100001)
                .read_to_end(&mut bytes)
                .map_err(io_failure)?;
            let truncated = bytes.len() > 100000;
            bytes.truncate(100000);
            let mut text = go_string(&bytes);
            if truncated {
                text.push_str("\n[output truncated]");
            }
            return Ok(output(text, false));
        }
        let start = input.start_line.max(1);
        if input.end_line > 0 && input.end_line < start {
            return Ok(output(String::new(), false));
        }
        let mut reader = BufReader::new(file);
        let mut bytes = Vec::new();
        let mut number = 0;
        let mut truncated = false;
        loop {
            context.check_active()?;
            let mut line = Vec::new();
            let n = (&mut reader)
                .take(100001)
                .read_until(b'\n', &mut line)
                .map_err(io_failure)?;
            if n == 0 {
                break;
            }
            if n >= 100001 && !line.ends_with(b"\n") {
                return Err(failure("bufio.Scanner: token too long"));
            }
            number += 1;
            if number < start {
                continue;
            }
            if input.end_line > 0 && number > input.end_line {
                break;
            }
            if line.ends_with(b"\n") {
                line.pop();
            }
            if line.ends_with(b"\r") {
                line.pop();
            }
            if !bytes.is_empty() {
                if bytes.len() == 100000 {
                    truncated = true;
                    break;
                }
                bytes.push(b'\n');
            }
            let remaining = 100000 - bytes.len();
            bytes.extend_from_slice(&line[..line.len().min(remaining)]);
            if line.len() > remaining {
                truncated = true;
                break;
            }
        }
        let mut text = go_string(&bytes);
        if truncated {
            text.push_str("\n[output truncated]");
        }
        Ok(output(text, false))
    }

    fn invoke(&self, context: &ToolContext, mut value: Value) -> Result<ToolOutput, Error> {
        if value.is_null() {
            value = json!({});
        }
        if let Value::Object(fields) = &mut value {
            let properties = &self.definition.input_schema.as_value()["properties"];
            fields.retain(|name, value| properties.get(name).is_some() && !value.is_null());
            for name in ["include", "exclude"] {
                if let Some(Value::Array(items)) = fields.get_mut(name) {
                    for item in items.iter_mut().filter(|item| item.is_null()) {
                        *item = json!("");
                    }
                }
            }
        }
        let mut input: Input = serde_json::from_value(value)
            .map_err(|error| Error::new(ErrorCategory::InvalidInput, error.to_string()))?;
        let name = self.definition.name.as_str();
        if matches!(name, "glob" | "grep") {
            if input.pattern.is_empty() {
                return Ok(output("pattern is required".into(), true));
            }
            if input.pattern.len() > 4096 {
                return Ok(output("pattern must not exceed 4096 bytes".into(), true));
            }
            if input.limit > 1000 {
                return Ok(output("limit must not exceed 1000".into(), true));
            }
            if !matches!(input.output_format.as_str(), "" | "text" | "json") {
                return Ok(output("output_format must be text or json".into(), true));
            }
            if input.before_context < 0 || input.after_context < 0 {
                return Ok(output("context values must be non-negative".into(), true));
            }
            if input.before_context > 10 || input.after_context > 10 {
                return Ok(output("context values must not exceed 10".into(), true));
            }
            if input.mode.is_empty() {
                input.mode = "matches".into();
            }
            if !matches!(input.mode.as_str(), "matches" | "files" | "count") {
                return Ok(output("mode must be matches, files, or count".into(), true));
            }
            if !input.glob.is_empty() {
                if let Err(error) = Pattern::new(&input.glob) {
                    return Ok(output(format!("invalid glob filter: {error}"), true));
                }
                input
                    .include
                    .get_or_insert_default()
                    .push(input.glob.clone());
            }
        }
        let ws = Workspace::new(&context.work_dir).map_err(io_failure)?;
        let path = ws.relative(&input.path).map_err(io_failure)?;
        if name == "read_file" {
            return self.read(&ws, &path, &input, &context.operation);
        }
        if name == "list_files" {
            let mut names: Vec<_> = ws
                .entries(&path)
                .map_err(io_failure)?
                .into_iter()
                .map(|entry| {
                    if entry.directory {
                        format!("{}/", entry.name)
                    } else {
                        entry.name
                    }
                })
                .collect();
            names.sort();
            names.truncate(if input.limit <= 0 {
                100
            } else {
                input.limit as usize
            });
            return Ok(output(names.join("\n"), false));
        }
        let filters = match Filters::new(&ws, &input, &context.operation) {
            Ok(filters) => filters,
            Err(error) => {
                context.operation.check_active()?;
                return Ok(output(error.to_string(), true));
            }
        };
        let query = fingerprint(name, &ws, &input);
        let offset = match decode_cursor(&input.cursor, &query) {
            Ok(offset) => offset,
            Err(error) => return Ok(output(error, true)),
        };
        let limit = if input.limit <= 0 {
            200
        } else {
            input.limit as usize
        };
        if name == "glob" {
            let pattern = match Pattern::new(&input.pattern) {
                Ok(p) => p,
                Err(error) => return Ok(output(format!("invalid glob pattern: {error}"), true)),
            };
            let mut page = Vec::new();
            let mut seen = 0;
            walk(
                &ws,
                &path,
                input.skip_default_dirs.unwrap_or(true),
                &context.operation,
                |found, entry| {
                    let relative = relative_string(found);
                    if !entry.directory && filters.includes(&relative) {
                        let query_relative = found.strip_prefix(&path).unwrap_or(found);
                        let query_relative = if query_relative.as_os_str().is_empty() {
                            ".".into()
                        } else {
                            query_relative.to_string_lossy()
                        };
                        let matched = if input.pattern.contains("**") {
                            pattern.matches(&query_relative)
                        } else {
                            pattern.matches_path_or_basename(&query_relative)
                        };
                        if matched {
                            if seen >= offset {
                                page.push(relative);
                            }
                            seen += 1;
                        }
                    }
                    Ok(page.len() <= limit)
                },
            )?;
            let truncated = page.len() > limit;
            page.truncate(limit);
            let next = if truncated {
                encode_cursor(&query, offset + page.len())
            } else {
                String::new()
            };
            let text = if input.output_format == "json" {
                json_text(&Page {
                    matches: &page,
                    truncated,
                    next_cursor: next,
                    incomplete: false,
                    omitted_files: vec![],
                    omitted_files_truncated: false,
                })
                .expect("page JSON")
            } else {
                text_metadata(
                    if page.is_empty() {
                        "(no matches)".into()
                    } else {
                        page.join("\n")
                    },
                    page.len(),
                    truncated,
                    &next,
                )
            };
            return Ok(output(text, false));
        }
        self.grep(
            &ws,
            &path,
            &input,
            &filters,
            (&query, offset, limit),
            &context.operation,
        )
    }

    fn grep(
        &self,
        ws: &Workspace,
        root: &Path,
        input: &Input,
        filters: &Filters,
        page: (&str, usize, usize),
        context: &Context,
    ) -> Result<ToolOutput, Error> {
        let (query, offset, limit) = page;
        let re = match go_regex(&input.pattern, input.ignore_case) {
            Ok(re) => re,
            Err(error) => return Ok(output(format!("invalid regex: {error}"), true)),
        };
        let mut matches = Vec::<Match>::new();
        let mut files = Vec::new();
        let mut counts = Vec::new();
        let mut omitted = Vec::new();
        let mut omitted_truncated = false;
        let mut seen = 0;
        let mut add_omitted = |message: String| {
            if omitted.len() < 100 {
                omitted.push(message);
            } else {
                omitted_truncated = true;
            }
        };
        walk(
            ws,
            root,
            input.skip_default_dirs.unwrap_or(true),
            context,
            |path, entry| {
                let relative = relative_string(path);
                if entry.directory || entry.symlink || !filters.includes(&relative) {
                    return Ok(true);
                }
                let file = match ws.open(path) {
                    Ok(file) => file,
                    Err(_) => {
                        add_omitted(format!("{relative}: cannot open"));
                        return Ok(true);
                    }
                };
                let meta = match file.metadata() {
                    Ok(meta) => meta,
                    Err(_) => {
                        add_omitted(format!("{relative}: cannot stat"));
                        return Ok(true);
                    }
                };
                if !meta.is_file() {
                    return Ok(true);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.nlink() != 1 {
                        add_omitted(format!("{relative}: hard-linked file refused"));
                        return Ok(true);
                    }
                }
                if meta.len() > 10 << 20 {
                    add_omitted(format!("{relative}: file exceeds 10 MiB search bound"));
                    return Ok(true);
                }
                let mut data = Vec::new();
                file.take((10 << 20) + 1)
                    .read_to_end(&mut data)
                    .map_err(io_failure)?;
                let mut lines = Vec::new();
                for line in data.split_inclusive(|b| *b == b'\n') {
                    if line.len() >= 1 << 20 && (line.len() > 1 << 20 || !line.ends_with(b"\n")) {
                        add_omitted(format!("{relative}: line too long"));
                        break;
                    }
                    let line = line.strip_suffix(b"\n").unwrap_or(line);
                    lines.push(line.strip_suffix(b"\r").unwrap_or(line));
                }
                if data.len() > 10 << 20 {
                    add_omitted(format!("{relative}: file grew past 10 MiB search bound"));
                    lines.pop();
                }
                let mut count = 0;
                for (i, line) in lines.iter().enumerate() {
                    context.check_active()?;
                    if !re.is_match(&go_string(line)) {
                        continue;
                    }
                    count += 1;
                    if input.mode == "matches" {
                        if seen >= offset {
                            matches.push(Match {
                                path: relative.clone(),
                                line: i + 1,
                                text: bound_line(line),
                                before: lines[i.saturating_sub(input.before_context as usize)..i]
                                    .iter()
                                    .map(|l| bound_line(l))
                                    .collect(),
                                after: lines[i + 1
                                    ..(i + 1 + input.after_context as usize).min(lines.len())]
                                    .iter()
                                    .map(|l| bound_line(l))
                                    .collect(),
                            });
                        }
                        seen += 1;
                        if matches.len() > limit {
                            return Ok(false);
                        }
                    }
                }
                if count > 0 && input.mode != "matches" {
                    if seen >= offset {
                        if input.mode == "files" {
                            files.push(relative);
                        } else {
                            counts.push(Count {
                                path: relative,
                                count,
                            });
                        }
                    }
                    seen += 1;
                }
                Ok(files.len() + counts.len() <= limit)
            },
        )?;
        let truncated = matches.len() + files.len() + counts.len() > limit;
        matches.truncate(limit);
        files.truncate(limit);
        counts.truncate(limit);
        omitted.sort();
        let count = matches.len() + files.len() + counts.len();
        let next = if truncated {
            encode_cursor(query, offset + count)
        } else {
            String::new()
        };
        if input.output_format == "json" {
            #[derive(Serialize)]
            #[serde(untagged)]
            enum Records<'a> {
                Files(&'a [String]),
                Counts(&'a [Count]),
                Matches(&'a [Match]),
            }
            let page = match input.mode.as_str() {
                "files" => Records::Files(&files),
                "count" => Records::Counts(&counts),
                _ => Records::Matches(&matches),
            };
            return Ok(output(
                json_text(&Page {
                    matches: page,
                    truncated,
                    next_cursor: next,
                    incomplete: !omitted.is_empty() || omitted_truncated,
                    omitted_files: omitted,
                    omitted_files_truncated: omitted_truncated,
                })
                .expect("page JSON"),
                false,
            ));
        }
        let mut text = match input.mode.as_str() {
            "files" => files.join("\n"),
            "count" => counts
                .iter()
                .map(|m| format!("{}: {}", m.path, m.count))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => matches
                .iter()
                .map(|m| {
                    let mut lines = Vec::new();
                    for (i, line) in m.before.iter().enumerate() {
                        lines.push(format!(
                            "{}-{}- {}",
                            m.path,
                            m.line - m.before.len() + i,
                            line
                        ));
                    }
                    lines.push(format!("{}:{}: {}", m.path, m.line, m.text));
                    for (i, line) in m.after.iter().enumerate() {
                        lines.push(format!("{}-{}- {}", m.path, m.line + i + 1, line));
                    }
                    lines.join("\n")
                })
                .collect::<Vec<_>>()
                .join(if input.before_context > 0 || input.after_context > 0 {
                    "\n--\n"
                } else {
                    "\n"
                }),
        };
        for omitted in omitted {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("[skipped {omitted}]"));
        }
        if omitted_truncated {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str("[additional skipped files omitted from output]");
        }
        if text.is_empty() {
            text = "(no matches)".into();
        }
        Ok(output(text_metadata(text, count, truncated, &next), false))
    }
}
impl Tool for Search {
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
            self.invoke(context, call.arguments)
        })
    }
}
