//! Naming the item a change falls in, and the source around it.

use super::language::Language;
#[cfg(test)]
use super::language::{csharp_item_label, kotlin_item_label, markdown_item_label, rust_item_label};

pub(super) fn infer_entry_symbol(path: &str, line: usize, hunk: &str) -> String {
    if let Some(language) = Language::of_path(path)
        && let Some(symbol) = infer_source_symbol(path, line, language)
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

fn infer_source_symbol(path: &str, line: usize, language: Language) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let target = line.saturating_sub(1);
    let lines: Vec<&str> = text.lines().collect();
    let start = target.saturating_sub(160);
    lines
        .get(start..=target.min(lines.len().saturating_sub(1)))?
        .iter()
        .rev()
        .find_map(|raw| language.item_label(raw.trim_start()))
}

pub(super) fn source_context(path: &str, line: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let language = Language::of_path(path);
    let target = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    let start =
        find_source_item_start(language, &lines, target).unwrap_or(target.saturating_sub(8));
    let end = find_source_item_end(language, &lines, start)
        .unwrap_or_else(|| target.saturating_add(24).min(lines.len().saturating_sub(1)));

    lines[start..=end]
        .iter()
        .enumerate()
        .map(|(idx, text)| format!("{:>5} | {}", start + idx + 1, text))
        .collect()
}

fn find_source_item_start(
    language: Option<Language>,
    lines: &[&str],
    target: usize,
) -> Option<usize> {
    let language = language?;
    let start = target.saturating_sub(160);
    lines
        .iter()
        .enumerate()
        .take(target + 1)
        .skip(start)
        .rev()
        .find(|(_, raw)| language.item_label(raw.trim_start()).is_some())
        .map(|(idx, _)| idx)
}

fn find_source_item_end(language: Option<Language>, lines: &[&str], start: usize) -> Option<usize> {
    if language == Some(Language::Markdown) {
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
        .find(|(_, line)| Language::Markdown.item_label(line.trim_start()).is_some())
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
