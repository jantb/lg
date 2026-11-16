//! Taking the model's answer in: what lg keeps of what a read said.
//!
//! After a file's first read its open findings change only when one is
//! fixed, when the reviewer ticks it off, or when an edit breaks something
//! on a line it changed. The model is asked to keep to that, and lg holds it
//! to it here: a re-read only says which earlier findings are fixed and
//! which stand, a standing finding keeps the words it was first given, and
//! anything new is kept only as an issue on a line changed since.

use std::collections::{HashMap, HashSet};

use crate::git::guided::{
    FileRead, GuidedFile, HunkLine, LineKind, ReadStep, TickedFinding, WholeRead,
};

use super::Verdict;
use super::findings::{self, Finding, Severity};
use super::found::ticked_in;

/// The most nits one read of a file keeps.
pub const FILE_NIT_BUDGET: usize = 3;
/// The most nits one read of the whole change keeps.
pub const WHOLE_NIT_BUDGET: usize = 5;

/// How the answer to one read is taken in.
pub(super) enum TakeIn {
    /// The plan, kept as it was written.
    AsWritten,
    FileFirst {
        path: String,
    },
    FileDelta(FileBasis),
    WholeFirst,
    WholeDelta(WholeBasis),
}

/// A finding of an earlier read, carried into the read after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Carried {
    pub path: String,
    /// The `F<id>` the re-read answers it by; none for one the reviewer
    /// dismissed, which the re-read is not asked about.
    pub id: Option<usize>,
    /// As the earlier read gave it.
    pub finding: Finding,
    /// The line it is on in the diff as it is now, as far as lg can tell.
    pub now: usize,
}

/// What a re-read of a file the reviewer changed is set against.
pub(super) struct FileBasis {
    pub path: String,
    pub summary: Option<String>,
    pub earlier: Vec<Carried>,
    /// Every line the earlier read's change added or removed.
    pub changed_then: Vec<String>,
}

/// What a re-read of the whole change is set against.
pub(super) struct WholeBasis {
    pub summary: Option<String>,
    pub earlier: Vec<Carried>,
    /// Every file as the earlier read saw it.
    pub files: Vec<ReadStep>,
}

/// `step` as a read of it sees it, with the diff lines `cited` points at.
pub(super) fn read_step(step: &GuidedFile, cited: impl IntoIterator<Item = usize>) -> ReadStep {
    let mut quotes: Vec<(usize, String)> = Vec::new();
    for line in cited {
        if quotes.iter().any(|(at, _)| *at == line) {
            continue;
        }
        if let Some(index) = findings::index_of(step, line) {
            quotes.push((line, step.lines[index].text.clone()));
        }
    }
    ReadStep {
        path: step.path.clone(),
        key: step.key(),
        changed: changed_lines(step).map(|line| line.text.clone()).collect(),
        quotes,
    }
}

fn changed_lines(step: &GuidedFile) -> impl Iterator<Item = &HunkLine> {
    step.lines
        .iter()
        .filter(|line| matches!(line.kind, LineKind::Added | LineKind::Removed))
}

/// The diff lines of `step`, by index, that were added or modified since a
/// read whose change had `changed_then`: each changed line now whose text
/// that change did not have, a text it had more than once matched as often.
pub(super) fn changed_since(changed_then: &[String], step: &GuidedFile) -> HashSet<usize> {
    let mut then: HashMap<&str, usize> = HashMap::new();
    for text in changed_then {
        *then.entry(text.as_str()).or_default() += 1;
    }
    let mut changed = HashSet::new();
    for (index, line) in step.lines.iter().enumerate() {
        if !matches!(line.kind, LineKind::Added | LineKind::Removed) {
            continue;
        }
        match then.get_mut(line.text.as_str()) {
            Some(count) if *count > 0 => *count -= 1,
            _ => {
                changed.insert(index);
            }
        }
    }
    changed
}

/// The line `line` of an earlier read is on now: the line of `step` that
/// reads as `quote` did, nearest the old number; the old number when no
/// line does.
fn reanchor(step: &GuidedFile, line: usize, quote: Option<&str>) -> usize {
    let Some(quote) = quote else {
        return line;
    };
    step.lines
        .iter()
        .filter(|candidate| candidate.text == quote)
        .filter_map(HunkLine::anchor)
        .map(|(number, _)| number)
        .min_by_key(|number| number.abs_diff(line))
        .unwrap_or(line)
}

fn quote_of(read: &ReadStep, line: usize) -> Option<&str> {
    read.quotes
        .iter()
        .find(|(at, _)| *at == line)
        .map(|(_, quote)| quote.as_str())
}

/// Numbers the findings still open `F1`, `F2`, … in the order given; a
/// dismissed one is carried without a number.
fn carry(
    earlier: impl IntoIterator<Item = (String, Finding, usize)>,
    ticked: &[TickedFinding],
) -> Vec<Carried> {
    let mut next = 0;
    earlier
        .into_iter()
        .map(|(path, finding, now)| {
            let id = (!ticked_in(ticked, &path, &finding.text)).then(|| {
                next += 1;
                next
            });
            Carried {
                path,
                id,
                finding,
                now,
            }
        })
        .collect()
}

/// What a re-read of `step` is set against: `earlier`, a finished read of
/// the file as it was.
pub(super) fn file_basis(
    earlier: &FileRead,
    step: &GuidedFile,
    ticked: &[TickedFinding],
) -> FileBasis {
    let reading = findings::read(&earlier.text);
    let carried = reading.findings.into_iter().map(|finding| {
        let now = reanchor(step, finding.line, quote_of(&earlier.step, finding.line));
        (step.path.clone(), finding, now)
    });
    FileBasis {
        path: step.path.clone(),
        summary: reading.summary,
        earlier: carry(carried, ticked),
        changed_then: earlier.step.changed.clone(),
    }
}

/// What a re-read of the whole change is set against: `earlier`, a finished
/// read of it as it was.
pub(super) fn whole_basis(
    earlier: &WholeRead,
    steps: &[GuidedFile],
    ticked: &[TickedFinding],
) -> WholeBasis {
    let reading = findings::read(&earlier.text);
    let carried = reading.cited.into_iter().map(|(path, finding)| {
        let then = earlier.files.iter().find(|file| file.path == path);
        let now = match steps.iter().find(|step| step.path == path) {
            Some(step) => reanchor(
                step,
                finding.line,
                then.and_then(|then| quote_of(then, finding.line)),
            ),
            None => finding.line,
        };
        (path, finding, now)
    });
    WholeBasis {
        summary: reading.summary,
        earlier: carry(carried, ticked),
        files: earlier.files.clone(),
    }
}

/// A first read's answer as lg keeps it: a finding the reviewer dismissed
/// raised again in the same words is dropped, and nits past `nit_budget`
/// are dropped in the order given. `path` is the file a finding that names
/// none is about. Anything dropped settles the verdict anew on what is left.
pub(super) fn take_in_first(
    reply: &str,
    path: Option<&str>,
    nit_budget: usize,
    ticked: &[TickedFinding],
) -> String {
    let mut kept = Vec::new();
    let mut nits = 0;
    let mut dropped = Vec::new();
    let mut left = Vec::new();
    for line in reply.lines() {
        let bare = findings::without_markup(line);
        let found = findings::parse_finding(bare)
            .map(|finding| (path.map(str::to_string), finding))
            .or_else(|| findings::parse_cited(bare).map(|(path, finding)| (Some(path), finding)));
        let Some((about, finding)) = found else {
            kept.push(line);
            continue;
        };
        let dismissed = about
            .as_deref()
            .is_some_and(|about| ticked_in(ticked, about, &finding.text));
        let over_budget = finding.severity == Severity::Nit && nits >= nit_budget;
        if dismissed || over_budget {
            dropped.push(finding.severity);
            continue;
        }
        if finding.severity == Severity::Nit {
            nits += 1;
        }
        left.push(finding.severity);
        kept.push(line);
    }
    if dropped.is_empty() {
        return reply.to_string();
    }
    let Some(at) = kept
        .iter()
        .rposition(|line| findings::after_key(findings::without_markup(line), "verdict").is_some())
    else {
        return kept.join("\n");
    };
    // A verdict worse than anything dropped was given for something still
    // there, said or not; one no worse may have been for what was dropped.
    let given = verdict_of(kept[at]);
    let verdict = match given {
        Some(given) if given > worst(dropped) => given,
        _ => worst(left),
    };
    let verdict = format!("Verdict: {}", verdict_word(verdict));
    kept[at] = &verdict;
    kept.join("\n")
}

fn verdict_of(line: &str) -> Option<Verdict> {
    let word = findings::after_key(findings::without_markup(line), "verdict")?;
    match word
        .trim_matches(|c: char| !c.is_alphabetic())
        .to_ascii_lowercase()
        .as_str()
    {
        "ok" => Some(Verdict::Ok),
        "nit" => Some(Verdict::Nit),
        "issue" => Some(Verdict::Issue),
        _ => None,
    }
}

/// What a re-read answered, by the ids lg gave the earlier findings.
#[derive(Debug, Default)]
struct DeltaReply {
    summary: Option<String>,
    fixed: HashSet<usize>,
    /// Each standing finding, with the line the answer says it is on now.
    stands: HashMap<usize, Option<usize>>,
    /// Findings the answer raised, with the file when it named one.
    raised: Vec<(Option<String>, Finding)>,
}

fn read_delta(reply: &str) -> DeltaReply {
    let mut out = DeltaReply::default();
    for line in reply.lines() {
        let bare = findings::without_markup(line);
        if let Some(summary) = findings::after_key(bare, "summary") {
            out.summary = Some(summary.to_string());
        } else if let Some(fixed) = findings::after_key(bare, "fixed") {
            out.fixed
                .extend(ids_in(fixed).into_iter().map(|(id, _)| id));
        } else if let Some(stands) = findings::after_key(bare, "stands") {
            for (id, after) in ids_in(stands) {
                out.stands.insert(id, line_in(after));
            }
        } else if let Some(finding) = findings::parse_finding(bare) {
            out.raised.push((None, finding));
        } else if let Some((path, finding)) = findings::parse_cited(bare) {
            out.raised.push((Some(path), finding));
        }
    }
    out
}

/// Each `F<id>` in `text`, with what follows it up to the next id.
fn ids_in(text: &str) -> Vec<(usize, &str)> {
    let starts: Vec<(usize, usize, usize)> = text
        .char_indices()
        .filter(|&(at, ch)| {
            ch.eq_ignore_ascii_case(&'f')
                && !text[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
        })
        .filter_map(|(at, _)| {
            let digits = text[at + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .count();
            let id = text[at + 1..at + 1 + digits].parse().ok()?;
            Some((at, at + 1 + digits, id))
        })
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &(_, end, id))| {
            let until = starts.get(n + 1).map_or(text.len(), |&(at, _, _)| at);
            (id, &text[end..until])
        })
        .collect()
}

/// The first `L<n>` in `text`.
fn line_in(text: &str) -> Option<usize> {
    text.match_indices('L').find_map(|(at, _)| {
        let digits: String = text[at + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

/// The new commentary of a file re-read against `basis`, built by lg from
/// `reply`: the summary, each fixed finding as it was first worded, each
/// standing one as it was first worded on the line it is on now, then the
/// issues the edit introduced on lines it changed, and the verdict of what
/// is open. An earlier finding the reply leaves out stands.
pub(super) fn take_in_file_delta(
    basis: &FileBasis,
    step: &GuidedFile,
    reply: &str,
    ticked: &[TickedFinding],
) -> String {
    let reply = read_delta(reply);
    let changed = changed_since(&basis.changed_then, step);
    let (fixed, kept) = settle(&basis.earlier, &reply, |carried, line| {
        line.filter(|&line| findings::index_of(step, line).is_some())
            .unwrap_or(carried.now)
    });
    let raised: Vec<(String, Finding)> = reply
        .raised
        .into_iter()
        .filter(|(path, _)| path.as_deref().is_none_or(|path| path == basis.path))
        .map(|(_, finding)| (basis.path.clone(), finding))
        .filter(|(_, finding)| {
            findings::index_of(step, finding.line).is_some_and(|at| changed.contains(&at))
        })
        .collect();
    let raised = new_issues(raised, &kept, ticked);
    let mut out = vec![summary_line(reply.summary.or(basis.summary.clone()))];
    out.extend(
        fixed
            .iter()
            .map(|carried| format!("Fixed: {}", carried.finding.text)),
    );
    out.extend(
        kept.iter()
            .chain(&raised)
            .map(|(_, finding)| finding_line(finding)),
    );
    out.push(open_verdict(kept.iter().chain(&raised), ticked));
    out.join("\n")
}

/// The new commentary of the whole change re-read against `basis`, built as
/// [`take_in_file_delta`] builds a file's: a new finding is kept only as an
/// issue in a file whose change differs from when the earlier read saw it,
/// on a line changed since.
pub(super) fn take_in_whole_delta(
    basis: &WholeBasis,
    steps: &[GuidedFile],
    reply: &str,
    ticked: &[TickedFinding],
) -> String {
    let reply = read_delta(reply);
    let step_of = |path: &str| steps.iter().find(|step| step.path == path);
    let (fixed, kept) = settle(&basis.earlier, &reply, |carried, line| {
        line.filter(|&line| {
            step_of(&carried.path).is_some_and(|step| findings::index_of(step, line).is_some())
        })
        .unwrap_or(carried.now)
    });
    let raised: Vec<(String, Finding)> = reply
        .raised
        .into_iter()
        .filter_map(|(path, finding)| Some((path?, finding)))
        .filter(|(path, finding)| {
            let Some(step) = step_of(path) else {
                return false;
            };
            let then = basis.files.iter().find(|file| file.path == *path);
            if then.is_some_and(|then| then.key == step.key()) {
                return false;
            }
            let changed = changed_since(then.map_or(&[][..], |then| &then.changed), step);
            findings::index_of(step, finding.line).is_some_and(|at| changed.contains(&at))
        })
        .collect();
    let raised = new_issues(raised, &kept, ticked);
    let mut out = vec![summary_line(reply.summary.or(basis.summary.clone()))];
    out.extend(
        fixed
            .iter()
            .map(|carried| format!("Fixed: {}: {}", carried.path, carried.finding.text)),
    );
    out.extend(
        kept.iter()
            .chain(&raised)
            .map(|(path, finding)| format!("{path}:{}", finding_line(finding))),
    );
    out.push(open_verdict(kept.iter().chain(&raised), ticked));
    out.join("\n")
}

/// The earlier findings `reply` says are fixed, and every other one as it
/// was first worded on the line `line_now` puts it, given the line the reply
/// named for it. Said to stand and to be fixed, a finding stands.
fn settle<'a>(
    earlier: &'a [Carried],
    reply: &DeltaReply,
    line_now: impl Fn(&Carried, Option<usize>) -> usize,
) -> (Vec<&'a Carried>, Vec<(String, Finding)>) {
    let mut fixed = Vec::new();
    let mut kept = Vec::new();
    for carried in earlier {
        let stands = carried.id.and_then(|id| reply.stands.get(&id));
        let is_fixed = carried
            .id
            .is_some_and(|id| reply.fixed.contains(&id) && stands.is_none());
        if is_fixed {
            fixed.push(carried);
            continue;
        }
        let line = line_now(carried, stands.copied().flatten());
        kept.push((
            carried.path.clone(),
            Finding {
                line,
                ..carried.finding.clone()
            },
        ));
    }
    (fixed, kept)
}

/// Of `raised`, the issues neither dismissed nor already standing in the
/// same words.
fn new_issues(
    raised: Vec<(String, Finding)>,
    kept: &[(String, Finding)],
    ticked: &[TickedFinding],
) -> Vec<(String, Finding)> {
    raised
        .into_iter()
        .filter(|(_, finding)| finding.severity == Severity::Issue)
        .filter(|(path, finding)| !ticked_in(ticked, path, &finding.text))
        .filter(|(path, finding)| {
            !kept
                .iter()
                .any(|(kept_path, kept)| kept_path == path && kept.text == finding.text)
        })
        .collect()
}

fn summary_line(summary: Option<String>) -> String {
    format!("Summary: {}", summary.unwrap_or_default())
}

fn finding_line(finding: &Finding) -> String {
    format!(
        "L{} {}: {}",
        finding.line,
        finding.severity.word(),
        finding.text
    )
}

fn open_verdict<'a>(
    findings: impl Iterator<Item = &'a (String, Finding)>,
    ticked: &[TickedFinding],
) -> String {
    let open = findings
        .filter(|(path, finding)| !ticked_in(ticked, path, &finding.text))
        .map(|(_, finding)| finding.severity);
    format!("Verdict: {}", verdict_word(worst(open)))
}

fn worst(severities: impl IntoIterator<Item = Severity>) -> Verdict {
    severities
        .into_iter()
        .map(|severity| match severity {
            Severity::Issue => Verdict::Issue,
            Severity::Nit => Verdict::Nit,
            Severity::Question => Verdict::Ok,
        })
        .max()
        .unwrap_or(Verdict::Ok)
}

fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Ok => "ok",
        Verdict::Nit => "nit",
        Verdict::Issue => "issue",
    }
}
