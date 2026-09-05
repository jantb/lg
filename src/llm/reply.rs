//! Turning a raw reply into the text the task asked for.

use crate::state::{ReviewStyleFinding, ReviewStyleSeverity};

use super::prompt::review_style_file_role;
use super::think::strip_think_tags;

const REVIEW_ASSIST_MAX_CHARS: usize = 16_000;
const REVIEW_ASSIST_MAX_LINES: usize = 128;
const REVIEW_PR_MAX_CHARS: usize = 8_000;
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

pub fn finalize_review_assist(raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let mut lines = Vec::new();
    for line in cleaned
        .trim()
        .trim_matches('"')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("```"))
    {
        lines.push(trim_outer_quotes(line).to_string());
        if lines.len() >= REVIEW_ASSIST_MAX_LINES {
            break;
        }
    }
    lines
        .join("\n")
        .chars()
        .take(REVIEW_ASSIST_MAX_CHARS)
        .collect()
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

pub fn finalize_review_pr_text(raw: &str) -> String {
    strip_think_tags(raw)
        .trim()
        .trim_matches('"')
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().starts_with("```"))
        .map(trim_outer_quotes_without_backticks)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .chars()
        .take(REVIEW_PR_MAX_CHARS)
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

pub fn finalize_review_style_flag_for_path(path: &str, raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let mut finding = parse_review_style_finding(&cleaned);
    suppress_layer_misclassification(path, &mut finding);
    format_review_style_finding(&finding)
}

fn format_review_style_finding(finding: &ReviewStyleFinding) -> String {
    format!(
        "severity: {}\nline: {}\nreason: {}",
        finding.severity.label(),
        finding
            .line
            .map(|line| line.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        finding.reason
    )
}

fn suppress_layer_misclassification(path: &str, finding: &mut ReviewStyleFinding) {
    // A layer complaint is wrong about a service or flow file, which is allowed
    // to hold business rules, and equally wrong about a file whose layering lg
    // cannot read at all — there is no layer for it to be in the wrong one of.
    if !matches!(
        review_style_file_role(path),
        "service-layer" | "flow" | "unclassified"
    ) {
        return;
    }
    if !matches!(
        finding.severity,
        ReviewStyleSeverity::Warn | ReviewStyleSeverity::Fail
    ) {
        return;
    }
    let reason = finding.reason.to_ascii_lowercase();
    let calls_out_wrong_layer = reason.contains("non-service")
        || reason.contains("non service")
        || reason.contains("service layer")
        || reason.contains("direct repository call")
        || reason.contains("repository call");
    if calls_out_wrong_layer && !reason_mentions_other_style_rule(&reason) {
        finding.severity = ReviewStyleSeverity::Ok;
        finding.line = None;
        finding.reason = "No style issue found.".to_string();
    }
}

fn reason_mentions_other_style_rule(reason: &str) -> bool {
    [
        "kafka",
        "jackson",
        "java.time",
        "mockito",
        "loggerfactory",
        "ktor",
        "generated",
        "var ",
        "mutable",
    ]
    .iter()
    .any(|needle| reason.contains(needle))
}

/// The text after `key:` on a line that opens with it, matched against
/// `lower` so the key is case-insensitive while the value keeps its case.
fn field_value<'a>(line: &'a str, lower: &str, key: &str) -> Option<&'a str> {
    lower
        .starts_with(key)
        .then(|| line.split_once(':'))?
        .map(|(_, value)| value)
}

pub fn parse_review_style_finding(raw: &str) -> ReviewStyleFinding {
    let mut severity = None;
    let mut line_number = None;
    let mut reason = None;
    for line in raw
        .trim()
        .trim_matches('"')
        .lines()
        .map(clean_review_style_line)
        .filter(|line| !line.is_empty())
    {
        let lower = line.to_ascii_lowercase();
        if let Some(value) = field_value(line, &lower, "severity:") {
            if let Some(parsed) = parse_review_style_severity(value) {
                severity = Some(parsed);
                reason = None;
            }
        } else if let Some(value) = field_value(line, &lower, "line:") {
            line_number = parse_review_style_line_number(value);
        } else if let Some(value) = field_value(line, &lower, "reason:") {
            if severity.is_some() {
                reason = Some(value.trim().to_string());
            }
        } else if severity.is_none() {
            severity = parse_review_style_severity(line);
        } else if reason.is_none() {
            reason = Some(line.to_string());
        }
    }
    let severity = severity.unwrap_or(ReviewStyleSeverity::Ok);
    let reason = reason
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or_else(|| "No style issue found.".to_string());
    ReviewStyleFinding {
        severity,
        line: line_number,
        reason,
    }
}

fn clean_review_style_line(line: &str) -> &str {
    line.trim()
        .trim_start_matches(|ch: char| {
            ch.is_ascii_whitespace()
                || ch == '-'
                || ch == '*'
                || ch == '>'
                || ch == '`'
                || ch == '"'
                || ch == '\''
        })
        .trim_end_matches(['`', '"', '\''])
        .trim()
}

fn parse_review_style_severity(s: &str) -> Option<ReviewStyleSeverity> {
    let upper = s
        .trim()
        .trim_matches(|ch: char| {
            ch.is_ascii_whitespace()
                || ch == '`'
                || ch == '"'
                || ch == '\''
                || ch == '.'
                || ch == ':'
        })
        .to_ascii_uppercase();
    match upper.as_str() {
        "FAIL" | "RED" => Some(ReviewStyleSeverity::Fail),
        "WARN" | "WARNING" | "FLAG" => Some(ReviewStyleSeverity::Warn),
        "OK" | "GREEN" => Some(ReviewStyleSeverity::Ok),
        _ => None,
    }
}

fn parse_review_style_line_number(s: &str) -> Option<usize> {
    let value = s.trim();
    if value.eq_ignore_ascii_case("unknown") || value.eq_ignore_ascii_case("n/a") {
        return None;
    }
    value
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse().ok())
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
    use crate::llm::REVIEW_ASSIST_NUM_PREDICT;

    /// The path-independent half of [`finalize_review_style_flag_for_path`].
    fn finalize_review_style_flag(raw: &str) -> String {
        let cleaned = strip_think_tags(raw);
        let finding = parse_review_style_finding(&cleaned);
        format_review_style_finding(&finding)
    }

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
    fn review_assist_finalizer_keeps_deeper_analysis() {
        let raw = (0..48)
            .map(|idx| format!("- finding {idx}"))
            .collect::<Vec<_>>()
            .join("\n");

        let finalized = finalize_review_assist(&raw);

        assert!(finalized.contains("- finding 0"));
        assert!(finalized.contains("- finding 47"));
    }

    #[test]
    fn review_assist_finalizer_allows_sixteen_k_output() {
        let finalized = finalize_review_assist(&"x".repeat(20_000));

        assert_eq!(finalized.chars().count(), REVIEW_ASSIST_MAX_CHARS);
        assert_eq!(REVIEW_ASSIST_MAX_CHARS, 16_000);
        assert_eq!(REVIEW_ASSIST_NUM_PREDICT, 16_000);
    }

    #[test]
    fn finalize_review_pr_text_preserves_markdown_without_fences() {
        let finalized = finalize_review_pr_text(
            "<think>ignore</think>\n```markdown\n## Summary\n- Keeps markdown.\n```\n",
        );

        assert_eq!(finalized, "## Summary\n- Keeps markdown.");
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

    #[test]
    fn finalize_review_style_flag_normalizes_output() {
        assert_eq!(
            finalize_review_style_flag(
                "severity: WARN\nline: 42\nreason: controller does too much"
            ),
            "severity: WARN\nline: 42\nreason: controller does too much"
        );
        assert_eq!(
            finalize_review_style_flag("FAIL\nDirect Kafka publish"),
            "severity: FAIL\nline: unknown\nreason: Direct Kafka publish"
        );
        assert_eq!(
            finalize_review_style_flag("not enough evidence"),
            "severity: OK\nline: unknown\nreason: No style issue found."
        );
    }

    #[test]
    fn service_file_style_finalizer_suppresses_layer_false_positive() {
        assert_eq!(
            finalize_review_style_flag_for_path(
                "src/main/kotlin/CompletePendingTransactionService.kt",
                "severity: FAIL\n\
                 line: 192\n\
                 reason: Direct repository call in non-Service/non-flow file; business logic belongs in service layer.",
            ),
            "severity: OK\nline: unknown\nreason: No style issue found."
        );
    }

    #[test]
    fn service_file_style_finalizer_keeps_unrelated_violations() {
        assert_eq!(
            finalize_review_style_flag_for_path(
                "src/main/kotlin/CompletePendingTransactionService.kt",
                "severity: FAIL\n\
                 line: 210\n\
                 reason: Direct Kafka publish should go through the outbox.",
            ),
            "severity: FAIL\nline: 210\nreason: Direct Kafka publish should go through the outbox."
        );
    }

    #[test]
    fn review_style_parser_ignores_schema_when_reading_reasoning_fallback() {
        let finding = parse_review_style_finding(
            "Return exactly three lines:\n\
             `severity: OK|WARN|FAIL`\n\
             `line: <new-file line number, or unknown>`\n\
             `reason: <one concise reason>`\n\n\
             Draft output:\n\
             severity: FAIL\n\
             line: 273\n\
             reason: Controller contains business logic instead of delegating to a service.",
        );

        assert_eq!(finding.severity, ReviewStyleSeverity::Fail);
        assert_eq!(finding.line, Some(273));
        assert_eq!(
            finding.reason,
            "Controller contains business logic instead of delegating to a service."
        );
    }
}
