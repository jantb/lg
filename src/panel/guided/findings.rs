//! The model's read of one step taken apart: the one-line summary, the
//! findings it pins to lines of the hunk, and whatever else it said. Findings
//! are drawn under the line they are about, so the pane beneath the hunk is
//! left with only what has no line to sit on.

use crate::git::guided::GuidedHunk;

/// Ordered by how much it matters, so the worst on a line is its `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Nit,
    Issue,
}

/// One thing the model flagged, on the line it cited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The line as the step prompt numbers it: the new file's number, or the
    /// old file's for a removed line.
    pub line: usize,
    pub severity: Severity,
    pub text: String,
}

/// A step's commentary, read in the shape the step prompt asks for. An
/// answer that ignored the shape lands whole in `rest`, so nothing is lost.
#[derive(Debug, Default)]
pub struct Reading {
    pub summary: Option<String>,
    pub findings: Vec<Finding>,
    /// Where to look next, when the answer depends on code not shown.
    pub check: Option<String>,
    /// What an earlier read flagged that the code now fixes.
    pub fixed: Vec<String>,
    pub rest: Vec<String>,
}

pub fn read(text: &str) -> Reading {
    let mut reading = Reading::default();
    for raw in text.lines() {
        let line = bare(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(summary) = after_key(line, "summary") {
            reading.summary = Some(summary.to_string());
        } else if let Some(fixed) = after_key(line, "fixed") {
            reading.fixed.push(fixed.to_string());
        } else if let Some(check) = after_key(line, "check") {
            reading.check = Some(check.to_string());
        } else if after_key(line, "verdict").is_some() {
            // The step list shows it; the pane does not need it again.
        } else if let Some(finding) = finding(line) {
            reading.findings.push(finding);
        } else {
            reading.rest.push(raw.to_string());
        }
    }
    reading
}

/// The findings that sit on a line `step` shows, each with the index of that
/// line in the hunk, top to bottom; and those that cite a line it does not.
pub fn place<'a>(
    step: &GuidedHunk,
    findings: &'a [Finding],
) -> (Vec<(usize, &'a Finding)>, Vec<&'a Finding>) {
    let mut placed = Vec::new();
    let mut unplaced = Vec::new();
    for finding in findings {
        match index_of(step, finding.line) {
            Some(at) => placed.push((at, finding)),
            None => unplaced.push(finding),
        }
    }
    placed.sort_by_key(|(at, _)| *at);
    (placed, unplaced)
}

/// The index in `step` of the line the prompt labelled `L{line}`: a line the
/// new file has with that number, else a removed line the old file had it on.
fn index_of(step: &GuidedHunk, line: usize) -> Option<usize> {
    step.lines
        .iter()
        .position(|l| l.new == Some(line))
        .or_else(|| {
            step.lines
                .iter()
                .position(|l| l.new.is_none() && l.old == Some(line))
        })
}

/// The line without list markers, emphasis or code ticks around it.
fn bare(line: &str) -> &str {
    let line = line.trim();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("\u{2022} "))
        .unwrap_or(line);
    line.trim_start_matches(['*', '`']).trim()
}

/// What follows `key:` at the start of `line`, in any case.
fn after_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let head = line.get(..key.len())?;
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    let rest = line[key.len()..].trim_start_matches(['*', '`']);
    Some(
        rest.strip_prefix(':')?
            .trim_start_matches(['*', '`'])
            .trim(),
    )
}

/// `L123 issue: text`, as the step prompt asks for it, with the separators a
/// model is likely to vary.
fn finding(line: &str) -> Option<Finding> {
    let rest = line.strip_prefix('L')?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let number = rest[..digits].parse().ok()?;
    // A range or a list of lines names the first of them.
    let rest = rest[digits..]
        .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '-' | ',' | 'L'))
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '`' | '*' | '('));
    let word_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let severity = match rest[..word_end].to_ascii_lowercase().as_str() {
        "issue" => Severity::Issue,
        "nit" => Severity::Nit,
        _ => return None,
    };
    // One separator, and no more: the text itself may open on `-1`.
    let text = rest[word_end..]
        .trim_start_matches(['*', ')', '`'])
        .trim_start();
    let text = [":", "\u{2014}", "\u{2013}", "- "]
        .iter()
        .find_map(|separator| text.strip_prefix(separator))
        .unwrap_or(text)
        .trim();
    Some(Finding {
        line: number,
        severity,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_taken_apart_whatever_its_markup_and_the_rest_is_kept() {
        let reading = read(
            "Summary: Caches the lookup.\n\
             L12 issue: -1 means missing but is used as an index.\n\
             - `L40` **issue** \u{2014} unchecked unwrap\n\
             * L7-L9 (nit): long\n\
             Fixed: the unchecked cast.\n\
             Check: who else calls lookup.\n\
             This line ignored the shape.\n\
             Verdict: issue",
        );

        assert_eq!(reading.summary.as_deref(), Some("Caches the lookup."));
        assert_eq!(reading.check.as_deref(), Some("who else calls lookup."));
        assert_eq!(reading.fixed, vec!["the unchecked cast."]);
        assert_eq!(reading.rest, vec!["This line ignored the shape."]);
        assert!(reading.findings.contains(&Finding {
            line: 12,
            severity: Severity::Issue,
            text: "-1 means missing but is used as an index.".into(),
        }));
        assert!(reading.findings.contains(&Finding {
            line: 40,
            severity: Severity::Issue,
            text: "unchecked unwrap".into(),
        }));
        assert!(reading.findings.contains(&Finding {
            line: 7,
            severity: Severity::Nit,
            text: "long".into(),
        }));
    }

    /// The prompt numbers a removed line by the old file, everything else by
    /// the new one; a line the hunk does not show has no row.
    #[test]
    fn a_finding_lands_on_the_line_the_prompt_numbered() {
        let step = crate::git::guided::parse_hunks(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
             @@ -10,3 +10,2 @@\n keep\n-gone\n-also\n+came\n",
        )
        .remove(0);
        let findings = read("L11 issue: new\nL12 nit: removed\nL99 nit: elsewhere").findings;

        let (placed, unplaced) = place(&step, &findings);

        let text_at = |line: usize| {
            placed
                .iter()
                .find(|(_, f)| f.line == line)
                .map(|(at, _)| step.lines[*at].text.as_str())
        };
        assert_eq!(text_at(11), Some("+came"));
        assert_eq!(text_at(12), Some("-also"));
        assert_eq!(text_at(99), None);
        assert!(unplaced.iter().any(|f| f.line == 99));
    }
}
