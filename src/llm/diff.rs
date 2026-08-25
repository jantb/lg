//! Reducing a patch to the parts that fit in a prompt.

const MAX_DIFF_EXCERPT_LINES: usize = 180;
const MAX_DIFF_EXCERPT_BYTES: usize = 16_000;
const MAX_SUMMARY_FILES: usize = 24;
const MAX_SIGNAL_LINES: usize = 48;

#[derive(Default)]
struct DiffFileSummary {
    path: String,
    added: usize,
    removed: usize,
    hunks: Vec<String>,
}

pub fn summarize_diff(diff: &str) -> String {
    let mut files: Vec<DiffFileSummary> = Vec::new();
    let mut current: Option<usize> = None;
    let mut signals: Vec<String> = Vec::new();

    for line in diff.lines() {
        if let Some(path) = parse_diff_path(line) {
            files.push(DiffFileSummary {
                path,
                ..Default::default()
            });
            current = Some(files.len() - 1);
            continue;
        }

        if let Some(i) = current {
            if line.starts_with("@@") {
                if files[i].hunks.len() < 3 {
                    files[i].hunks.push(truncate_line(line, 90));
                }
            } else if line.starts_with('+') && !line.starts_with("+++") {
                files[i].added += 1;
                push_signal(&mut signals, '+', line);
            } else if line.starts_with('-') && !line.starts_with("---") {
                files[i].removed += 1;
                push_signal(&mut signals, '-', line);
            }
        }
    }

    if files.is_empty() {
        return "No textual diff was found.".to_owned();
    }

    let mut out = String::new();
    out.push_str("Files changed:\n");
    for file in files.iter().take(MAX_SUMMARY_FILES) {
        out.push_str("- ");
        out.push_str(&file.path);
        out.push_str(&format!(" (+{} -{})", file.added, file.removed));
        if !file.hunks.is_empty() {
            out.push_str("; hunks: ");
            out.push_str(&file.hunks.join(" | "));
        }
        out.push('\n');
    }
    if files.len() > MAX_SUMMARY_FILES {
        out.push_str(&format!(
            "- ... {} more files\n",
            files.len() - MAX_SUMMARY_FILES
        ));
    }

    if !signals.is_empty() {
        out.push_str("\nNotable changed lines:\n");
        for line in signals {
            out.push_str("- ");
            out.push_str(&line);
            out.push('\n');
        }
    }

    out
}

pub fn diff_excerpt(diff: &str) -> String {
    let mut out = String::new();
    let mut bytes = 0usize;

    for (lines, line) in diff
        .lines()
        .filter(|line| is_excerpt_line(line))
        .enumerate()
    {
        let len = line.len() + 1;
        if lines >= MAX_DIFF_EXCERPT_LINES || bytes + len > MAX_DIFF_EXCERPT_BYTES {
            out.push_str("... diff excerpt truncated ...\n");
            break;
        }
        out.push_str(line);
        out.push('\n');
        bytes += len;
    }

    if out.trim().is_empty() {
        diff.lines()
            .take(40)
            .map(|line| truncate_line(line, 120))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        out
    }
}

fn parse_diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_, b_path) = rest.split_once(" b/")?;
    Some(b_path.to_owned())
}

fn push_signal(signals: &mut Vec<String>, prefix: char, line: &str) {
    if signals.len() >= MAX_SIGNAL_LINES {
        return;
    }
    let body = line[1..].trim();
    if body.is_empty() || matches!(body, "{" | "}" | ");" | "," | ")" | "]" | "};") {
        return;
    }
    signals.push(format!("{prefix} {}", truncate_line(body, 110)));
}

fn is_excerpt_line(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("@@")
        || (line.starts_with('+') && !line.starts_with("+++"))
        || (line.starts_with('-') && !line.starts_with("---"))
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}
