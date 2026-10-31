//! What the reads found, in each file and across the whole change, which of
//! it is still open, and ticking it off.

use std::collections::HashSet;

use crate::git::guided::{GuidedFile, TickedFinding};
use crate::state::{AppState, DiffViewMode};

use super::{Guided, OpenFinding, Verdict, findings};

impl Guided {
    /// Every finding not yet ticked off in what has been read, the final
    /// review's included: issues, then nits, then questions, each in the
    /// walk's order and down the file.
    pub fn open_findings(&self) -> Vec<OpenFinding> {
        self.findings_in_order()
            .into_iter()
            .filter(|(_, ticked)| !ticked)
            .map(|(open, _)| open)
            .collect()
    }

    /// Every finding of what has been read in [`Guided::open_findings`]'
    /// order, the ticked ones kept in their place and said to be ticked, so
    /// ticking the one on screen does not lose the walk its place.
    pub(super) fn findings_in_order(&self) -> Vec<(OpenFinding, bool)> {
        let mut out = Vec::new();
        let mut push = |step: usize, finding: findings::Finding, whole: bool| {
            let file = &self.steps[step - 1];
            let ticked = self.is_ticked(&ticket_in(&file.path, &finding));
            let at = findings::place(file, std::slice::from_ref(&finding))
                .0
                .first()
                .map(|(at, _)| *at);
            out.push((
                OpenFinding {
                    step,
                    at,
                    finding,
                    whole,
                },
                ticked,
            ));
        };
        for (index, step) in self.steps.iter().enumerate() {
            let Some(commentary) = self
                .commentary
                .get(&step.key())
                .filter(|c| c.done || c.checking)
            else {
                continue;
            };
            for finding in findings::read(&commentary.text).findings {
                push(index + 1, finding, false);
            }
        }
        if let Some(commentary) = self.commentary.get(&self.final_key()).filter(|c| c.done) {
            for (path, finding) in findings::read(&commentary.text).cited {
                if let Some(index) = self.steps.iter().position(|step| step.path == path) {
                    push(index + 1, finding, true);
                }
            }
        }
        let order = self.walk_order();
        let place = |step: usize| {
            order
                .iter()
                .position(|&at| at == step)
                .unwrap_or(order.len())
        };
        out.sort_by_key(|(open, _)| {
            (
                std::cmp::Reverse(open.finding.severity),
                place(open.step),
                open.finding.line,
            )
        });
        out
    }

    pub(super) fn flagged_lines(&self, mode: DiffViewMode) -> Vec<usize> {
        let flagged: HashSet<usize> = self
            .placed_findings()
            .into_iter()
            .filter(|(_, finding)| !self.is_ticked(&self.ticket(finding)))
            .map(|(at, _)| at)
            .collect();
        self.visit_order(mode)
            .into_iter()
            .filter(|at| flagged.contains(at))
            .collect()
    }

    pub(super) fn findings_at_cursor(&self) -> String {
        self.placed_findings()
            .into_iter()
            .filter(|(at, _)| *at == self.cursor)
            .map(|(_, finding)| finding.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn placed_findings(&self) -> Vec<(usize, findings::Finding)> {
        let Some(step) = self.step() else {
            return Vec::new();
        };
        findings::place(step, &self.findings_in(step))
            .0
            .into_iter()
            .map(|(at, finding)| (at, finding.clone()))
            .collect()
    }

    /// What was found in `step`: its own read's findings, then those the
    /// final review cites in its file.
    pub(super) fn findings_in(&self, step: &GuidedFile) -> Vec<findings::Finding> {
        let own = self
            .commentary
            .get(&step.key())
            .map(|commentary| findings::read(&commentary.text).findings)
            .unwrap_or_default();
        let cited = self
            .final_cited()
            .into_iter()
            .filter(|(path, _)| *path == step.path)
            .map(|(_, finding)| finding);
        own.into_iter().chain(cited).collect()
    }

    /// The findings the final review cites, by file, in the order it gave them.
    pub fn final_cited(&self) -> Vec<(String, findings::Finding)> {
        self.commentary
            .get(&self.final_key())
            .map(|commentary| findings::read(&commentary.text).cited)
            .unwrap_or_default()
    }

    /// The finding picked on the final step, kept in range as the review is read again.
    pub fn picked(&self) -> Option<(String, findings::Finding)> {
        let mut cited = self.final_cited();
        let at = self.final_pick.min(cited.len().checked_sub(1)?);
        Some(cited.swap_remove(at))
    }

    /// The picked finding as a place in its file's diff, when the change has that file.
    pub(super) fn picked_open(&self) -> Option<OpenFinding> {
        let (path, finding) = self.picked()?;
        let index = self.steps.iter().position(|step| step.path == path)?;
        let at = findings::place(&self.steps[index], std::slice::from_ref(&finding))
            .0
            .first()
            .map(|(at, _)| *at);
        Some(OpenFinding {
            step: index + 1,
            at,
            finding,
            whole: true,
        })
    }

    /// The step and diff line a change or an edit is aimed at: the cursor
    /// line of the file on screen, or on the final step the picked finding's.
    pub(super) fn aim(&self) -> Option<(usize, usize)> {
        if self.on_final() {
            let open = self.picked_open()?;
            let at = open
                .at
                .unwrap_or_else(|| self.steps[open.step - 1].first_changed());
            return Some((open.step, at));
        }
        self.step().map(|_| (self.current, self.cursor))
    }

    pub(super) fn final_done(&self) -> bool {
        self.commentary
            .get(&self.final_key())
            .is_some_and(|commentary| commentary.done)
    }

    /// The final review's findings not yet ticked off, one `path:L<n>` line each.
    pub(super) fn ticket(&self, finding: &findings::Finding) -> TickedFinding {
        ticket_in(
            self.step()
                .map(|step| step.path.as_str())
                .unwrap_or_default(),
            finding,
        )
    }

    /// The worst finding not yet ticked off; an answer that named none keeps its own verdict.
    pub fn verdict(&self, step: &GuidedFile) -> Option<Verdict> {
        let commentary = self.commentary.get(&step.key())?;
        let given = commentary.verdict()?;
        let findings = findings::read(&commentary.text).findings;
        let cited = findings.iter().map(|finding| (step.path.as_str(), finding));
        Some(self.settled(given, cited))
    }

    /// The final review's verdict, settled by ticks as a file's is.
    pub fn final_verdict(&self) -> Option<Verdict> {
        let commentary = self.commentary.get(&self.final_key())?;
        let given = commentary.verdict()?;
        let cited = findings::read(&commentary.text).cited;
        Some(self.settled(
            given,
            cited.iter().map(|(path, finding)| (path.as_str(), finding)),
        ))
    }

    pub(super) fn settled<'a>(
        &self,
        given: Verdict,
        findings: impl Iterator<Item = (&'a str, &'a findings::Finding)>,
    ) -> Verdict {
        let mut findings = findings.peekable();
        if findings.peek().is_none() {
            return given;
        }
        findings
            .filter(|(path, finding)| !self.is_ticked(&ticket_in(path, finding)))
            .map(|(_, finding)| match finding.severity {
                findings::Severity::Issue => Verdict::Issue,
                findings::Severity::Nit => Verdict::Nit,
                findings::Severity::Question => Verdict::Ok,
            })
            .max()
            .unwrap_or(Verdict::Ok)
    }

    /// How many findings of `severity` in the file are not yet ticked off.
    pub fn open_in_file(&self, path: &str, severity: findings::Severity) -> usize {
        self.steps
            .iter()
            .filter(|step| step.path == path)
            .filter_map(|step| self.commentary.get(&step.key()).filter(|c| c.done))
            .flat_map(|commentary| findings::read(&commentary.text).findings)
            .filter(|finding| finding.severity == severity)
            .filter(|finding| !self.is_ticked(&ticket_in(path, finding)))
            .count()
    }

    pub fn is_ticked(&self, finding: &TickedFinding) -> bool {
        self.progress.ticked.contains(finding)
    }
}

pub(super) fn tick_at_cursor(state: &mut AppState) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    if guided.on_final() {
        match guided.picked() {
            Some((path, finding)) => toggle_ticks(state, vec![ticket_in(&path, &finding)]),
            None => state.set_status("the final review has no findings to tick off", false),
        }
        return;
    }
    let here: Vec<TickedFinding> = guided
        .placed_findings()
        .into_iter()
        .filter(|(at, _)| *at == guided.cursor)
        .map(|(_, finding)| guided.ticket(&finding))
        .collect();
    if here.is_empty() {
        state.set_status("nothing flagged on this line", false);
        return;
    }
    toggle_ticks(state, here);
}

pub(super) fn toggle_ticks(state: &mut AppState, findings: Vec<TickedFinding>) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let ticked = &mut guided.progress.ticked;
    if findings.iter().all(|finding| ticked.contains(finding)) {
        ticked.retain(|finding| !findings.contains(finding));
    } else {
        let new: Vec<TickedFinding> = findings
            .into_iter()
            .filter(|f| !ticked.contains(f))
            .collect();
        ticked.extend(new);
    }
    if let Err(message) = guided.save() {
        state.set_status(message, true);
    }
}

pub(super) fn ticket_in(path: &str, finding: &findings::Finding) -> TickedFinding {
    TickedFinding {
        path: path.to_string(),
        line: finding.line,
        text: finding.text.clone(),
    }
}
