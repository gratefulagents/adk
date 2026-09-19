use super::{Hunk, MAX_FILE};

fn bounded(result: String) -> Result<String, String> {
    if result.len() > MAX_FILE {
        return Err(format!(
            "patched file is too large ({} bytes, limit {MAX_FILE})",
            result.len()
        ));
    }
    Ok(result)
}
pub(super) fn apply(content: &str, hunks: &[Hunk]) -> Result<String, String> {
    if hunks.is_empty() {
        return Ok(content.into());
    }
    if hunks[0].range_less {
        return range_less(content, hunks);
    }
    let starts: Vec<_> = content
        .split_inclusive('\n')
        .scan(0, |start, line| {
            let result = *start;
            *start += line.len();
            Some(result)
        })
        .collect();
    let mut cursor = 0;
    let mut delta: i64 = 0;
    let mut result = String::new();
    for hunk in hunks {
        let mut position = hunk.old_start;
        if hunk.old_count > 0 {
            position -= 1;
            if position < 0
                || position
                    .checked_add(hunk.old_count)
                    .is_none_or(|n| n > starts.len() as i64)
            {
                return Err("hunk line range does not exist".into());
            }
        } else if position < 0 || position > starts.len() as i64 {
            return Err("hunk insertion line range does not exist".into());
        }
        let expected = position + delta + i64::from(hunk.new_count > 0);
        if hunk.new_start != expected {
            return Err(format!(
                "hunk new-file range is inconsistent: got {}, want {expected}",
                hunk.new_start
            ));
        }
        let start = starts
            .get(position as usize)
            .copied()
            .unwrap_or(content.len());
        let mut old = String::new();
        let mut new = String::new();
        for line in &hunk.lines {
            if line.kind != b'+' {
                old.push_str(&line.text);
                if !line.no_newline {
                    old.push('\n');
                }
            }
            if line.kind != b'-' {
                new.push_str(&line.text);
                if !line.no_newline {
                    new.push('\n');
                }
            }
        }
        if start < cursor {
            return Err("overlapping hunks".into());
        }
        if !content[start..].starts_with(&old) {
            return Err("hunk does not match file content".into());
        }
        result.push_str(&content[cursor..start]);
        result.push_str(&new);
        cursor = start + old.len();
        delta += hunk.new_count - hunk.old_count;
    }
    result.push_str(&content[cursor..]);
    bounded(result)
}
fn next_end(content: &str, start: usize) -> usize {
    content[start..]
        .find('\n')
        .map_or(content.len(), |n| start + n + 1)
}
fn text(line: &str) -> &str {
    if let Some(line) = line.strip_suffix('\n') {
        line.strip_suffix('\r').unwrap_or(line)
    } else {
        line
    }
}
fn hash(line: &str) -> u64 {
    line.bytes().fold(14695981039346656037, |hash, b| {
        (hash ^ u64::from(b)).wrapping_mul(1099511628211)
    })
}
fn sequence(content: &str, mut start: usize, lines: &[&str], eof: bool) -> Result<usize, String> {
    let pattern: Vec<_> = lines.iter().map(|line| (hash(line), *line)).collect();
    let mut prefix = vec![0; pattern.len()];
    let mut matched = 0;
    for i in 1..pattern.len() {
        while matched > 0 && pattern[i] != pattern[matched] {
            matched = prefix[matched - 1];
        }
        if pattern[i] == pattern[matched] {
            matched += 1;
        }
        prefix[i] = matched;
    }
    let mut starts = vec![0; pattern.len()];
    let mut found = None;
    matched = 0;
    let mut line = 0;
    while start < content.len() {
        let end = next_end(content, start);
        let value = text(&content[start..end]);
        let value = (hash(value), value);
        while matched > 0 && value != pattern[matched] {
            matched = prefix[matched - 1];
        }
        if value == pattern[matched] {
            matched += 1;
        }
        starts[line % pattern.len()] = start;
        line += 1;
        if matched == pattern.len() {
            if !eof || end == content.len() {
                if found.is_some() {
                    return Err("range-less hunk matches file content more than once".into());
                }
                found = Some(starts[(line - pattern.len()) % pattern.len()]);
            }
            matched = prefix[matched - 1];
        }
        start = end;
    }
    found.ok_or_else(|| "range-less hunk does not match file content".into())
}
struct Edit {
    start: usize,
    end: usize,
    new: String,
}
fn range_less(content: &str, hunks: &[Hunk]) -> Result<String, String> {
    let ending = if content
        .find('\n')
        .is_some_and(|n| n > 0 && content.as_bytes()[n - 1] == b'\r')
    {
        "\r\n"
    } else {
        "\n"
    };
    let mut edits = Vec::new();
    let mut hunk_cursor = 0;
    let mut changed_cursor = 0;
    for hunk in hunks {
        if !hunk.range_less {
            return Err("cannot mix range-less and unified hunks".into());
        }
        let old: Vec<_> = hunk
            .lines
            .iter()
            .filter(|l| l.kind != b'+')
            .map(|l| l.text.as_str())
            .collect();
        if old.is_empty() {
            if !content.is_empty() || hunks.len() != 1 {
                return Err("range-less hunk has no context or removed lines".into());
            }
            let mut new = String::new();
            for line in &hunk.lines {
                if line.kind != b'-' {
                    new.push_str(&line.text);
                    new.push_str(ending);
                }
            }
            edits.push(Edit {
                start: 0,
                end: 0,
                new,
            });
            continue;
        }
        let mut search = hunk_cursor;
        if !hunk.locator.is_empty() {
            let mut found = false;
            while search < content.len() {
                let end = next_end(content, search);
                if text(&content[search..end]) == hunk.locator {
                    search = end;
                    found = true;
                    break;
                }
                search = end;
            }
            if !found {
                return Err("range-less hunk locator does not match file content".into());
            }
        }
        let matched = sequence(content, search, &old, hunk.eof)?;
        let mut position = matched;
        let mut i = 0;
        while i < hunk.lines.len() {
            if hunk.lines[i].kind == b' ' {
                position = next_end(content, position);
                i += 1;
                continue;
            }
            let mut edit = Edit {
                start: position,
                end: position,
                new: String::new(),
            };
            let mut added = false;
            while i < hunk.lines.len() && hunk.lines[i].kind != b' ' {
                let line = &hunk.lines[i];
                if line.kind == b'-' {
                    edit.end = next_end(content, edit.end);
                    position = edit.end;
                } else {
                    edit.new.push_str(&line.text);
                    edit.new.push_str(ending);
                    added = true;
                }
                i += 1;
            }
            if edit.start == content.len() && edit.start > 0 && !content.ends_with('\n') && added {
                edit.new.insert_str(0, ending);
            }
            if edit.end == content.len() && edit.end > 0 && !content.ends_with('\n') && added {
                edit.new.truncate(edit.new.len() - ending.len());
            }
            if edit.start < changed_cursor {
                return Err("overlapping range-less hunks".into());
            }
            changed_cursor = edit.end;
            edits.push(edit);
        }
        hunk_cursor = matched;
    }
    let mut result = String::new();
    let mut cursor = 0;
    for edit in edits {
        result.push_str(&content[cursor..edit.start]);
        result.push_str(&edit.new);
        cursor = edit.end;
    }
    result.push_str(&content[cursor..]);
    bounded(result)
}
