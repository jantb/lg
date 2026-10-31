//! The step list down the left: the overview, each file or finding, the
//! final review, and the badges and button around it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::git::guided::{GuidedFile, WalkMode};
use crate::panel::guided::findings::Severity;
use crate::panel::guided::plan::{Plan, Risk};
use crate::panel::guided::tree::Entry;
use crate::panel::guided::{Commentary, Guided, OVERVIEW, OpenFinding, Source, Verdict, tree};

use super::{
    CURSOR_BG, ISSUE, LABEL, NIT, NOTE, modal_rows, no_text_diff, severity_color, severity_mark,
    truncate_left, verdict_look,
};

/// Where the scope badge opening the progress line is, which a click switches.
pub(in crate::panel::guided) fn scope_badge_area(guided: &Guided, area: Rect) -> Option<Rect> {
    let (_, rows) = modal_rows(guided, area)?;
    let width = u16::try_from(scope_badge(&guided.source).width()).unwrap_or(u16::MAX);
    Some(Rect {
        width: width.min(rows[0].width),
        ..rows[0]
    })
}

/// Where the walk mode's badge is, after the scope badge; a click moves to
/// the next mode.
pub(in crate::panel::guided) fn walk_badge_area(guided: &Guided, area: Rect) -> Option<Rect> {
    let scope = scope_badge_area(guided, area)?;
    let x = scope.x + scope.width + 1;
    let width = u16::try_from(walk_badge(guided.walk()).width()).unwrap_or(u16::MAX);
    Some(Rect {
        x,
        width: width.min((scope.x + area.width).saturating_sub(x)),
        ..scope
    })
}

pub(super) fn walk_badge(walk: WalkMode) -> Span<'static> {
    Span::styled(
        format!(" {} \u{25b8} ", walk.label()),
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
    )
}

/// The step list without the reset button under it.
pub(in crate::panel::guided) fn steps_area(list: Rect) -> Rect {
    Rect {
        height: list.height.saturating_sub(1),
        ..list
    }
}

/// The last row of the step list.
pub(in crate::panel::guided) fn reset_button_area(list: Rect) -> Rect {
    Rect {
        y: list.y + list.height.saturating_sub(1),
        height: list.height.min(1),
        ..list
    }
}

/// The narrowest the step list is dragged to, and the least the hunk beside
/// it keeps.
const MIN_LIST_WIDTH: u16 = 12;

const MIN_SIDE_WIDTH: u16 = 30;

/// The step list's width in a modal `inner_width` wide: as wide as it was
/// dragged, or a quarter of the modal, and never so wide the hunk is squeezed.
pub(in crate::panel::guided) fn list_width(inner_width: u16, requested: Option<u16>) -> u16 {
    let widest = inner_width
        .saturating_sub(1 + MIN_SIDE_WIDTH)
        .max(MIN_LIST_WIDTH);
    requested
        .unwrap_or((inner_width / 4).clamp(24, 44))
        .clamp(MIN_LIST_WIDTH, widest)
}

/// One row of the step list.
enum ListRow {
    Overview,
    /// A directory of the file tree, or a word on what the list holds.
    Heading {
        text: String,
        depth: usize,
    },
    /// A concern of the plan, by its place in the plan, with the button that
    /// reads all of its files.
    Concern {
        index: usize,
        title: String,
    },
    File {
        step: usize,
        name: String,
        depth: usize,
    },
    /// A band of the list: the findings of one severity in issues mode, the
    /// files of one risk in triage.
    Band {
        text: String,
        color: Color,
    },
    Finding(OpenFinding),
    Final,
}

/// What a click on a row of the step list does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::panel::guided) enum RowTarget {
    Step(usize),
    /// The file of a finding, with the cursor on its line.
    Finding(OpenFinding),
    /// A concern heading, whose button reads its files.
    Concern(usize),
}

/// The step list as the walk mode lays it out: by concern, each concern's
/// files as their tree under it, or the whole file tree. Without a plan yet,
/// the file tree.
fn list_rows(guided: &Guided, plan: Option<&Plan>) -> Vec<ListRow> {
    let tree_rows = |paths: &[&str], depth: usize| {
        let mut files: Vec<(usize, &str)> = paths
            .iter()
            .filter_map(|path| {
                let at = guided.steps.iter().position(|step| step.path == *path)?;
                Some((at + 1, guided.steps[at].path.as_str()))
            })
            .collect();
        files.sort_by_key(|(_, path)| *path);
        tree::entries(&files)
            .into_iter()
            .map(move |entry| match entry {
                Entry::Dir { name, depth: d } => ListRow::Heading {
                    text: name,
                    depth: depth + d,
                },
                Entry::File {
                    step,
                    name,
                    depth: d,
                } => ListRow::File {
                    step,
                    name,
                    depth: depth + d,
                },
            })
            .collect::<Vec<_>>()
    };
    let mut rows = vec![ListRow::Overview];
    if guided.walk() == WalkMode::Issues {
        rows.extend(issue_rows(guided));
        rows.push(ListRow::Final);
        return rows;
    }
    match plan {
        Some(plan) => {
            for (index, concern) in plan.concerns.iter().enumerate() {
                rows.push(ListRow::Concern {
                    index,
                    title: concern.title.clone(),
                });
                let paths: Vec<&str> = concern.paths.iter().map(String::as_str).collect();
                rows.extend(tree_rows(&paths, 1));
            }
        }
        None => {
            let paths: Vec<&str> = guided.steps.iter().map(|step| step.path.as_str()).collect();
            rows.extend(tree_rows(&paths, 0));
        }
    }
    rows.push(ListRow::Final);
    rows
}

/// The step each row of the step list stands for, from its first row; a
/// heading stands for none. With the first row shown, as [`draw_steps`]
/// scrolls the list to keep the current step in the middle.
/// Issues mode's list: the open findings of what has been read, grouped by
/// severity, worst first.
fn issue_rows(guided: &Guided) -> Vec<ListRow> {
    let open = guided.open_findings();
    let read = guided
        .steps
        .iter()
        .filter(|step| guided.is_read(step))
        .count();
    let mut rows = Vec::new();
    if read < guided.total() {
        rows.push(ListRow::Heading {
            text: format!(
                "{read}/{} files read; \u{25b6} read a concern to add its findings",
                guided.total()
            ),
            depth: 0,
        });
    }
    for severity in Severity::ALL {
        let group: Vec<&OpenFinding> = open
            .iter()
            .filter(|open| open.finding.severity == severity)
            .collect();
        if group.is_empty() {
            continue;
        }
        rows.push(ListRow::Band {
            text: format!(
                "{}{} ({})",
                severity_mark(severity),
                group_name(severity),
                group.len()
            ),
            color: severity_color(severity),
        });
        rows.extend(group.into_iter().cloned().map(ListRow::Finding));
    }
    if open.is_empty() && read > 0 {
        rows.push(ListRow::Heading {
            text: "nothing open in what is read".to_string(),
            depth: 0,
        });
    }
    rows
}

/// What a click on each row of the step list does, from its first row; a
/// heading does nothing. With the first row shown: where the wheel scrolled
/// the list, else as [`draw_steps`] keeps the step or finding on screen in
/// the middle.
pub(in crate::panel::guided) fn row_targets(
    guided: &Guided,
    height: u16,
) -> (Vec<Option<RowTarget>>, usize) {
    let rows: Vec<Option<RowTarget>> = list_rows(guided, guided.plan().as_ref())
        .into_iter()
        .map(|row| match row {
            ListRow::Overview => Some(RowTarget::Step(OVERVIEW)),
            ListRow::Heading { .. } | ListRow::Band { .. } => None,
            ListRow::Concern { index, .. } => Some(RowTarget::Concern(index)),
            ListRow::File { step, .. } => Some(RowTarget::Step(step)),
            ListRow::Finding(open) => Some(RowTarget::Finding(open)),
            ListRow::Final => Some(RowTarget::Step(guided.final_step())),
        })
        .collect();
    let selected = rows
        .iter()
        .position(|row| match row {
            Some(RowTarget::Step(step)) => *step == guided.current,
            Some(RowTarget::Finding(open)) => is_here(guided, open),
            _ => false,
        })
        .unwrap_or(0);
    let last = rows.len().saturating_sub(usize::from(height));
    let offset = guided
        .list_scroll
        .unwrap_or_else(|| selected.saturating_sub(height as usize / 2))
        .min(last);
    (rows, offset)
}

fn is_here(guided: &Guided, open: &OpenFinding) -> bool {
    open.step == guided.current && open.at == Some(guided.cursor)
}

/// The badge at the right end of a concern's heading: how many of its files
/// are read, then what is open in them.
fn concern_button(guided: &Guided, paths: &[String]) -> Span<'static> {
    let (read, total) = guided.read_of(paths);
    if read == total {
        let open = |severity| {
            paths
                .iter()
                .map(|path| guided.open_in_file(path, severity))
                .sum::<usize>()
        };
        let (issues, nits) = (open(Severity::Issue), open(Severity::Nit));
        return match (issues, nits) {
            (0, 0) => Span::styled(" \u{2713} ", Style::default().fg(Color::Green)),
            (0, nits) => Span::styled(format!(" \u{25cb}{nits} "), Style::default().fg(NIT)),
            (issues, _) => Span::styled(format!(" \u{25cf}{issues} "), Style::default().fg(ISSUE)),
        };
    }
    Span::styled(
        format!(" {read}/{total} "),
        Style::default().fg(Color::Cyan),
    )
}

pub(super) fn scope_badge(source: &Source) -> Span<'static> {
    let (text, color) = match source {
        Source::Branch { base_ref, .. } => (format!(" WHOLE BRANCH vs {base_ref} "), Color::Blue),
        Source::Unpushed { base_ref, .. } if base_ref.is_empty() => {
            (" NOT PUSHED ".to_string(), Color::Magenta)
        }
        Source::Unpushed { base_ref, .. } => {
            (format!(" NOT PUSHED vs {base_ref} "), Color::Magenta)
        }
        Source::Codebase { .. } => (" WHOLE CODEBASE ".to_string(), Color::Green),
        Source::PullRequest { number, .. } => (format!(" PR #{number} "), Color::Cyan),
    };
    Span::styled(
        text,
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

/// The steps, one row each: the overview, each file, the final review.
pub(super) fn draw_steps(guided: &Guided, area: Rect, frame: &mut Frame) {
    let area = steps_area(area);
    let width = area.width as usize;
    let plan = guided.plan();
    let selected = |here: bool| {
        if here {
            Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    };
    let items: Vec<ListItem> = list_rows(guided, plan.as_ref())
        .into_iter()
        .map(|row| match row {
            ListRow::Overview => ListItem::new(Line::from(Span::styled(
                "\u{2605} Overview",
                selected(guided.current == OVERVIEW).fg(Color::LightCyan),
            ))),
            ListRow::Heading { text, depth } => {
                let indent = "  ".repeat(depth);
                let text = truncate_left(&text, width.saturating_sub(2 + indent.len()));
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::raw(indent),
                    Span::styled(text, Style::default().fg(Color::Gray)),
                ]))
            }
            ListRow::Concern { index, title } => {
                let paths = plan
                    .as_ref()
                    .and_then(|plan| plan.concerns.get(index))
                    .map(|concern| concern.paths.clone())
                    .unwrap_or_default();
                let button = concern_button(guided, &paths);
                // The badge sits at the right edge, where the eye finds it
                // in every heading's row alike.
                let room = width.saturating_sub(2 + button.width());
                let title = truncate_left(&title, room);
                let pad = room.saturating_sub(title.chars().count());
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        title,
                        Style::default()
                            .fg(Color::LightYellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" ".repeat(pad)),
                    button,
                ]))
            }
            ListRow::Band { text, color } => ListItem::new(Line::from(Span::styled(
                format!("  {text}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ))),
            ListRow::Finding(open) => {
                finding_row(guided, &open, width, selected(is_here(guided, &open)))
            }
            ListRow::File { step, name, depth } => file_row(guided, step, &name, depth, width),
            ListRow::Final => {
                let style = selected(guided.on_final());
                let verdict = verdict_mark(
                    guided.commentary.get(&guided.final_key()),
                    guided.final_verdict(),
                );
                ListItem::new(Line::from(vec![
                    verdict,
                    Span::styled("\u{25c6} Final review", style.fg(Color::LightCyan)),
                    Span::styled(" whole change", LABEL),
                ]))
                .style(style)
            }
        })
        .collect();
    let (_, offset) = row_targets(guided, area.height);
    let list = List::new(items.into_iter().skip(offset).collect::<Vec<_>>());
    frame.render_widget(list, area);
}

fn group_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Issue => "Issues",
        Severity::Nit => "Nits",
        Severity::Question => "Questions",
    }
}

/// An open finding's row: where it is, then what it says, cut to fit.
fn finding_row(
    guided: &Guided,
    open: &OpenFinding,
    width: usize,
    style: Style,
) -> ListItem<'static> {
    let path = &guided.steps[open.step - 1].path;
    let name = path.rsplit('/').next().unwrap_or(path);
    let whole = if open.whole { "\u{25c6}" } else { "" };
    let place = format!("    {whole}{name}:{} ", open.finding.line);
    let room = width.saturating_sub(place.chars().count());
    let text: String = if open.finding.text.chars().count() > room {
        let kept: String = open
            .finding
            .text
            .chars()
            .take(room.saturating_sub(1))
            .collect();
        format!("{kept}\u{2026}")
    } else {
        open.finding.text.clone()
    };
    ListItem::new(Line::from(vec![
        Span::styled(
            place,
            Style::default().fg(severity_color(open.finding.severity)),
        ),
        Span::styled(text, Style::default().fg(Color::White)),
    ]))
    .style(style)
}

/// A file's row: its verdict in the first column, where a narrow list cannot
/// cut it off; its name; then its notes and open findings.
fn file_row(
    guided: &Guided,
    number: usize,
    name: &str,
    depth: usize,
    width: usize,
) -> ListItem<'static> {
    let step = &guided.steps[number - 1];
    let here = guided.current == number;
    let mark = if guided.is_reviewed(step) {
        Span::styled("\u{2713} ", Style::default().fg(Color::Green))
    } else if here {
        Span::styled("\u{25b6} ", Style::default().fg(Color::LightCyan))
    } else {
        Span::styled("\u{b7} ", LABEL)
    };
    let verdict = verdict_mark(guided.commentary.get(&step.key()), guided.verdict(step));
    let base = match (here, guided.is_reviewed(step)) {
        (true, _) => Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD),
        (false, true) => LABEL,
        (false, false) => Style::default().fg(Color::White),
    };
    let mut tail = Vec::new();
    let notes = guided.notes_for(step).len();
    if notes > 0 {
        tail.push(Span::styled(format!(" \u{270e}{notes}"), NOTE));
    }
    tail.extend(file_tally(guided, step));
    // The name first; what happened to a file with no text diff after
    // everything else, where a narrow list cuts it off first.
    let indent = "  ".repeat(depth);
    let used = 4 + indent.len() + tail.iter().map(Span::width).sum::<usize>();
    if let Some(what) = no_text_diff(step) {
        tail.push(Span::styled(format!(" {what}"), LABEL));
    }
    let label = Span::styled(truncate_left(name, width.saturating_sub(used)), base);
    let mut spans = vec![verdict, Span::raw(indent), mark];
    spans.push(label);
    spans.extend(tail);
    ListItem::new(Line::from(spans)).style(base)
}

pub(super) fn risk_color(risk: Risk) -> Color {
    match risk {
        Risk::High => ISSUE,
        Risk::Medium => NIT,
        Risk::Low => Color::Green,
    }
}

/// A step's verdict as the list's first column shows it.
fn verdict_mark(commentary: Option<&Commentary>, verdict: Option<Verdict>) -> Span<'static> {
    match commentary {
        Some(c) if c.error.is_some() => Span::styled("\u{26a0} ", LABEL),
        Some(c) => match verdict {
            Some(verdict) => {
                Span::styled("\u{25cf} ", Style::default().fg(verdict_look(verdict).1))
            }
            None if !c.done => Span::styled("\u{2026} ", LABEL),
            None => Span::raw("  "),
        },
        None => Span::raw("  "),
    }
}

/// Starts the review over; armed by the first click, it asks for a second.
pub(super) fn draw_reset_button(guided: &Guided, area: Rect, frame: &mut Frame) {
    let (text, style) = if guided.reset_armed {
        (
            " \u{27f2} click again to reset ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            " \u{27f2} Reset review ",
            Style::default().fg(Color::Gray).bg(Color::Rgb(50, 50, 50)),
        )
    };
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
}

/// The findings still open in a file by severity, or a tick once it has been
/// read and nothing is open.
fn file_tally(guided: &Guided, step: &GuidedFile) -> Vec<Span<'static>> {
    let read = guided.verdict(step).is_some();
    let mut tally: Vec<Span<'static>> = Severity::ALL
        .into_iter()
        .map(|severity| (severity, guided.open_in_file(&step.path, severity)))
        .filter(|(_, open)| *open > 0)
        .map(|(severity, open)| {
            Span::styled(
                format!(" {}{open}", severity_mark(severity).trim_end()),
                Style::default().fg(severity_color(severity)),
            )
        })
        .collect();
    if tally.is_empty() && read {
        tally.push(Span::styled(" \u{2713}", Style::default().fg(Color::Green)));
    }
    tally
}
