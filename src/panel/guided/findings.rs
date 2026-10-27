//! A step's commentary taken apart into its summary and the findings it pins
//! to lines of the file, or for the final review, to lines of any file.

use crate::git::guided::GuidedFile;

/// Ordered so the worst of several is their `max`. A question is not a
/// defect, so it ranks below a nit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Question,
    Nit,
    Issue,
}

impl Severity {
    pub const ALL: [Self; 3] = [Self::Issue, Self::Nit, Self::Question];

    pub fn word(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Nit => "nit",
            Self::Question => "question",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The line as the step prompt numbers it: the new file's number, or the
    /// old file's for a removed line.
    pub line: usize,
    pub severity: Severity,
    pub text: String,
}

/// An answer that ignored the asked-for shape lands whole in `rest`.
#[derive(Debug, Default)]
pub struct Reading {
    pub summary: Option<String>,
    pub findings: Vec<Finding>,
    /// Findings that name their file, as the final review across files gives them.
    pub cited: Vec<(String, Finding)>,
    pub check: Option<String>,
    pub fixed: Vec<String>,
    pub rest: Vec<String>,
}

pub fn read(text: &str) -> Reading {
    let mut reading = Reading::default();
    for raw in text.lines() {
        let line = without_markup(raw);
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
            // Shown in the step list already.
        } else if let Some(finding) = parse_finding(line) {
            reading.findings.push(finding);
        } else if let Some(cited) = parse_cited(line) {
            reading.cited.push(cited);
        } else {
            reading.rest.push(raw.to_string());
        }
    }
    reading
}

/// The commentary `text` without its question lines, and those lines.
pub fn split_questions(text: &str) -> (String, Vec<String>) {
    let mut kept = Vec::new();
    let mut questions = Vec::new();
    for line in text.lines() {
        match parse_finding(without_markup(line)) {
            Some(finding) if finding.severity == Severity::Question => {
                questions.push(line.trim().to_string());
            }
            _ => kept.push(line),
        }
    }
    (kept.join("\n"), questions)
}

/// `text` with the findings `settled` names put back ahead of its verdict,
/// which the step prompt keeps as the last line. Anything in `settled` that is
/// not a finding is dropped.
pub fn with_settled(text: &str, settled: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    let verdict_at = lines
        .iter()
        .rposition(|line| after_key(without_markup(line), "verdict").is_some())
        .unwrap_or(lines.len());
    let findings = settled
        .lines()
        .filter(|line| parse_finding(without_markup(line)).is_some());
    lines.splice(verdict_at..verdict_at, findings);
    lines.join("\n")
}

/// Findings on a line the file's diff shows, with that line's index; then the rest.
pub fn place<'a>(
    step: &GuidedFile,
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
fn index_of(step: &GuidedFile, line: usize) -> Option<usize> {
    step.lines
        .iter()
        .position(|l| l.new == Some(line))
        .or_else(|| {
            step.lines
                .iter()
                .position(|l| l.new.is_none() && l.old == Some(line))
        })
}

fn without_markup(line: &str) -> &str {
    let line = line.trim();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("\u{2022} "))
        .unwrap_or(line);
    line.trim_start_matches(['*', '`']).trim()
}

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

/// `src/a.rs:L123 issue: text`: a finding that names its file.
fn parse_cited(line: &str) -> Option<(String, Finding)> {
    let at = line
        .match_indices(":L")
        .map(|(at, _)| at)
        .find(|&at| line[at + 2..].starts_with(|c: char| c.is_ascii_digit()))?;
    let path = line[..at].trim_matches(['`', '*']).trim();
    if path.is_empty() || path.contains(char::is_whitespace) {
        return None;
    }
    Some((path.to_string(), parse_finding(&line[at + 1..])?))
}

/// `L123 issue: text`, tolerating the separators a model tends to vary.
fn parse_finding(line: &str) -> Option<Finding> {
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
    let word = rest[..word_end].to_ascii_lowercase();
    let severity = Severity::ALL
        .into_iter()
        .find(|severity| severity.word() == word)?;
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
             L20 question: is the cache shared between threads?\n\
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
        assert!(reading.findings.contains(&Finding {
            line: 20,
            severity: Severity::Question,
            text: "is the cache shared between threads?".into(),
        }));
    }

    #[test]
    fn a_finding_across_files_keeps_the_file_it_names() {
        let reading = read(
            "Summary: Renames the cache.\n\
             src/a.rs:L12 issue: the caller still passes the old name.\n\
             - `src/b.rs:L3` nit: tests repeat the setup.\n\
             Verdict: issue",
        );

        assert!(reading.cited.contains(&(
            "src/a.rs".to_string(),
            Finding {
                line: 12,
                severity: Severity::Issue,
                text: "the caller still passes the old name.".into(),
            }
        )));
        assert!(
            reading
                .cited
                .iter()
                .any(|(path, finding)| path == "src/b.rs" && finding.line == 3)
        );
        assert!(reading.rest.is_empty(), "{:?}", reading.rest);
    }

    #[test]
    fn settled_questions_take_the_place_of_the_ones_asked() {
        let (text, questions) = split_questions(
            "Summary: Caches the lookup.\n\
             L12 issue: -1 is used as an index.\n\
             L20 question: is the cache shared between threads?\n\
             L30 question: is lookup called elsewhere?\n\
             Verdict: issue",
        );
        assert_eq!(questions.len(), 2);

        let settled = with_settled(
            &text,
            "Checked the callers.\nL20 issue: the cache is shared and unguarded.",
        );
        let reading = read(&settled);

        assert!(
            reading
                .findings
                .iter()
                .all(|finding| finding.severity != Severity::Question)
        );
        assert!(reading.findings.contains(&Finding {
            line: 20,
            severity: Severity::Issue,
            text: "the cache is shared and unguarded.".into(),
        }));
        assert!(reading.findings.iter().any(|finding| finding.line == 12));
        assert!(reading.rest.is_empty(), "{:?}", reading.rest);
        assert!(settled.trim_end().ends_with("Verdict: issue"));
    }

    #[test]
    fn a_finding_lands_on_the_line_the_prompt_numbered() {
        let step = crate::git::guided::parse_files(
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
