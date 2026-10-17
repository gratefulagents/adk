use crate::search::go_string;
use std::io::Write;

struct Run {
    first: usize,
    last: usize,
    offsets: Vec<usize>,
}
struct Lines<'a> {
    content: &'a [u8],
    starts: Vec<usize>,
}
impl Lines<'_> {
    fn end(&self, line: usize) -> usize {
        self.starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.content.len())
    }
    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|start| *start <= offset) - 1
    }
    fn text(&self, line: usize) -> &[u8] {
        let bytes = &self.content[self.starts[line]..self.end(line)];
        bytes.strip_suffix(b"\n").unwrap_or(bytes)
    }
    fn replacement(&self, run: &Run, old: &[u8], new: &[u8]) -> Vec<u8> {
        let mut result = Vec::new();
        let mut position = self.starts[run.first];
        for offset in &run.offsets {
            result.extend_from_slice(&self.content[position..*offset]);
            result.extend_from_slice(new);
            position = offset + old.len();
        }
        result.extend_from_slice(&self.content[position..self.end(run.last)]);
        result
    }
}
fn append_line(output: &mut Vec<u8>, prefix: u8, bytes: &[u8]) {
    output.push(prefix);
    output.extend_from_slice(bytes);
    output.push(b'\n');
}

pub(crate) fn render(
    content: &[u8],
    old: &[u8],
    new: &[u8],
    offsets: &[usize],
    mut truncated: bool,
) -> String {
    if offsets.is_empty() {
        return String::new();
    }
    let mut starts = vec![0];
    starts.extend(
        memchr::memchr_iter(b'\n', content)
            .map(|i| i + 1)
            .filter(|i| *i < content.len()),
    );
    let lines = Lines { content, starts };
    let mut runs: Vec<Run> = Vec::new();
    for offset in offsets {
        let first = lines.line_of(*offset);
        let last = lines.line_of(offset + old.len() - 1);
        if let Some(run) = runs.last_mut().filter(|run| first <= run.last) {
            run.last = run.last.max(last);
            run.offsets.push(*offset);
        } else {
            runs.push(Run {
                first,
                last,
                offsets: vec![*offset],
            });
        }
        let run = runs.last_mut().expect("just inserted");
        while run.last + 1 < lines.starts.len() {
            let text = lines.replacement(run, old, new);
            if text.is_empty() || text.ends_with(b"\n") {
                break;
            }
            run.last += 1;
        }
    }
    let mut result = Vec::new();
    let mut delta = 0isize;
    let mut first_run = 0;
    while first_run < runs.len() {
        if result.len() > 8192 {
            truncated = true;
            break;
        }
        let mut end_run = first_run + 1;
        while end_run < runs.len() && runs[end_run].first - runs[end_run - 1].last - 1 <= 6 {
            end_run += 1;
        }
        let hunk = &runs[first_run..end_run];
        let start = hunk[0].first.saturating_sub(3);
        let end = (hunk.last().expect("hunk").last + 3).min(lines.starts.len() - 1);
        let mut body = Vec::new();
        let (mut old_count, mut new_count) = (0, 0);
        let mut cursor = start;
        for run in hunk {
            for line in cursor..run.first {
                append_line(&mut body, b' ', lines.text(line));
                old_count += 1;
                new_count += 1;
            }
            for line in run.first..=run.last {
                append_line(&mut body, b'-', lines.text(line));
                old_count += 1;
            }
            let text = lines.replacement(run, old, new);
            if !text.is_empty() {
                let mut split = text.split(|b| *b == b'\n').peekable();
                while let Some(line) = split.next() {
                    if line.is_empty() && split.peek().is_none() {
                        break;
                    }
                    append_line(&mut body, b'+', line);
                    new_count += 1;
                }
            }
            cursor = run.last + 1;
        }
        for line in cursor..=end {
            append_line(&mut body, b' ', lines.text(line));
            old_count += 1;
            new_count += 1;
        }
        let new_start = start as isize + 1 + delta - isize::from(new_count == 0);
        writeln!(
            result,
            "@@ -{},{} +{},{} @@",
            start + 1,
            old_count,
            new_start,
            new_count
        )
        .expect("vector write");
        result.extend_from_slice(&body);
        delta += new_count as isize - old_count as isize;
        first_run = end_run;
    }
    if result.len() > 8192 {
        let cut = result[..8192]
            .iter()
            .rposition(|b| *b == b'\n')
            .filter(|i| *i > 0)
            .unwrap_or(8192);
        result.truncate(cut);
        truncated = true;
    }
    if result.ends_with(b"\n") {
        result.pop();
    }
    if truncated {
        result.extend_from_slice(b"\n... [diff truncated]");
    }
    go_string(&result)
}

#[cfg(test)]
mod tests {
    use super::render;

    fn diff(content: &str, old: &str, new: &str, replace_all: bool) -> String {
        let count = memchr::memmem::find_iter(content.as_bytes(), old.as_bytes()).count();
        let offsets: Vec<_> = memchr::memmem::find_iter(content.as_bytes(), old.as_bytes())
            .take(if replace_all { 256 } else { 1 })
            .collect();
        render(
            content.as_bytes(),
            old.as_bytes(),
            new.as_bytes(),
            &offsets,
            replace_all && count > 256,
        )
    }

    #[test]
    fn pinned_edit_diff_line_and_join_examples() {
        for (name, content, old, new, all, expected) in [
            (
                "single",
                "a\nb\nc\nd\ne\nf\ng\nh\n",
                "d",
                "D",
                false,
                "@@ -1,7 +1,7 @@\n a\n b\n c\n-d\n+D\n e\n f\n g",
            ),
            (
                "multiline",
                "a\nb\nc\nd\ne\nf\ng\nh\n",
                "c\nd",
                "X",
                false,
                "@@ -1,7 +1,6 @@\n a\n b\n-c\n-d\n+X\n e\n f\n g",
            ),
            (
                "partial",
                "func foo() {\n\treturn oldValue\n}\n",
                "oldValue",
                "newValue",
                false,
                "@@ -1,3 +1,3 @@\n func foo() {\n-\treturn oldValue\n+\treturn newValue\n }",
            ),
            (
                "nearby",
                "a\ntarget\nb\nc\ntarget\nd\n",
                "target",
                "T",
                true,
                "@@ -1,6 +1,6 @@\n a\n-target\n+T\n b\n c\n-target\n+T\n d",
            ),
            (
                "same line",
                "foo bar foo\nother\n",
                "foo",
                "qux",
                true,
                "@@ -1,2 +1,2 @@\n-foo bar foo\n+qux bar qux\n other",
            ),
            (
                "deletion",
                "a\nb\nc\n",
                "b\n",
                "",
                false,
                "@@ -1,3 +1,2 @@\n a\n-b\n c",
            ),
            (
                "join",
                "foo\nbar\n",
                "foo\n",
                "baz",
                false,
                "@@ -1,2 +1,1 @@\n-foo\n-bar\n+bazbar",
            ),
            (
                "keep newline",
                "foo\nbar\n",
                "foo\n",
                "baz\n",
                false,
                "@@ -1,2 +1,2 @@\n-foo\n+baz\n bar",
            ),
            (
                "prefix join",
                "a\nxfoo\nbar\nc\n",
                "foo\n",
                "",
                false,
                "@@ -1,4 +1,3 @@\n a\n-xfoo\n-bar\n+xbar\n c",
            ),
            (
                "cascade",
                "a\na\nb\n",
                "a\n",
                "A",
                true,
                "@@ -1,3 +1,1 @@\n-a\n-a\n-b\n+AAb",
            ),
            (
                "EOF",
                "foo\nbar\n",
                "bar\n",
                "B",
                false,
                "@@ -1,2 +1,2 @@\n foo\n-bar\n+B",
            ),
            (
                "append",
                "a\nb",
                "b",
                "b\nc",
                false,
                "@@ -1,2 +1,3 @@\n a\n-b\n+b\n+c",
            ),
            ("no match", "a\nb\n", "missing", "x", false, ""),
        ] {
            assert_eq!(diff(content, old, new, all), expected, "{name}");
        }
    }

    #[test]
    fn pinned_edit_diff_distant_hunks_and_line_number_shifts() {
        let mut lines = ["line"; 30];
        lines[4] = "target";
        lines[24] = "target";
        let content = lines.join("\n") + "\n";
        let output = diff(&content, "target", "changed", true);
        assert_eq!(output.matches("@@").count(), 4);
        assert!(output.contains("@@ -2,7 +2,7 @@"));
        assert!(output.contains("@@ -22,7 +22,7 @@"));
        assert_eq!(output.matches("-target").count(), 2);
        assert_eq!(output.matches("+changed").count(), 2);
        let content = content.replace("target", "first");
        let output = diff(&content, "first", "first\nadded\nadded", true);
        assert!(output.contains("@@ -2,7 +2,9 @@"));
        assert!(output.contains("@@ -22,7 +24,9 @@"));
    }

    #[test]
    fn empty_new_ranges_and_match_cap_boundaries() {
        for content in ["x", "x\n"] {
            assert_eq!(diff(content, content, "", false), "@@ -1,1 +0,0 @@\n-x");
        }
        assert_eq!(diff("\n", "\n", "", false), "@@ -1,1 +0,0 @@\n-");
        assert_eq!(diff("\n", "\n", "\n\n", false), "@@ -1,1 +1,2 @@\n-\n+\n+");
        for count in [256, 257] {
            let output = diff(&"x\n".repeat(count), "x", "y", true);
            assert_eq!(output.matches("-x").count(), 256);
            assert_eq!(output.matches("+y").count(), 256);
            assert_eq!(output.ends_with("... [diff truncated]"), count == 257);
        }
    }

    #[test]
    fn pinned_edit_diff_truncation_caps() {
        let content = format!("padding {}\ntarget\n", "x".repeat(70)).repeat(500);
        let output = diff(&content, "target", "changed", true);
        assert!(output.len() <= 8192 + "... [diff truncated]".len() + 1);
        assert!(output.ends_with("... [diff truncated]"));
    }
}
