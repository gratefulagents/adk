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
