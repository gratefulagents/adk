use super::{Hunk, Line, MAX_FILES, MAX_PATCH, PatchFile};
use regex::Regex;
use std::sync::OnceLock;

pub(super) fn quote(value: &str) -> String {
    static PRINTABLE: OnceLock<Regex> = OnceLock::new();
    let printable = PRINTABLE.get_or_init(|| Regex::new(r"^[\pL\pM\pN\pP\pS ]$").unwrap());
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            c if c < ' ' || c == '\u{7f}' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if !c.is_ascii() && !printable.is_match(c.encode_utf8(&mut [0; 4])) => {
                if c as u32 <= 0xffff {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                } else {
                    out.push_str(&format!("\\U{:08x}", c as u32));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(super) fn path(value: &str, strip: bool) -> Result<String, String> {
    let mut value = value.trim();
    if value == "/dev/null" {
        return Ok(String::new());
    }
    if strip && (value.starts_with("a/") || value.starts_with("b/")) {
        value = &value[2..];
    }
    if value.trim().is_empty() {
        return Err("path is required".into());
    }
    if value.len() > 512 {
        return Err(format!(
            "path is too long ({} bytes, limit 512)",
            value.len()
        ));
    }
    if value.starts_with('/') || value.contains(['\\', '\0', '\t', '\n', '\r']) {
        return Err("path must be a relative slash-separated path".into());
    }
    let parts: Vec<_> = value.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() || parts.iter().any(|s| matches!(*s, "." | "..")) {
        return Err("path traversal is not allowed".into());
    }
    Ok(parts.join("/"))
}
fn control(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}
fn mode(value: &str) -> Result<u32, String> {
    match value {
        "100644" => Ok(0o644),
        "100755" => Ok(0o755),
        _ => Err(format!(
            "unsupported Git blob mode {} (only 100644 and 100755 are supported)",
            quote(value)
        )),
    }
}
pub(super) fn parse(patch: &str) -> Result<Vec<PatchFile>, String> {
    if patch.trim().is_empty() {
        return Err("patch is required".into());
    }
    if patch.len() > MAX_PATCH {
        return Err(format!(
            "patch is too large ({} bytes, limit {MAX_PATCH})",
            patch.len()
        ));
    }
    if patch.contains('\0') {
        return Err("binary patch data is not supported".into());
    }
    let lines: Vec<_> = patch.split('\n').collect();
    if control(lines[0]) == "*** Begin Patch" {
        openai(&lines)
    } else {
        unified(&lines)
    }
}
fn push_file(files: &mut Vec<PatchFile>, file: PatchFile) -> Result<(), String> {
    files.push(file);
    if files.len() > MAX_FILES {
        return Err(format!("patch changes too many files (limit {MAX_FILES})"));
    }
    Ok(())
}
fn finish_openai(
    files: &mut Vec<PatchFile>,
    current: &mut Option<PatchFile>,
) -> Result<(), String> {
    let Some(file) = current.take() else {
        return Ok(());
    };
    if file
        .hunks
        .iter()
        .enumerate()
        .any(|(i, h)| h.eof && i + 1 != file.hunks.len())
    {
        return Err("*** End of File must terminate an update".into());
    }
    if !file.old.is_empty() && file.old == file.new && file.hunks.is_empty() {
        return Err(format!("update file {} contains no changes", file.old));
    }
    push_file(files, file)
}
fn append_hunk(file: &mut PatchFile, hunk: Hunk, count: &mut usize) -> Result<(), String> {
    if *count >= 256 {
        return Err("OpenAI patch has too many hunks (limit 256)".into());
    }
    file.hunks.push(hunk);
    *count += 1;
    Ok(())
}
fn openai_path(directive: &str, value: &str) -> Result<String, String> {
    let value = path(value, false).map_err(|e| format!("{directive}: {e}"))?;
    if value.is_empty() {
        return Err(format!("{directive} cannot be /dev/null"));
    }
    Ok(value)
}
fn openai(lines: &[&str]) -> Result<Vec<PatchFile>, String> {
    let mut files = Vec::new();
    let mut current: Option<PatchFile> = None;
    let mut kind = "";
    let mut moved = false;
    let mut count = 0;
    let mut i = 1;
    while i < lines.len() {
        let line = control(lines[i]);
        if line == "*** End Patch" {
            if !(i + 1 == lines.len() || (i + 2 == lines.len() && lines[i + 1].is_empty())) {
                return Err("content follows *** End Patch".into());
            }
            finish_openai(&mut files, &mut current)?;
            if files.is_empty() {
                return Err("patch contains no file changes".into());
            }
            return Ok(files);
        } else if line.starts_with("*** Update File: ")
            || line.starts_with("*** Add File: ")
            || line.starts_with("*** Delete File: ")
        {
            finish_openai(&mut files, &mut current)?;
            moved = false;
            let mut file = PatchFile::default();
            if let Some(value) = line.strip_prefix("*** Update File: ") {
                file.old = openai_path("update file path", value)?;
                file.new = file.old.clone();
                kind = "update";
            } else if let Some(value) = line.strip_prefix("*** Add File: ") {
                file.new = openai_path("add file path", value)?;
                kind = "add";
            } else {
                file.old = openai_path(
                    "delete file path",
                    line.strip_prefix("*** Delete File: ").unwrap(),
                )?;
                file.delete_all = true;
                kind = "delete";
            }
            current = Some(file);
        } else if let Some(value) = line.strip_prefix("*** Move to: ") {
            if current.is_none() || kind != "update" || moved {
                return Err("move destination must follow one update file directive".into());
            }
            current.as_mut().unwrap().new = openai_path("move destination", value)?;
            moved = true;
        } else if let Some(locator) = line.strip_prefix("@@") {
            if current.is_none() || kind != "update" {
                return Err("range-less hunk must follow an update file directive".into());
            }
            let (hunk, next) = range_hunk(lines, i + 1, locator.trim())?;
            append_hunk(current.as_mut().unwrap(), hunk, &mut count)?;
            i = next;
            continue;
        } else if line == "*** End of File" {
            if current.is_none() || kind != "update" || current.as_ref().unwrap().hunks.is_empty() {
                return Err("*** End of File must follow an update hunk".into());
            }
            let hunk = current.as_mut().unwrap().hunks.last_mut().unwrap();
            if hunk.eof {
                return Err("duplicate *** End of File".into());
            }
            hunk.eof = true;
        } else {
            if kind == "update"
                && current.as_ref().is_some_and(|f| f.hunks.is_empty())
                && line.starts_with([' ', '+', '-'])
            {
                let (hunk, next) = range_hunk(lines, i, "")?;
                append_hunk(current.as_mut().unwrap(), hunk, &mut count)?;
                i = next;
                continue;
            }
            if current.is_none() || kind != "add" || !line.starts_with('+') {
                return Err(format!("unsupported OpenAI patch line {}", quote(line)));
            }
            let file = current.as_mut().unwrap();
            if file.hunks.is_empty() {
                append_hunk(
                    file,
                    Hunk {
                        range_less: true,
                        ..Default::default()
                    },
                    &mut count,
                )?;
            }
            let hunk = &mut file.hunks[0];
            if hunk.lines.len() >= 16384 {
                return Err("OpenAI hunk has too many lines (limit 16384)".into());
            }
            hunk.lines.push(Line {
                kind: b'+',
                text: line[1..].into(),
                no_newline: false,
            });
            hunk.new_count += 1;
        }
        i += 1;
    }
    Err("patch is missing *** End Patch".into())
}
fn range_hunk(lines: &[&str], start: usize, locator: &str) -> Result<(Hunk, usize), String> {
    let mut hunk = Hunk {
        range_less: true,
        locator: locator.into(),
        ..Default::default()
    };
    let mut changed = false;
    for (i, line) in lines.iter().enumerate().skip(start) {
        let line = control(line);
        if line.starts_with("@@") || line.starts_with("*** ") {
            if hunk.lines.is_empty() {
                return Err("range-less hunk has no body".into());
            }
            if !changed {
                return Err("range-less hunk has no changes".into());
            }
            return Ok((hunk, i));
        }
        if !line.starts_with([' ', '+', '-']) {
            return Err("malformed range-less hunk body".into());
        }
        if hunk.lines.len() >= 16384 {
            return Err("OpenAI hunk has too many lines (limit 16384)".into());
        }
        let kind = line.as_bytes()[0];
        hunk.lines.push(Line {
            kind,
            text: line[1..].into(),
            no_newline: false,
        });
        if kind != b'+' {
            hunk.old_count += 1;
        }
        if kind != b'-' {
            hunk.new_count += 1;
        }
        if kind != b' ' {
            changed = true;
        }
    }
    Err("range-less hunk is not followed by a patch directive".into())
}
#[derive(Default)]
struct Headers {
    old_literal: bool,
    new_literal: bool,
    old: bool,
    new: bool,
    rename_from: bool,
    rename_to: bool,
    diff: Option<(String, String)>,
}
fn finish_unified(
    files: &mut Vec<PatchFile>,
    current: &mut Option<PatchFile>,
    headers: &mut Headers,
) -> Result<(), String> {
    let Some(mut file) = current.take() else {
        return Ok(());
    };
    if file.old.is_empty() && file.new.is_empty() && headers.diff.is_none() {
        return Err("patch file has no paths".into());
    }
    if let Some((old, new)) = &headers.diff {
        if file.old.is_empty() {
            file.old = old.clone();
        }
        if file.new.is_empty() {
            file.new = new.clone();
        }
    }
    if !file.hunks.is_empty() && (!headers.old || !headers.new) {
        return Err("patch hunks require exactly one ---/+++ header pair".into());
    }
    if headers.rename_from != headers.rename_to {
        return Err("rename requires paired rename from/to directives".into());
    }
    file.old = path(&file.old, !headers.old_literal).map_err(|e| format!("old path: {e}"))?;
    file.new = path(&file.new, !headers.new_literal).map_err(|e| format!("new path: {e}"))?;
    if file.old.is_empty() && file.new.is_empty() {
        return Err("patch file cannot have /dev/null as both paths".into());
    }
    if let Some((old, new)) = &headers.diff {
        let old = path(old, true).map_err(|e| format!("diff old path: {e}"))?;
        let new = path(new, true).map_err(|e| format!("diff new path: {e}"))?;
        if (!file.old.is_empty() && !old.is_empty() && file.old != old)
            || (!file.new.is_empty() && !new.is_empty() && file.new != new)
        {
            return Err("patch headers or rename metadata disagree with diff paths".into());
        }
    }
    if !(file.old.is_empty()
        || file.new.is_empty()
        || file.old == file.new
        || (headers.rename_from && headers.rename_to))
    {
        return Err(format!(
            "rename from {} to {} requires paired rename metadata",
            file.old, file.new
        ));
    }
    if file.old == file.new
        && file.hunks.is_empty()
        && file.old_mode.is_none()
        && file.new_mode.is_none()
    {
        return Err(format!("patch file {} contains no changes", file.old));
    }
    if file.old.is_empty() && file.old_mode.is_some() {
        return Err(format!("new file {} cannot declare an old mode", file.new));
    }
    if file.new.is_empty() && file.new_mode.is_some() {
        return Err(format!(
            "deleted file {} cannot declare a new mode",
            file.old
        ));
    }
    push_file(files, file)?;
    *headers = Headers::default();
    Ok(())
}
fn unified(lines: &[&str]) -> Result<Vec<PatchFile>, String> {
    let mut files = Vec::new();
    let mut current: Option<PatchFile> = None;
    let mut headers = Headers::default();
    let mut i = 0;
    while i < lines.len() {
        let line = control(lines[i]);
        if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
            return Err("binary patches are not supported".into());
        }
        if line.starts_with("diff --git ") {
            finish_unified(&mut files, &mut current, &mut headers)?;
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() != 4 {
                return Err("quoted or malformed diff paths are not supported".into());
            }
            current = Some(PatchFile::default());
            headers.diff = Some((parts[2].into(), parts[3].into()));
        } else if let Some(value) = line.strip_prefix("--- ") {
            if current.is_some() && headers.old {
                if headers.new
                    && !current.as_ref().unwrap().hunks.is_empty()
                    && headers.diff.is_none()
                {
                    finish_unified(&mut files, &mut current, &mut headers)?;
                } else {
                    return Err("duplicate old-file header".into());
                }
            }
            current.get_or_insert_with(PatchFile::default).old =
                value.split('\t').next().unwrap().into();
            headers.old = true;
        } else if let Some(value) = line.strip_prefix("+++ ") {
            if current.is_none() || !headers.old {
                return Err("new path appears before old path".into());
            }
            if headers.new {
                return Err("duplicate new-file header".into());
            }
            current.as_mut().unwrap().new = value.split('\t').next().unwrap().into();
            headers.new = true;
        } else if let Some((prefix, value)) = [
            "old mode ",
            "new mode ",
            "new file mode ",
            "deleted file mode ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix).map(|v| (*prefix, v)))
        {
            let file = current
                .as_mut()
                .ok_or_else(|| format!("{} appears before a file path", prefix.trim_end()))?;
            let old = prefix == "old mode " || prefix == "deleted file mode ";
            let slot = if old {
                &mut file.old_mode
            } else {
                &mut file.new_mode
            };
            if slot.is_some() {
                return Err(format!(
                    "duplicate {} mode",
                    if old { "old" } else { "new" }
                ));
            }
            *slot = Some(mode(value)?);
        } else if let Some(value) = line.strip_prefix("rename from ") {
            let file = current
                .as_mut()
                .ok_or("rename source appears before a file path")?;
            if headers.rename_from {
                return Err("duplicate rename source".into());
            }
            file.old = value.into();
            headers.old_literal = true;
            headers.rename_from = true;
        } else if let Some(value) = line.strip_prefix("rename to ") {
            let file = current
                .as_mut()
                .ok_or("rename destination appears before a file path")?;
            if headers.rename_to {
                return Err("duplicate rename destination".into());
            }
            file.new = value.into();
            headers.new_literal = true;
            headers.rename_to = true;
        } else if line.starts_with("copy from ") || line.starts_with("copy to ") {
            return Err("copy directives are not supported".into());
        } else if line.starts_with("@@ ") {
            let file = current.as_mut().ok_or("hunk appears before a file path")?;
            let (hunk, next) = unified_hunk(lines, i)?;
            file.hunks.push(hunk);
            i = next;
            continue;
        } else if !(line.is_empty()
            || line.starts_with("index ")
            || line.starts_with("similarity index ")
            || line.starts_with("dissimilarity index "))
        {
            return Err(format!(
                "unsupported or out-of-hunk patch line {}",
                quote(line)
            ));
        }
        i += 1;
    }
    finish_unified(&mut files, &mut current, &mut headers)?;
    if files.is_empty() {
        return Err("patch contains no file changes".into());
    }
    Ok(files)
}
fn unified_hunk(lines: &[&str], start: usize) -> Result<(Hunk, usize), String> {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    let regex = HEADER.get_or_init(|| {
        Regex::new(r"^@@ -([0-9]+)(?:,([0-9]+))? \+([0-9]+)(?:,([0-9]+))? @@").unwrap()
    });
    let header = control(lines[start]);
    let captures = regex
        .captures(header)
        .ok_or_else(|| format!("malformed hunk header {}", quote(header)))?;
    let number = |i, default| -> Result<i64, String> {
        captures.get(i).map_or(Ok(default), |v| {
            v.as_str().parse::<i64>().map_err(|_| {
                format!(
                    "strconv.Atoi: parsing {}: value out of range",
                    quote(v.as_str())
                )
            })
        })
    };
    let mut hunk = Hunk {
        old_start: number(1, 0)?,
        old_count: number(2, 1)?,
        new_start: number(3, 0)?,
        new_count: number(4, 1)?,
        ..Default::default()
    };
    if (hunk.old_count > 0 && hunk.old_start == 0) || (hunk.new_count > 0 && hunk.new_start == 0) {
        return Err("invalid hunk line range".into());
    }
    let (mut old, mut new) = (0, 0);
    let mut i = start + 1;
    loop {
        if old == hunk.old_count && new == hunk.new_count {
            while i < lines.len() && control(lines[i]) == "\\ No newline at end of file" {
                hunk.lines
                    .last_mut()
                    .ok_or("newline marker without a hunk line")?
                    .no_newline = true;
                i += 1;
            }
            return Ok((hunk, i));
        }
        let Some(body) = lines.get(i) else {
            return Err("hunk body is shorter than its header".into());
        };
        i += 1;
        if control(body) == "\\ No newline at end of file" {
            hunk.lines
                .last_mut()
                .ok_or("newline marker without a hunk line")?
                .no_newline = true;
            continue;
        }
        if !body.starts_with([' ', '+', '-']) {
            return Err("malformed hunk body".into());
        }
        let kind = body.as_bytes()[0];
        if kind != b'+' {
            old += 1;
        }
        if kind != b'-' {
            new += 1;
        }
        if old > hunk.old_count || new > hunk.new_count {
            return Err("hunk body is longer than its header".into());
        }
        hunk.lines.push(Line {
            kind,
            text: body[1..].into(),
            no_newline: false,
        });
    }
}
