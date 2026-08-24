pub mod author;
pub mod branches;
pub mod commit;
pub mod commits;
pub mod confirm;
pub mod conflict;
pub mod delete_branch;
pub mod environments;
pub mod files;
pub mod flow;
pub mod help;
pub mod keys;
pub mod main;
pub(crate) mod markdown;
pub mod model;
pub mod push;
pub mod review_chat;
pub(crate) mod scroll;
pub mod stage_all;
pub mod status;
pub mod worktree;

/// Splits `text` into chunks no wider than `width`, breaking on spaces so a
/// long value wraps instead of running off a modal's right edge. A word with
/// no break in it — a path, typically — is cut at the width rather than
/// allowed past it.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
        // A single word longer than the row is cut at the width, not past it.
        while current.chars().count() > width {
            let head: String = current.chars().take(width).collect();
            current = current.chars().skip(width).collect();
            lines.push(head);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
