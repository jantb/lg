//! The overview step: what the change is, where its problems are, and the
//! plan for walking it. A finding, a file worth watching or a risky file
//! leads to its place in the walk when clicked.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::git::guided::WalkMode;
use crate::panel::guided::plan::{Plan, Risk};
use crate::panel::guided::{Guided, OpenFinding};
use crate::panel::wrap_words;
use crate::state::{AppState, SPINNER_FRAMES};

use super::commentary::commentary_lines;
use super::list::risk_color;
use super::panes::commentary_area;
use super::{
    KEY, LABEL, progress_text, severity_color, severity_mark, truncate_left, wrapped_rows,
};

/// Where a row of the overview leads when clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::panel::guided) enum OverviewTarget {
    Finding(OpenFinding),
    /// A file, counted from one.
    File(usize),
}

/// The overview as drawn, one row a line, and the rows that lead somewhere.
pub(super) struct OverviewLines {
    pub lines: Vec<Line<'static>>,
    pub targets: Vec<(Range<usize>, OverviewTarget)>,
}

impl OverviewLines {
    fn push(&mut self, line: Line<'static>) {
        self.lines.push(line);
    }

    fn extend(&mut self, lines: impl IntoIterator<Item = Line<'static>>) {
        self.lines.extend(lines);
    }

    fn push_to(&mut self, lines: Vec<Line<'static>>, target: Option<OverviewTarget>) {
        let start = self.lines.len();
        self.lines.extend(lines);
        if let Some(target) = target {
            self.targets.push((start..self.lines.len(), target));
        }
    }
}

pub(super) fn draw_overview(state: &AppState, guided: &Guided, area: Rect, frame: &mut Frame) {
    let lines = overview_lines(state, guided, area.width).lines;
    let max = wrapped_rows(&lines, area.width).saturating_sub(area.height as usize) as u16;
    guided.side_max.set(max);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((guided.side_scroll.min(max), 0)),
        area,
    );
}

/// What a click at `column`, `row` on the overview leads to.
pub(in crate::panel::guided) fn overview_target_at(
    state: &AppState,
    side: Rect,
    column: u16,
    row: u16,
) -> Option<OverviewTarget> {
    let guided = state.guided.as_deref()?;
    if guided.step().is_some() || guided.on_final() {
        return None;
    }
    let area = commentary_area(guided, side);
    if !crate::panel::pointer::inside(area, column, row) {
        return None;
    }
    let at = usize::from(guided.side_scroll.min(guided.side_max.get()) + row - area.y);
    overview_lines(state, guided, area.width)
        .targets
        .into_iter()
        .find(|(rows, _)| rows.contains(&at))
        .map(|(_, target)| target)
}

pub(super) fn overview_lines(state: &AppState, guided: &Guided, width: u16) -> OverviewLines {
    let mut out = OverviewLines {
        lines: Vec::new(),
        targets: Vec::new(),
    };
    out.push(heading("What this change is", Color::White));
    match (guided.plan(), guided.planning()) {
        (Some(plan), _) => {
            if let Some(summary) = &plan.summary {
                out.extend(wrapped(
                    vec![Span::raw("  ")],
                    summary,
                    Style::default().fg(Color::White),
                    width,
                ));
            }
            problem_lines(state, guided, width, &mut out);
            plan_lines(guided, &plan, width, &mut out);
        }
        (None, Some(planning)) => {
            let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
            out.push(Line::from(Span::styled(
                format!(
                    "  {spinner} planning the walk: {}",
                    progress_text(planning, 24)
                ),
                Style::default().fg(Color::Cyan),
            )));
        }
        (None, None) => out.extend(commentary_lines(
            state,
            guided.commentary.get(super::OVERVIEW_KEY),
            width,
        )),
    }
    out.push(Line::from(""));
    for text in guided.overview.lines() {
        out.extend(wrapped(Vec::new(), text, LABEL, width));
    }
    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled("\u{2192}", KEY),
        Span::raw(" starts the walk at the first file"),
    ]));
    out
}

/// Every finding still open, worst first, each leading to its line; and
/// while the files are read, how far that has got.
fn problem_lines(state: &AppState, guided: &Guided, width: u16, out: &mut OverviewLines) {
    out.push(Line::from(""));
    out.push(heading("Where the problems are", Color::Gray));
    let open = guided.open_findings();
    let (settled, total) = guided.files_settled();
    if state.ai_assist && settled < total {
        let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
        out.push(Line::from(Span::styled(
            format!("  {spinner} reading {settled}/{total} file(s); what they find shows here"),
            Style::default().fg(Color::Cyan),
        )));
    } else if open.is_empty() && guided.final_done() {
        out.push(Line::from(Span::styled(
            "  \u{2713} nothing open",
            Style::default().fg(Color::Green),
        )));
    }
    let any = !open.is_empty();
    let path_room = usize::from(width / 2);
    for finding in open {
        let path = &guided.steps[finding.step - 1].path;
        let severity = finding.finding.severity;
        let lead = vec![
            Span::raw("  "),
            Span::styled(
                severity_mark(severity),
                Style::default().fg(severity_color(severity)),
            ),
            Span::styled(
                format!(
                    "{}:L{} ",
                    truncate_left(path, path_room),
                    finding.finding.line
                ),
                Style::default().fg(Color::White),
            ),
        ];
        let text = finding.finding.text.clone();
        out.push_to(
            wrapped(lead, &text, Style::default(), width),
            Some(OverviewTarget::Finding(finding)),
        );
    }
    if !any {
        return;
    }
    out.push(Line::from(Span::styled(
        "  click one to go to it \u{b7} \u{2193} walks them worst first \u{b7} C to fix them with Claude",
        LABEL,
    )));
}

/// The plan the walk modes order the files by, as the overview shows it.
fn plan_lines(guided: &Guided, plan: &Plan, width: u16, out: &mut OverviewLines) {
    let file = |path: &str| {
        guided
            .steps
            .iter()
            .position(|step| step.path == path)
            .map(|index| OverviewTarget::File(index + 1))
    };
    if !plan.watch.is_empty() {
        out.push(Line::from(""));
        out.push(heading("Worth your attention", Color::Gray));
        for watch in &plan.watch {
            // The longest path it names, so `a/b.rs` wins over `b.rs`.
            let named = guided
                .steps
                .iter()
                .filter(|step| watch.contains(&step.path))
                .max_by_key(|step| step.path.len())
                .and_then(|step| file(&step.path));
            out.push_to(
                wrapped(
                    vec![Span::styled(
                        "  \u{2192} ",
                        Style::default().fg(Color::Cyan),
                    )],
                    watch,
                    Style::default(),
                    width,
                ),
                named,
            );
        }
    }
    let mut risky: Vec<(&String, &(Risk, String))> = plan
        .risk
        .iter()
        .filter(|(_, (risk, _))| *risk != Risk::Low)
        .collect();
    risky.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    if !risky.is_empty() {
        out.push(Line::from(""));
        out.push(heading("Risk", Color::Gray));
        let path_room = usize::from(width / 2);
        for (path, (risk, why)) in risky {
            let lead = vec![
                Span::styled(
                    format!("  \u{25cf} {} ", risk.word()),
                    Style::default().fg(risk_color(*risk)),
                ),
                Span::styled(
                    format!("{} ", truncate_left(path, path_room)),
                    Style::default().fg(Color::White),
                ),
            ];
            out.push_to(
                wrapped(lead, &format!("\u{2014} {why}"), LABEL, width),
                file(path),
            );
        }
    }
    out.push(Line::from(""));
    out.push(heading("Concerns", Color::Gray));
    for (at, concern) in plan.concerns.iter().enumerate() {
        out.extend(wrapped(
            vec![Span::raw("  ")],
            &format!("{}. {}", at + 1, concern.title),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            width,
        ));
        if let Some(why) = &concern.why {
            out.extend(wrapped(
                vec![Span::raw("     ")],
                why,
                Style::default(),
                width,
            ));
        }
        out.extend(wrapped(
            vec![Span::raw("     ")],
            &concern.paths.join(", "),
            LABEL,
            width,
        ));
    }
    if !plan.skim.is_empty() {
        let mut skim: Vec<(&String, &String)> = plan.skim.iter().collect();
        skim.sort();
        out.push(Line::from(""));
        out.push(heading("Safe to skim (S marks them reviewed)", Color::Gray));
        for (path, why) in skim {
            out.extend(wrapped(
                vec![Span::raw("  ")],
                &format!("{path} \u{2014} {why}"),
                LABEL,
                width,
            ));
        }
    }
    for rest in plan.rest.iter().filter(|rest| !rest.trim().is_empty()) {
        out.extend(wrapped(vec![Span::raw("  ")], rest.trim(), LABEL, width));
    }
    let walk: WalkMode = guided.walk();
    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled(format!("Walk by {}", walk.label().to_lowercase()), KEY),
        Span::styled(
            format!(": {} \u{b7} M or the badge switches", walk.describe()),
            LABEL,
        ),
    ]));
}

fn heading(text: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

/// `text` after `lead`, wrapped to `width` with each further row indented
/// under the text, so a row on screen is a line here and a click's row
/// finds its line.
fn wrapped(lead: Vec<Span<'static>>, text: &str, style: Style, width: u16) -> Vec<Line<'static>> {
    let lead_width: usize = lead.iter().map(Span::width).sum();
    let indent = " ".repeat(lead_width);
    let room = usize::from(width).saturating_sub(lead_width);
    let mut lead = Some(lead);
    wrap_words(text, room)
        .into_iter()
        .map(|chunk| {
            let mut spans = lead
                .take()
                .unwrap_or_else(|| vec![Span::raw(indent.clone())]);
            spans.push(Span::styled(chunk, style));
            Line::from(spans)
        })
        .collect()
}
