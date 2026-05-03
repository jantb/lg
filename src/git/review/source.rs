pub(super) fn infer_entry_symbol(path: &str, line: usize, hunk: &str) -> String {
    if path.ends_with(".rs") {
        if let Some(symbol) = infer_rust_symbol(path, line) {
            return symbol;
        }
    }
    if matches_kotlin_path(path) {
        if let Some(symbol) = infer_kotlin_symbol(path, line) {
            return symbol;
        }
    }
    if let Some(symbol) = hunk_symbol(hunk) {
        return symbol;
    }
    "file scope".to_string()
}

fn hunk_symbol(hunk: &str) -> Option<String> {
    let symbol = hunk.rsplit("@@").next()?.trim();
    if symbol.is_empty()
        || symbol == "where"
        || symbol.starts_with("use ")
        || symbol.starts_with("impl ")
    {
        return None;
    }
    Some(super::truncate_review_text(symbol, 96))
}

fn infer_rust_symbol(path: &str, line: usize) -> Option<String> {
    infer_source_symbol(path, line, rust_item_label)
}

fn infer_kotlin_symbol(path: &str, line: usize) -> Option<String> {
    infer_source_symbol(path, line, kotlin_item_label)
}

fn infer_source_symbol(
    path: &str,
    line: usize,
    label: fn(&str) -> Option<String>,
) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let target = line.saturating_sub(1);
    let lines: Vec<&str> = text.lines().collect();
    let start = target.saturating_sub(160);
    for raw in lines
        .get(start..=target.min(lines.len().saturating_sub(1)))?
        .iter()
        .rev()
    {
        let trimmed = raw.trim_start();
        if let Some(symbol) = label(trimmed) {
            return Some(symbol);
        }
    }
    None
}

pub(super) fn matches_kotlin_path(path: &str) -> bool {
    path.ends_with(".kt") || path.ends_with(".kts")
}

pub(super) fn rust_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    for prefix in [
        "async fn ",
        "fn ",
        "impl ",
        "trait ",
        "struct ",
        "enum ",
        "mod ",
        "const ",
        "static ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
                .next()
                .unwrap_or(rest)
                .trim();
            if !name.is_empty() {
                return Some(format!("{} {name}", prefix.trim_end()));
            }
        }
    }
    None
}

pub(super) fn kotlin_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("private ")
        .or_else(|| line.strip_prefix("internal "))
        .or_else(|| line.strip_prefix("protected "))
        .or_else(|| line.strip_prefix("public "))
        .unwrap_or(line);
    let line = line
        .strip_prefix("suspend ")
        .or_else(|| line.strip_prefix("inline "))
        .unwrap_or(line);
    for prefix in [
        "fun ",
        "class ",
        "data class ",
        "sealed class ",
        "enum class ",
        "object ",
        "interface ",
        "companion object",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
                .next()
                .unwrap_or(rest)
                .trim();
            let label = prefix.trim_end();
            if prefix == "companion object" {
                return Some(label.to_string());
            }
            if !name.is_empty() {
                return Some(format!("{label} {name}"));
            }
        }
    }
    None
}

pub(super) fn source_context(path: &str, line: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let target = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    let start = find_source_item_start(path, &lines, target).unwrap_or(target.saturating_sub(8));
    let end = find_source_item_end(&lines, start)
        .unwrap_or_else(|| target.saturating_add(24).min(lines.len().saturating_sub(1)));

    lines[start..=end]
        .iter()
        .enumerate()
        .map(|(idx, text)| format!("{:>5} | {}", start + idx + 1, text))
        .collect()
}

fn find_source_item_start(path: &str, lines: &[&str], target: usize) -> Option<usize> {
    let start = target.saturating_sub(160);
    for (idx, raw) in lines.iter().enumerate().take(target + 1).skip(start).rev() {
        let trimmed = raw.trim_start();
        let is_item = if path.ends_with(".rs") {
            rust_item_label(trimmed).is_some()
        } else if matches_kotlin_path(path) {
            kotlin_item_label(trimmed).is_some()
        } else {
            false
        };
        if is_item {
            return Some(idx);
        }
    }
    None
}

fn find_source_item_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut balance = 0isize;
    let mut saw_open = false;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => {
                    balance += 1;
                    saw_open = true;
                }
                '}' => balance -= 1,
                _ => {}
            }
        }
        if saw_open && balance <= 0 {
            return Some(idx);
        }
        if !saw_open && idx > start && line.trim().is_empty() {
            return Some(idx.saturating_sub(1));
        }
    }
    (!lines.is_empty()).then_some(lines.len() - 1)
}
