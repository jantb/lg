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
    if matches_csharp_path(path)
        && let Some(symbol) = infer_source_symbol(path, line, csharp_item_label)
    {
        return symbol;
    }
    if matches_markdown_path(path)
        && let Some(symbol) = infer_source_symbol(path, line, markdown_item_label)
    {
        return symbol;
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

pub(super) fn matches_csharp_path(path: &str) -> bool {
    path.ends_with(".cs") || path.ends_with(".csx")
}

pub(super) fn matches_markdown_path(path: &str) -> bool {
    path.ends_with(".md") || path.ends_with(".markdown")
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

pub(super) fn csharp_item_label(line: &str) -> Option<String> {
    let mut line = line;
    loop {
        let stripped = [
            "public ",
            "private ",
            "protected ",
            "internal ",
            "static ",
            "sealed ",
            "abstract ",
            "virtual ",
            "override ",
            "partial ",
            "async ",
            "readonly ",
            "unsafe ",
            "new ",
        ]
        .into_iter()
        .find_map(|modifier| line.strip_prefix(modifier));
        match stripped {
            Some(rest) => line = rest,
            None => break,
        }
    }
    for prefix in [
        "namespace ",
        "class ",
        "record struct ",
        "record ",
        "struct ",
        "interface ",
        "enum ",
        "delegate ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = csharp_name(rest)?;
            return Some(format!("{} {name}", prefix.trim_end()));
        }
    }
    csharp_method_name(line).map(|name| format!("method {name}"))
}

/// A C# method has no keyword to key off, so it is recognised by shape: a
/// return type, a name, then a parameter list on the same line.
fn csharp_method_name(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let head = line[..open].trim_end();
    if head.contains('=') || head.ends_with(',') {
        return None;
    }
    let name = head.rsplit(|c: char| c.is_whitespace()).next()?;
    let (name, generics) = name
        .split_once('<')
        .map_or((name, false), |(n, _)| (n, true));
    if name.is_empty() || !head.contains(char::is_whitespace) && !generics {
        return None;
    }
    let valid = name
        .chars()
        .all(|c| c == '_' || c == '.' || c.is_alphanumeric());
    valid.then(|| name.to_string())
}

fn csharp_name(rest: &str) -> Option<String> {
    let name = rest
        .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
        .next()
        .unwrap_or(rest)
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Markdown has no items, but its headings are the sections a reviewer thinks
/// in, so a change is reported against the heading it falls under.
pub(super) fn markdown_item_label(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let title = line[hashes..].trim();
    (!title.is_empty()).then(|| format!("section {title}"))
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
    let end = find_source_item_end(path, &lines, start)
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
        } else if matches_csharp_path(path) {
            csharp_item_label(trimmed).is_some()
        } else if matches_markdown_path(path) {
            markdown_item_label(trimmed).is_some()
        } else {
            false
        };
        if is_item {
            return Some(idx);
        }
    }
    None
}

fn find_source_item_end(path: &str, lines: &[&str], start: usize) -> Option<usize> {
    if matches_markdown_path(path) {
        return find_markdown_section_end(lines, start);
    }
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

/// A markdown section runs until the next heading, so braces mean nothing here.
fn find_markdown_section_end(lines: &[&str], start: usize) -> Option<usize> {
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| markdown_item_label(line.trim_start()).is_some())
        .map_or(lines.len(), |(idx, _)| idx);
    end.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_item_label_extracts_named_items() {
        assert_eq!(rust_item_label("fn render() {"), Some("fn render".into()));
        assert_eq!(
            rust_item_label("pub async fn build() -> Result<()> {"),
            Some("async fn build".into())
        );
        assert_eq!(
            rust_item_label("pub(crate) struct AppState {"),
            Some("struct AppState".into())
        );
        assert_eq!(rust_item_label("let x = 1;"), None);
    }

    #[test]
    fn csharp_item_label_reads_types_and_methods() {
        assert_eq!(
            csharp_item_label("public sealed class OrderService : IOrders {"),
            Some("class OrderService".into())
        );
        assert_eq!(
            csharp_item_label("public async Task<Order> LoadAsync(string id)"),
            Some("method LoadAsync".into())
        );
        assert_eq!(
            csharp_item_label("internal record struct Point(int X, int Y);"),
            Some("record struct Point".into())
        );
        assert_eq!(csharp_item_label("var total = Sum(values);"), None);
    }

    #[test]
    fn markdown_item_label_reads_headings() {
        assert_eq!(
            markdown_item_label("## Getting started"),
            Some("section Getting started".into())
        );
        assert_eq!(markdown_item_label("just a paragraph"), None);
    }

    #[test]
    fn markdown_context_covers_the_section_the_change_falls_in() {
        let dir = std::env::temp_dir().join(format!("lg-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guide.md");
        std::fs::write(
            &path,
            "# Title\nintro\n\n## Setup\nstep one\nstep two\n\n## Usage\nrun it\n",
        )
        .unwrap();
        let path = path.to_string_lossy().to_string();

        let context = source_context(&path, 5);
        let text = context.join("\n");

        assert!(text.contains("## Setup"), "{text}");
        assert!(text.contains("step two"), "{text}");
        assert!(!text.contains("## Usage"), "{text}");
        assert_eq!(infer_entry_symbol(&path, 5, "@@ -1 +1 @@"), "section Setup");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kotlin_item_label_strips_visibility_and_modifier_prefixes() {
        assert_eq!(
            kotlin_item_label("private fun handle(): Int {"),
            Some("fun handle".into())
        );
        assert_eq!(
            kotlin_item_label("data class Point(val x: Int)"),
            Some("data class Point".into())
        );
        assert_eq!(
            kotlin_item_label("companion object {"),
            Some("companion object".into())
        );
        assert_eq!(kotlin_item_label("val n = 1"), None);
    }
}
