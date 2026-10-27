//! Turning a raw reply into the text the task asked for.

use super::think::strip_think_tags;

const REVIEW_CHAT_MAX_CHARS: usize = 12_000;

/// Pulls the reported language and every reported shape out of the reply, in the
/// order given. A missing or malformed line is skipped, so the caller keeps
/// whatever it already had.
pub fn parse_conventions(raw: &str) -> (Option<String>, Vec<String>) {
    let mut language = None;
    let mut shapes: Vec<String> = Vec::new();
    for line in strip_think_tags(raw).lines() {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = trim_outer_quotes(value.trim()).trim().to_string();
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "language" if language.is_none() => language = Some(value),
            "shape" | "message shape" | "style" | "comment_style" | "comment style"
                if !shapes
                    .iter()
                    .any(|shape| shape.eq_ignore_ascii_case(&value)) =>
            {
                shapes.push(value);
            }
            _ => {}
        }
    }
    (language, shapes)
}

pub fn finalize(raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let mut lines: Vec<String> = cleaned
        .trim()
        .trim_matches('"')
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("```"))
        .map(trim_outer_quotes)
        .map(str::to_string)
        .collect();

    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

pub fn finalize_review_chat(raw: &str) -> String {
    strip_think_tags(raw)
        .trim()
        .trim_matches('"')
        .lines()
        .map(trim_outer_quotes_without_backticks)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(REVIEW_CHAT_MAX_CHARS)
        .collect()
}

/// The merged lines the model wrote, with everything it wrapped them in taken
/// off: reasoning, a code fence, and the blank lines around them.
///
/// Indentation inside the block is left exactly as it came, because it is the
/// answer rather than formatting around it.
pub fn finalize_conflict_hunk(raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let mut lines: Vec<&str> = cleaned.lines().collect();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines
        .first()
        .is_some_and(|line| line.trim_start().starts_with("```"))
        && lines.last().is_some_and(|line| line.trim() == "```")
    {
        lines.remove(0);
        lines.pop();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn trim_outer_quotes(s: &str) -> &str {
    s.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
}

fn trim_outer_quotes_without_backticks(s: &str) -> &str {
    s.trim().trim_matches('"').trim_matches('\'')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conflict_resolution_keeps_its_indentation_and_loses_its_wrapping() {
        let raw = "<think>both sides add a field</think>\n\n```rust\n    let a = 1;\n        let b = 2;\n```\n\n";

        assert_eq!(
            finalize_conflict_hunk(raw),
            "    let a = 1;\n        let b = 2;\n"
        );
    }

    #[test]
    fn a_conflict_resolution_that_is_only_a_fence_ends_up_empty() {
        assert_eq!(finalize_conflict_hunk("\n\n"), "");
    }

    /// A resolution that deletes both sides is a real answer, and the splice
    /// needs it to stay empty rather than becoming a blank line.
    #[test]
    fn a_conflict_resolution_that_deletes_everything_stays_empty() {
        assert_eq!(finalize_conflict_hunk("<think>drop it</think>"), "");
    }

    #[test]
    fn a_conflict_resolution_without_a_fence_is_taken_as_written() {
        assert_eq!(finalize_conflict_hunk("CANNOT RESOLVE"), "CANNOT RESOLVE\n");
    }

    #[test]
    fn conventions_reply_is_parsed_into_language_and_style() {
        let (language, shapes) = parse_conventions(
            "<think>hm</think>\nlanguage: Norwegian\nstyle: \"terse, imperative, no filler\"\n",
        );
        assert_eq!(language.as_deref(), Some("Norwegian"));
        assert_eq!(shapes, vec!["terse, imperative, no filler".to_string()]);
    }

    #[test]
    fn a_reply_without_the_expected_lines_yields_nothing() {
        let (language, shapes) = parse_conventions("I could not tell.");
        assert!(language.is_none());
        assert!(shapes.is_empty());
    }

    #[test]
    fn finalize_strips_quotes_and_keeps_overflow() {
        assert_eq!(finalize("  \"feat: add\"  "), "feat: add");
        let long = "x".repeat(200);
        assert_eq!(finalize(&long), long);
    }

    #[test]
    fn finalize_keeps_long_subject_without_cutting_it_off() {
        assert_eq!(
            finalize(
                "feat(tui): show a longer generated message that needs extra detail instead of being cut off"
            ),
            "feat(tui): show a longer generated message that needs extra detail instead of being cut off"
        );
    }

    #[test]
    fn finalize_preserves_body_layout() {
        assert_eq!(
            finalize(
                "feat(tui): show active generation state\n\nAdds status counts.\nKeeps focused panels visible.\nKeeps the modal useful for longer messages.\nAvoids cutting off generated context.\nExtra line ignored."
            ),
            "feat(tui): show active generation state\n\nAdds status counts.\nKeeps focused panels visible.\nKeeps the modal useful for longer messages.\nAvoids cutting off generated context.\nExtra line ignored."
        );
    }

    /// The commit path is what the stray tag was reaching, so pin it there too.
    #[test]
    fn finalize_drops_the_draft_a_bare_close_ends() {
        assert_eq!(
            finalize("fix: draft subject\n</think>\nfix: real subject\n"),
            "fix: real subject"
        );
    }

    #[test]
    fn conventions_reply_yields_the_language_and_the_shape() {
        let (language, shapes) = parse_conventions(
            "language: Norwegian\n\
             shape: lowercase imperative subject, no prefix, bullet body\n\
             shape: Conventional Commits subject, no body\n\
             shape: lowercase imperative subject, no prefix, bullet body\n",
        );

        assert_eq!(language.as_deref(), Some("Norwegian"));
        assert_eq!(
            shapes,
            vec![
                "lowercase imperative subject, no prefix, bullet body".to_string(),
                "Conventional Commits subject, no body".to_string(),
            ]
        );
    }
}
