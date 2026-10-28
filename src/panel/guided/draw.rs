//! Drawing a guided review: the steps down the left, the file on screen with
//! its notes, and the model's read of it beneath; on the final step, the whole
//! changeset with the model's read of it as one change.

use ratatui::{
    Frame,
    layout::{Constraint, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::findings::{self, Finding, Severity};
use super::layout::{self, Row};
use super::plan::{Plan, Risk};
use super::tree::{self, Entry};
use super::{Commentary, Guided, Mode, OVERVIEW, OpenFinding, Progress, Source, Verdict};
use crate::git::guided::{GuidedFile, HunkLine, LineKind, Side, TickedFinding, WalkMode};
use crate::panel::keys;
use crate::state::{AppState, DiffViewMode, SPINNER_FRAMES};
use crate::ui::{self, DiffContentKind};

const LABEL: Style = Style::new().fg(Color::DarkGray);
const KEY: Style = Style::new().fg(Color::Yellow);
const NOTE: Style = Style::new().fg(Color::LightMagenta);
const CURSOR_BG: Color = Color::Rgb(40, 46, 60);
const ISSUE: Color = Color::LightRed;
const NIT: Color = Color::Yellow;
const QUESTION: Color = Color::LightCyan;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    // The whole screen but the footer, where the status line reports what
    // the last action did.
    let modal = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let title = format!("Guided review \u{b7} {}", guided.source.title());
    let inner = ui::modal_frame(frame, modal, &title);
    if inner.width < 50 || inner.height < 10 {
        frame.render_widget(
            Paragraph::new("Terminal too small for a guided review"),
            inner,
        );
        return;
    }
    let (rows, mut dividers) = ui::modal_rows(
        frame,
        inner,
        &[
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(bottom_height(guided)),
        ],
    );
    draw_progress(state, guided, rows[0], frame);
    let list_width = list_width(inner.width, state.guided_list_width);
    let (panes, gaps) = ui::modal_columns(
        frame,
        rows[1],
        &[Constraint::Length(list_width), Constraint::Min(30)],
    );
    dividers.extend(gaps);
    draw_steps(guided, panes[0], frame);
    draw_reset_button(guided, reset_button_area(panes[0]), frame);
    match &guided.mode {
        Mode::Summary { event, body } => draw_summary(guided, *event, body, panes[1], frame),
        _ => draw_step(state, guided, panes[1], frame, &mut dividers),
    }
    draw_bottom(guided, rows[2], frame);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

/// Where the step list and the pane beside it land, measured the way
/// [`render`] lays them out with the list `list_width` asked for; `None`
/// when the terminal is too small to draw either.
pub(super) fn pane_areas(
    guided: &Guided,
    area: Rect,
    requested_list_width: Option<u16>,
) -> Option<(Rect, Rect)> {
    let (inner, rows) = modal_rows(guided, area)?;
    let list_width = list_width(inner.width, requested_list_width);
    let (panes, _) = ui::modal_column_areas(
        rows[1],
        &[Constraint::Length(list_width), Constraint::Min(30)],
    );
    Some((panes[0], panes[1]))
}

/// The modal's inside and its rows: progress, panes, the bottom line.
fn modal_rows(guided: &Guided, area: Rect) -> Option<(Rect, Vec<Rect>)> {
    let modal = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let inner = ui::modal_inner(modal);
    if inner.width < 50 || inner.height < 10 {
        return None;
    }
    let (rows, _) = ui::modal_row_areas(
        inner,
        &[
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(bottom_height(guided)),
        ],
    );
    Some((inner, rows.to_vec()))
}

/// Where the scope badge opening the progress line is, which a click switches.
pub(super) fn scope_badge_area(guided: &Guided, area: Rect) -> Option<Rect> {
    let (_, rows) = modal_rows(guided, area)?;
    let width = u16::try_from(scope_badge(&guided.source).width()).unwrap_or(u16::MAX);
    Some(Rect {
        width: width.min(rows[0].width),
        ..rows[0]
    })
}

/// A long read's progress as words, with a bar `width` wide where there is a
/// measure: how long reading the prompt has taken against how long it
/// should, then how many files the plan has placed.
fn progress_text(progress: Progress, width: usize) -> String {
    match progress {
        Progress::Queued { ahead: 0 } => "starting".to_string(),
        Progress::Queued { ahead } => format!("queued behind {ahead} read(s) already running"),
        Progress::Reading {
            elapsed,
            expected: Some(expected),
        } => {
            let done = elapsed.as_millis().min(expected.as_millis());
            format!(
                "{} reading, {}s of ~{}s",
                bar(done as usize, expected.as_millis().max(1) as usize, width),
                elapsed.as_secs(),
                expected.as_secs().max(1)
            )
        }
        Progress::Reading {
            elapsed,
            expected: None,
        } => format!("reading, {}s", elapsed.as_secs()),
        Progress::Thinking { elapsed } => format!("thinking, {}s", elapsed.as_secs()),
        Progress::Placing { named, total } => format!(
            "{} {named}/{total} files placed",
            bar(named, total.max(1), width)
        ),
        Progress::Finding { so_far } => format!("writing, {so_far} finding(s) so far"),
    }
}

fn bar(done: usize, total: usize, width: usize) -> String {
    let filled = (done * width / total).min(width);
    format!(
        "{}{}",
        "\u{2593}".repeat(filled),
        "\u{2591}".repeat(width - filled)
    )
}

/// Where the walk mode's badge is, after the scope badge; a click moves to
/// the next mode.
pub(super) fn walk_badge_area(guided: &Guided, area: Rect) -> Option<Rect> {
    let scope = scope_badge_area(guided, area)?;
    let x = scope.x + scope.width + 1;
    let width = u16::try_from(walk_badge(guided.walk()).width()).unwrap_or(u16::MAX);
    Some(Rect {
        x,
        width: width.min((scope.x + area.width).saturating_sub(x)),
        ..scope
    })
}

fn walk_badge(walk: WalkMode) -> Span<'static> {
    Span::styled(
        format!(" {} \u{25b8} ", walk.label()),
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
    )
}

/// The step list without the reset button under it.
pub(super) fn steps_area(list: Rect) -> Rect {
    Rect {
        height: list.height.saturating_sub(1),
        ..list
    }
}

/// The last row of the step list.
pub(super) fn reset_button_area(list: Rect) -> Rect {
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
pub(super) fn list_width(inner_width: u16, requested: Option<u16>) -> u16 {
    let widest = inner_width
        .saturating_sub(1 + MIN_SIDE_WIDTH)
        .max(MIN_LIST_WIDTH);
    requested
        .unwrap_or((inner_width / 4).clamp(24, 44))
        .clamp(MIN_LIST_WIDTH, widest)
}

/// Triage's bands, by [`super::triage_rank`]: riskiest first, then the files
/// the plan rated no risk for, then those safe to skim.
const RISK_BANDS: [(&str, Color); 5] = [
    ("\u{25cf} High risk", ISSUE),
    ("\u{25cf} Medium risk", NIT),
    ("\u{25cf} Low risk", Color::Green),
    ("\u{b7} Not rated", Color::Gray),
    (
        "\u{25cb} Safe to skim (S marks them reviewed)",
        Color::DarkGray,
    ),
];

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
pub(super) enum RowTarget {
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
    if let Some(plan) = plan.filter(|_| guided.walk() == WalkMode::Triage) {
        for (rank, (text, color)) in RISK_BANDS.iter().enumerate() {
            let paths: Vec<&str> = guided
                .steps
                .iter()
                .map(|step| step.path.as_str())
                .filter(|path| super::triage_rank(plan, path) == rank)
                .collect();
            if paths.is_empty() {
                continue;
            }
            rows.push(ListRow::Band {
                text: format!("{text} ({})", paths.len()),
                color: *color,
            });
            rows.extend(tree_rows(&paths, 1));
        }
        rows.push(ListRow::Final);
        return rows;
    }
    match plan.filter(|_| guided.walk() == WalkMode::Concerns) {
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
pub(super) fn row_targets(guided: &Guided, height: u16) -> (Vec<Option<RowTarget>>, usize) {
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

/// The button at the right end of a concern's heading: reads every file of
/// it, then counts them in, then tallies what is open in them.
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
    if guided.reading_ahead(paths) {
        return Span::styled(
            format!(" {read}/{total} "),
            Style::default().fg(Color::Cyan),
        );
    }
    Span::styled(
        " \u{25b6} read all ",
        Style::default().fg(Color::White).bg(Color::Rgb(50, 50, 50)),
    )
}

/// Whether `column` is on the button of concern `index`'s heading in a list
/// drawn in `list`.
pub(super) fn on_concern_button(guided: &Guided, list: Rect, index: usize, column: u16) -> bool {
    let Some(concern) = guided
        .plan()
        .and_then(|plan| plan.concerns.into_iter().nth(index))
    else {
        return false;
    };
    let width = u16::try_from(concern_button(guided, &concern.paths).width()).unwrap_or(0);
    let end = list.x + list.width;
    column >= end.saturating_sub(width) && column < end
}

fn bottom_height(guided: &Guided) -> u16 {
    match &guided.mode {
        Mode::Note { text, .. } | Mode::Question { text } | Mode::Fix { text, .. } => {
            (text.lines().count().max(1) as u16 + 1).min(6)
        }
        _ => 1,
    }
}

/// One line: where the walk is, how much of it is done, and what is running.
fn draw_progress(state: &AppState, guided: &Guided, area: Rect, frame: &mut Frame) {
    let total = guided.total();
    let reviewed = guided.reviewed_count();
    let bar_width = 20usize;
    let filled = (reviewed * bar_width).checked_div(total).unwrap_or(0);
    let mut spans = vec![
        scope_badge(&guided.source),
        Span::raw(" "),
        walk_badge(guided.walk()),
        Span::raw(" "),
        Span::styled(
            if guided.current == OVERVIEW {
                format!("overview \u{b7} {total} files ")
            } else if guided.on_final() {
                format!("final review \u{b7} {total} files ")
            } else {
                format!("file {}/{total} ", guided.current)
            },
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{2588}".repeat(filled), Style::default().fg(Color::Green)),
        Span::styled(
            "\u{2591}".repeat(bar_width - filled),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(format!(" {reviewed} reviewed"), LABEL),
        Span::styled(
            format!(" \u{b7} {} note(s)", guided.progress.notes.len()),
            NOTE,
        ),
    ];
    spans.extend(verdict_tally(guided));
    let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
    if let Some(planning) = guided.planning() {
        spans.push(Span::styled(
            format!("  {spinner} planning {}", progress_text(planning, 10)),
            Style::default().fg(Color::Cyan),
        ));
    }
    if let Some(progress) = guided.final_progress().filter(|_| state.ai_assist) {
        spans.push(Span::styled(
            format!("  {spinner} final review {}", progress_text(progress, 10)),
            Style::default().fg(Color::Cyan),
        ));
    }
    let wanted: Vec<String> = guided
        .steps
        .iter()
        .filter(|step| guided.wants_read(&step.path))
        .map(|step| step.path.clone())
        .collect();
    let (read, total) = guided.read_of(&wanted);
    if read < total {
        spans.push(Span::styled(
            format!("  {spinner} reading {read}/{total} file(s)"),
            Style::default().fg(Color::Cyan),
        ));
    }
    if guided.loading.is_some() {
        spans.push(Span::styled(
            format!("  {spinner} reading the change"),
            Style::default().fg(Color::Cyan),
        ));
    }
    if let Some((_, what)) = &guided.fixing {
        spans.push(Span::styled(
            format!("  {spinner} claude is changing {what}"),
            Style::default().fg(Color::Cyan),
        ));
    }
    if !state.ai_assist {
        spans.push(Span::styled("  AI off", LABEL));
    } else {
        spans.push(Span::styled(
            format!(
                "  {} \u{b7} {}",
                state.llm_provider.label(),
                state.llm_model
            ),
            LABEL,
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn verdict_tally(guided: &Guided) -> Vec<Span<'static>> {
    let verdicts: Vec<Verdict> = guided
        .steps
        .iter()
        .filter_map(|step| guided.verdict(step))
        .collect();
    [Verdict::Issue, Verdict::Nit, Verdict::Ok]
        .into_iter()
        .map(|verdict| (verdict, verdicts.iter().filter(|v| **v == verdict).count()))
        .filter(|(_, count)| *count > 0)
        .map(|(verdict, count)| {
            let (word, color) = verdict_look(verdict);
            Span::styled(
                format!("  \u{25cf} {count} {word}"),
                Style::default().fg(color),
            )
        })
        .collect()
}

fn scope_badge(source: &Source) -> Span<'static> {
    let (text, color) = match source {
        Source::Branch { base_ref, .. } => (format!(" WHOLE BRANCH vs {base_ref} "), Color::Blue),
        Source::Unpushed { base_ref, .. } if base_ref.is_empty() => {
            (" NOT PUSHED ".to_string(), Color::Magenta)
        }
        Source::Unpushed { base_ref, .. } => {
            (format!(" NOT PUSHED vs {base_ref} "), Color::Magenta)
        }
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
fn draw_steps(guided: &Guided, area: Rect, frame: &mut Frame) {
    let area = steps_area(area);
    let width = area.width as usize;
    let plan = guided.plan();
    let order = guided.walk_order();
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
                // The button sits at the right edge, where the eye finds it
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
            ListRow::File { step, name, depth } => {
                file_row(guided, plan.as_ref(), &order, step, &name, depth, width)
            }
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
/// cut it off; its place in the reading order on the map; its name; then its
/// notes and open findings.
fn file_row(
    guided: &Guided,
    plan: Option<&Plan>,
    order: &[usize],
    number: usize,
    name: &str,
    depth: usize,
    width: usize,
) -> ListItem<'static> {
    let step = &guided.steps[number - 1];
    let here = guided.current == number;
    let triage = guided.walk() == WalkMode::Triage;
    let skim = plan.is_some_and(|plan| plan.skim.contains_key(&step.path));
    let mark = if guided.is_reviewed(step) {
        Span::styled("\u{2713} ", Style::default().fg(Color::Green))
    } else if here {
        Span::styled("\u{25b6} ", Style::default().fg(Color::LightCyan))
    } else {
        Span::styled("\u{b7} ", LABEL)
    };
    let verdict = verdict_mark(guided.commentary.get(&step.key()), guided.verdict(step));
    let base = match (here, guided.is_reviewed(step) || (triage && skim)) {
        (true, _) => Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD),
        (false, true) => LABEL,
        (false, false) => Style::default().fg(Color::White),
    };
    let place = match (plan, guided.walk()) {
        (Some(_), WalkMode::Map) => order
            .iter()
            .position(|&at| at == number)
            .map(|at| Span::styled(format!("{} ", at + 1), Style::default().fg(Color::Cyan))),
        _ => None,
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
    let used = 4
        + indent.len()
        + place.as_ref().map_or(0, Span::width)
        + tail.iter().map(Span::width).sum::<usize>();
    if let Some(what) = no_text_diff(step) {
        tail.push(Span::styled(format!(" {what}"), LABEL));
    }
    let label = Span::styled(truncate_left(name, width.saturating_sub(used)), base);
    let mut spans = vec![verdict, Span::raw(indent), mark];
    spans.extend(place);
    spans.push(label);
    spans.extend(tail);
    ListItem::new(Line::from(spans)).style(base)
}

fn risk_color(risk: Risk) -> Color {
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
fn draw_reset_button(guided: &Guided, area: Rect, frame: &mut Frame) {
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

/// What happened to a file the diff has no text for.
fn no_text_diff(step: &GuidedFile) -> Option<String> {
    step.lines.is_empty().then(|| {
        step.file_note
            .clone()
            .unwrap_or_else(|| "no text diff".into())
    })
}

fn verdict_look(verdict: Verdict) -> (&'static str, Color) {
    match verdict {
        Verdict::Ok => ("ok", Color::Green),
        Verdict::Nit => ("nit", NIT),
        Verdict::Issue => ("issue", Color::Red),
    }
}

fn severity_mark(severity: Severity) -> &'static str {
    match severity {
        Severity::Issue => "\u{25cf} ",
        Severity::Nit => "\u{25cb} ",
        Severity::Question => "? ",
    }
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Question => QUESTION,
        Severity::Nit => NIT,
        Severity::Issue => ISSUE,
    }
}

fn file_label(step: &GuidedFile) -> String {
    let (added, removed) = (step.added(), step.removed());
    match no_text_diff(step) {
        Some(name) => name,
        None => format!("+{added} -{removed} \u{b7} {} hunk(s)", step.hunks()),
    }
}

/// The step on screen: its file's diff above, the model's read of it below.
fn draw_step(
    state: &AppState,
    guided: &Guided,
    area: Rect,
    frame: &mut Frame,
    dividers: &mut Vec<Rect>,
) {
    let Some(step) = guided.step() else {
        if guided.on_final() {
            draw_final(state, guided, area, frame, dividers);
        } else {
            draw_overview(state, guided, area, frame);
        }
        return;
    };
    let mode = state.diff_view_mode;
    let (parts, gaps) = step_areas(guided, step, area, mode);
    ui::draw_dividers(frame, &gaps);
    dividers.extend(gaps);
    draw_diff(guided, step, mode, parts[0], frame);
    match super::chat_session(state) {
        Some(id) => {
            let (columns, gaps) = ui::modal_columns(frame, parts[1], &CHAT_SPLIT);
            dividers.extend(gaps);
            draw_commentary(state, guided, step, columns[0], frame);
            draw_chat(state, guided, id, columns[1], frame);
        }
        None => draw_commentary(state, guided, step, parts[1], frame),
    }
}

/// The chat is left out: it takes the mouse for itself.
pub(super) fn selectable_areas(guided: &Guided, side: Rect, mode: DiffViewMode) -> Vec<Rect> {
    let Some(parts) = diff_and_below(guided, side, mode) else {
        return vec![side];
    };
    vec![parts[0], commentary_area(guided, parts[1])]
}

const CHAT_SPLIT: [Constraint; 2] = [Constraint::Percentage(40), Constraint::Percentage(60)];

/// The diff and what sits below it, on a file's step or the final review.
fn diff_and_below(guided: &Guided, side: Rect, mode: DiffViewMode) -> Option<Vec<Rect>> {
    if guided.on_final() {
        return Some(final_areas(guided, side, mode).0);
    }
    let step = guided.step()?;
    Some(step_areas(guided, step, side, mode).0)
}

/// The model's read below the diff, left of the chat while one runs.
fn commentary_area(guided: &Guided, below: Rect) -> Rect {
    if guided.chat.is_some() {
        ui::modal_column_areas(below, &CHAT_SPLIT).0[0]
    } else {
        below
    }
}

pub(super) fn chat_screen_area(guided: &Guided, side: Rect, mode: DiffViewMode) -> Option<Rect> {
    guided.chat?;
    let parts = diff_and_below(guided, side, mode)?;
    let (columns, _) = ui::modal_column_areas(parts[1], &CHAT_SPLIT);
    let chat = columns[1];
    Some(Rect {
        y: chat.y + 1,
        height: chat.height.saturating_sub(1),
        ..chat
    })
}

fn draw_chat(
    state: &AppState,
    guided: &Guided,
    id: crate::session::SessionId,
    area: Rect,
    frame: &mut Frame,
) {
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    let focused = guided.mode == Mode::Chat;
    let (title, hint) = if focused {
        (
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            "  Ctrl-] back to the review",
        )
    } else {
        (Style::default().fg(Color::Gray), "  C or click to type")
    };
    let file = guided.chat_file.as_deref().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("Claude \u{b7} {file}"), title),
            Span::styled(hint, LABEL),
        ])),
        Rect { height: 1, ..area },
    );
    let screen = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    crate::term::render_screen(session.screen(), screen, frame.buffer_mut());
    if focused
        && let Some((row, col)) = session.cursor_position()
        && row < screen.height
        && col < screen.width
    {
        frame.set_cursor_position((screen.x + col, screen.y + row));
    }
}

/// Where the diff is drawn: the file's, or on the final step the whole changeset.
pub(super) fn diff_area(guided: &Guided, side: Rect, mode: DiffViewMode) -> Option<Rect> {
    Some(diff_and_below(guided, side, mode)?[0])
}

/// The findings sit in the diff, so it gets most of the room.
pub(super) fn step_areas(
    guided: &Guided,
    step: &GuidedFile,
    area: Rect,
    mode: DiffViewMode,
) -> (Vec<Rect>, Vec<Rect>) {
    let rows = diff_lines(guided, step, mode, area.width).lines.len();
    split_for_diff(area, rows, diff_room(guided, area))
}

/// A running chat needs room to be read, so the diff gives up more of it.
fn diff_room(guided: &Guided, area: Rect) -> u16 {
    if guided.chat.is_some() {
        area.height / 2
    } else {
        area.height * 2 / 3
    }
}

fn split_for_diff(area: Rect, rows: usize, most: u16) -> (Vec<Rect>, Vec<Rect>) {
    let diff_height = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .min(most)
        .max(area.height.min(6));
    ui::modal_row_areas(area, &[Constraint::Length(diff_height), Constraint::Min(3)])
}

fn draw_diff(
    guided: &Guided,
    step: &GuidedFile,
    mode: DiffViewMode,
    area: Rect,
    frame: &mut Frame,
) {
    let diff = diff_lines(guided, step, mode, area.width);
    let scroll = diff.scroll(area.height);
    frame.render_widget(Paragraph::new(diff.lines).scroll((scroll as u16, 0)), area);
}

pub(super) fn finding_at(
    state: &AppState,
    side: Rect,
    column: u16,
    row: u16,
) -> Option<TickedFinding> {
    let guided = state.guided.as_deref()?;
    let mode = state.diff_view_mode;
    use crate::panel::pointer::inside;
    let ticked_at = |findings: Vec<(std::ops::Range<usize>, TickedFinding)>, at: usize| {
        findings
            .into_iter()
            .find(|(rows, _)| rows.contains(&at))
            .map(|(_, finding)| finding)
    };
    if guided.on_final() {
        let (parts, _) = final_areas(guided, side, mode);
        let (diff, commentary) = (parts[0], commentary_area(guided, parts[1]));
        if inside(diff, column, row) {
            let lines = final_diff_lines(guided, mode, diff.width);
            let scroll = guided.final_scroll.min(guided.final_max.get());
            return ticked_at(lines.findings, usize::from(scroll + row - diff.y));
        }
        if inside(commentary, column, row) {
            let lines = final_commentary_lines(state, guided, commentary.width);
            let scroll = guided.side_scroll.min(guided.side_max.get());
            return ticked_at(lines.findings, usize::from(scroll + row - commentary.y));
        }
        return None;
    }
    let step = guided.step()?;
    let (parts, _) = step_areas(guided, step, side, mode);
    let area = parts[0];
    if !inside(area, column, row) {
        return None;
    }
    let diff = diff_lines(guided, step, mode, area.width);
    let at = diff.scroll(area.height) + usize::from(row - area.y);
    ticked_at(diff.findings, at)
}

struct DiffLines {
    lines: Vec<Line<'static>>,
    cursor: std::ops::Range<usize>,
    findings: Vec<(std::ops::Range<usize>, TickedFinding)>,
}

impl DiffLines {
    /// Keeps the cursor line and what is pinned under it in view.
    fn scroll(&self, height: u16) -> usize {
        (self.cursor.end + 2)
            .saturating_sub(usize::from(height))
            .min(self.cursor.start)
    }
}

fn diff_lines(guided: &Guided, step: &GuidedFile, mode: DiffViewMode, width: u16) -> DiffLines {
    let mut out = DiffLines {
        lines: vec![file_title(guided, step)],
        cursor: 0..0,
        findings: Vec::new(),
    };
    let lead = inline_lead(mode, width);
    let flagged = inline_findings(guided, step, &lead, width);
    push_diff(
        guided,
        step,
        mode,
        width,
        &flagged,
        Some(guided.cursor),
        &mut out,
    );
    out
}

/// A file's rows, with its notes and `flagged` findings under their lines,
/// added to `out`; the rows of line `cursor` become `out.cursor`.
fn push_diff(
    guided: &Guided,
    step: &GuidedFile,
    mode: DiffViewMode,
    width: u16,
    flagged: &[Inline],
    cursor: Option<usize>,
    out: &mut DiffLines,
) {
    let lines = &mut out.lines;
    let notes = guided.notes_for(step);
    let lead = inline_lead(mode, width);
    for row in layout::rows(step, mode) {
        let start = lines.len();
        let worst = flagged
            .iter()
            .filter(|inline| !inline.ticked && row.lines().any(|line| line == inline.at))
            .map(|inline| inline.finding.severity)
            .max();
        let bar = match worst {
            Some(severity) => {
                Span::styled("\u{258c}", Style::default().fg(severity_color(severity)))
            }
            None => Span::raw(" "),
        };
        let body = match row {
            Row::Whole(at) => vec![unified_line(step, at, Some(at) == cursor)],
            Row::Pair(left, right) => {
                paired_lines(step, left, right, cursor, width.saturating_sub(1))
            }
        };
        lines.extend(body.into_iter().enumerate().map(|(i, line)| {
            let lead = if i == 0 { bar.clone() } else { Span::raw(" ") };
            Line::from(std::iter::once(lead).chain(line.spans).collect::<Vec<_>>())
                .style(line.style)
        }));
        for at in row.lines() {
            let here = Some(at) == cursor;
            if let Some(anchor) = step.lines[at].anchor() {
                for (_, note) in notes
                    .iter()
                    .filter(|(_, note)| (note.line, note.side) == anchor)
                {
                    lines.extend(note_lines(&note.body, &lead));
                }
            }
            for inline in flagged.iter().filter(|inline| inline.at == at) {
                let first = lines.len();
                lines.extend(inline.lines.iter().cloned().map(|line| {
                    if here {
                        line.style(Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD))
                    } else {
                        line
                    }
                }));
                out.findings
                    .push((first..lines.len(), inline.ticket.clone()));
            }
        }
        if row.lines().any(|at| Some(at) == cursor) {
            out.cursor = start..lines.len();
        }
    }
    // Notes pinned to lines this diff no longer shows, said once at the end.
    let shown: Vec<(usize, Side)> = step.lines.iter().filter_map(|l| l.anchor()).collect();
    for (_, note) in notes
        .iter()
        .filter(|(_, n)| !shown.contains(&(n.line, n.side)))
    {
        lines.push(Line::from(vec![
            Span::styled(format!("  \u{270e} L{} (moved) ", note.line), NOTE),
            Span::styled(note.body.clone(), NOTE),
        ]));
    }
    if step.lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no text diff to show for this file",
            LABEL,
        )));
    }
}

fn file_title(guided: &Guided, step: &GuidedFile) -> Line<'static> {
    let mut title = vec![Span::styled(
        step.path.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    title.push(Span::styled(format!("  {}", file_label(step)), LABEL));
    if let Some(note) = step.file_note.as_ref().filter(|_| !step.lines.is_empty()) {
        title.push(Span::styled(
            format!("  [{note}]"),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(verdict) = guided.verdict(step) {
        let (word, color) = verdict_look(verdict);
        title.push(Span::raw("  "));
        title.push(Span::styled(
            format!(" {word} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if guided.is_reviewed(step) {
        title.push(Span::styled(
            "  reviewed",
            Style::default().fg(Color::Green),
        ));
    }
    Line::from(title)
}

fn unified_line(step: &GuidedFile, at: usize, here: bool) -> Line<'static> {
    let line = &step.lines[at];
    if line.kind == LineKind::Header {
        return Line::from(vec![
            Span::styled(format!("{:>12}", "\u{22ef} "), LABEL),
            Span::styled(line.text.clone(), Style::default().fg(Color::Cyan)),
        ]);
    }
    let number = |n: Option<usize>| n.map(|n| n.to_string()).unwrap_or_default();
    let gutter = format!("{:>5} {:>5} ", number(line.old), number(line.new));
    let spans: Vec<Span<'static>> = std::iter::once(Span::styled(gutter, LABEL))
        .chain(
            ui::highlight_diff_line_for_path(&line.text, &step.path)
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style)),
        )
        .collect();
    let rendered = Line::from(spans);
    if here {
        rendered.style(Style::default().bg(CURSOR_BG))
    } else {
        rendered
    }
}

fn paired_lines(
    step: &GuidedFile,
    left: Option<usize>,
    right: Option<usize>,
    cursor: Option<usize>,
    width: u16,
) -> Vec<Line<'static>> {
    let cell = |at: Option<usize>, number: fn(&HunkLine) -> Option<usize>| {
        let line = &step.lines[at?];
        let kind = match line.kind {
            LineKind::Context => DiffContentKind::Context,
            LineKind::Added => DiffContentKind::Added,
            LineKind::Removed => DiffContentKind::Removed,
            LineKind::Meta | LineKind::Header => return None,
        };
        Some((number(line)?, line.text.get(1..).unwrap_or_default(), kind))
    };
    let rows = ui::side_by_side_pair_for_path(
        cell(left, |line| line.old),
        cell(right, |line| line.new),
        width,
        &step.path,
    );
    let (left_width, right_start) = ui::side_by_side_columns(width);
    let at_cursor = |at: Option<usize>| at.is_some() && at == cursor;
    let lit = match (at_cursor(left), at_cursor(right)) {
        (true, true) => Some(0..width),
        (true, false) => Some(0..left_width),
        (false, true) => Some(right_start..width),
        (false, false) => None,
    };
    match lit {
        Some(columns) => rows.into_iter().map(|row| light(row, &columns)).collect(),
        None => rows,
    }
}

fn light(line: Line<'static>, columns: &std::ops::Range<u16>) -> Line<'static> {
    let mut column = 0u16;
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let start = column;
            column = column.saturating_add(u16::try_from(span.width()).unwrap_or(u16::MAX));
            if columns.contains(&start) {
                let lit = span.style.bg(CURSOR_BG);
                span.style(lit)
            } else {
                span
            }
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn note_lines(body: &str, lead: &str) -> Vec<Line<'static>> {
    body.lines()
        .enumerate()
        .map(|(i, text)| {
            let mark = if i == 0 { "\u{270e} " } else { "  " };
            Line::from(vec![
                Span::styled(format!("{lead}{mark}"), NOTE),
                Span::styled(text.to_string(), NOTE),
            ])
        })
        .collect()
}

struct Inline {
    at: usize,
    finding: Finding,
    ticket: TickedFinding,
    ticked: bool,
    lines: Vec<Line<'static>>,
}

/// Side by side, right of the middle: under the new code it is nearly always about.
fn inline_lead(mode: DiffViewMode, width: u16) -> String {
    let column = match mode {
        DiffViewMode::Unified => 7,
        DiffViewMode::SideBySide => {
            1 + usize::from(ui::side_by_side_columns(width.saturating_sub(1)).1)
        }
    };
    " ".repeat(column)
}

fn inline_findings(guided: &Guided, step: &GuidedFile, lead: &str, width: u16) -> Vec<Inline> {
    let Some(commentary) = guided.commentary.get(&step.key()) else {
        return Vec::new();
    };
    placed_inline(
        guided,
        step,
        &findings::read(&commentary.text).findings,
        lead,
        width,
    )
}

/// The final review's findings that cite `step`'s file, under their lines.
fn final_inline(guided: &Guided, step: &GuidedFile, lead: &str, width: u16) -> Vec<Inline> {
    let Some(commentary) = guided.commentary.get(&guided.final_key()) else {
        return Vec::new();
    };
    let cited: Vec<Finding> = findings::read(&commentary.text)
        .cited
        .into_iter()
        .filter(|(path, _)| *path == step.path)
        .map(|(_, finding)| finding)
        .collect();
    placed_inline(guided, step, &cited, lead, width)
}

fn placed_inline(
    guided: &Guided,
    step: &GuidedFile,
    found: &[Finding],
    lead: &str,
    width: u16,
) -> Vec<Inline> {
    findings::place(step, found)
        .0
        .into_iter()
        .map(|(at, finding)| {
            let ticket = super::ticket_in(&step.path, finding);
            let ticked = guided.is_ticked(&ticket);
            Inline {
                at,
                finding: finding.clone(),
                ticket,
                ticked,
                lines: finding_lines(finding, ticked, lead, width),
            }
        })
        .collect()
}

fn finding_lines(finding: &Finding, ticked: bool, lead: &str, width: u16) -> Vec<Line<'static>> {
    let (mark, style) = match (ticked, finding.severity) {
        (true, _) => (
            "\u{2713} ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::CROSSED_OUT),
        ),
        (false, severity) => (
            severity_mark(severity),
            Style::default().fg(severity_color(severity)),
        ),
    };
    let indent = lead.chars().count() + 2;
    let available = usize::from(width).saturating_sub(indent).max(10);
    crate::panel::markdown::word_wrap(&finding.text, available)
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let head = if i == 0 {
                format!("{lead}{mark}")
            } else {
                " ".repeat(indent)
            };
            Line::from(vec![Span::styled(head, style), Span::styled(chunk, style)])
        })
        .collect()
}

fn draw_overview(state: &AppState, guided: &Guided, area: Rect, frame: &mut Frame) {
    let mut lines = vec![Line::from(Span::styled(
        "What this change is",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];
    match (guided.plan(), guided.planning()) {
        (Some(plan), _) => lines.extend(plan_lines(&plan, guided.walk())),
        (None, Some(planning)) => {
            let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
            lines.push(Line::from(Span::styled(
                format!(
                    "  {spinner} planning the walk: {}",
                    progress_text(planning, 24)
                ),
                Style::default().fg(Color::Cyan),
            )));
        }
        (None, None) => lines.extend(commentary_lines(
            state,
            guided.commentary.get(super::OVERVIEW_KEY),
            area.width,
        )),
    }
    lines.push(Line::from(""));
    for text in guided.overview.lines() {
        lines.push(Line::from(Span::styled(text.to_string(), LABEL)));
    }
    lines.extend(exchange_lines(
        state,
        guided,
        super::OVERVIEW_KEY,
        area.width,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("\u{2192}", KEY),
        Span::raw(" starts the walk at the first file"),
    ]));
    let max = wrapped_rows(&lines, area.width).saturating_sub(area.height as usize) as u16;
    guided.side_max.set(max);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((guided.side_scroll.min(max), 0)),
        area,
    );
}

/// The plan the walk modes order the files by, as the overview shows it.
fn plan_lines(plan: &Plan, walk: WalkMode) -> Vec<Line<'static>> {
    let heading = |text: &str| {
        Line::from(Span::styled(
            text.to_string(),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let mut lines = Vec::new();
    if let Some(summary) = &plan.summary {
        lines.push(Line::from(Span::styled(
            format!("  {summary}"),
            Style::default().fg(Color::White),
        )));
    }
    lines.push(Line::from(""));
    lines.push(heading("Concerns"));
    for (at, concern) in plan.concerns.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!("  {}. {}", at + 1, concern.title),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        )));
        if let Some(why) = &concern.why {
            lines.push(Line::from(Span::raw(format!("     {why}"))));
        }
        lines.push(Line::from(Span::styled(
            format!("     {}", concern.paths.join(", ")),
            LABEL,
        )));
    }
    if !plan.watch.is_empty() {
        lines.push(Line::from(""));
        lines.push(heading("Worth your attention"));
        for watch in &plan.watch {
            lines.push(Line::from(vec![
                Span::styled("  \u{2192} ", Style::default().fg(Color::Cyan)),
                Span::raw(watch.clone()),
            ]));
        }
    }
    let mut risky: Vec<(&String, &(Risk, String))> = plan
        .risk
        .iter()
        .filter(|(_, (risk, _))| *risk != Risk::Low)
        .collect();
    risky.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    if !risky.is_empty() {
        lines.push(Line::from(""));
        lines.push(heading("Risk"));
        for (path, (risk, why)) in risky {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{25cf} {} ", risk.word()),
                    Style::default().fg(risk_color(*risk)),
                ),
                Span::styled(path.clone(), Style::default().fg(Color::White)),
                Span::styled(format!(" \u{2014} {why}"), LABEL),
            ]));
        }
    }
    if !plan.skim.is_empty() {
        let mut skim: Vec<(&String, &String)> = plan.skim.iter().collect();
        skim.sort();
        lines.push(Line::from(""));
        lines.push(heading("Safe to skim (S marks them reviewed)"));
        for (path, why) in skim {
            lines.push(Line::from(Span::styled(
                format!("  {path} \u{2014} {why}"),
                LABEL,
            )));
        }
    }
    for rest in plan.rest.iter().filter(|rest| !rest.trim().is_empty()) {
        lines.push(Line::from(Span::styled(
            format!("  {}", rest.trim()),
            LABEL,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(format!("Walk by {}", walk.label().to_lowercase()), KEY),
        Span::styled(
            format!(": {} \u{b7} M or the badge switches", walk.describe()),
            LABEL,
        ),
    ]));
    lines
}

/// The final step: the whole changeset above, every file one after another
/// with the final review's findings under their lines, and the model's read
/// of it as one change below.
fn draw_final(
    state: &AppState,
    guided: &Guided,
    area: Rect,
    frame: &mut Frame,
    dividers: &mut Vec<Rect>,
) {
    let mode = state.diff_view_mode;
    let (parts, gaps) = final_areas(guided, area, mode);
    ui::draw_dividers(frame, &gaps);
    dividers.extend(gaps);
    let (diff, below) = (parts[0], parts[1]);
    let lines = final_diff_lines(guided, mode, diff.width).lines;
    let max = (lines.len() as u16).saturating_sub(diff.height);
    guided.final_max.set(max);
    let scroll = guided.final_scroll.min(max);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), diff);

    let commentary = match super::chat_session(state) {
        Some(id) => {
            let (columns, gaps) = ui::modal_columns(frame, below, &CHAT_SPLIT);
            dividers.extend(gaps);
            draw_chat(state, guided, id, columns[1], frame);
            columns[0]
        }
        None => below,
    };
    let lines = final_commentary_lines(state, guided, commentary.width).lines;
    let max = (lines.len() as u16).saturating_sub(commentary.height);
    guided.side_max.set(max);
    let scroll = guided.side_scroll.min(max);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), commentary);
}

pub(super) fn final_areas(
    guided: &Guided,
    area: Rect,
    mode: DiffViewMode,
) -> (Vec<Rect>, Vec<Rect>) {
    let rows = final_diff_lines(guided, mode, area.width).lines.len();
    split_for_diff(area, rows, diff_room(guided, area))
}

/// Every file's diff in turn, each under its title.
fn final_diff_lines(guided: &Guided, mode: DiffViewMode, width: u16) -> DiffLines {
    let mut out = DiffLines {
        lines: vec![Line::from(vec![
            Span::styled(
                "The whole change",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} file(s) \u{b7} j/k scroll", guided.total()),
                LABEL,
            ),
        ])],
        cursor: 0..0,
        findings: Vec::new(),
    };
    let lead = inline_lead(mode, width);
    for step in &guided.steps {
        out.lines.push(Line::from(""));
        out.lines.push(file_title(guided, step));
        let flagged = final_inline(guided, step, &lead, width);
        push_diff(guided, step, mode, width, &flagged, None, &mut out);
    }
    out
}

/// The model's read of the whole change: its summary, then the findings it
/// cites by file, each ticked off by a click.
fn final_commentary_lines(state: &AppState, guided: &Guided, width: u16) -> DiffLines {
    let key = guided.final_key();
    let commentary = guided.commentary.get(&key);
    let mut out = DiffLines {
        lines: Vec::new(),
        cursor: 0..0,
        findings: Vec::new(),
    };
    let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
    let progress = guided.final_progress().filter(|_| state.ai_assist);
    let working = |progress: Progress| {
        Line::from(Span::styled(
            format!("  {spinner} {}", progress_text(progress, 24)),
            Style::default().fg(Color::Cyan),
        ))
    };
    match commentary.filter(|c| c.error.is_none() && !c.text.is_empty()) {
        None => match progress {
            Some(progress) => {
                out.lines.push(Line::from(Span::styled(
                    "AI \u{b7} reading the whole change",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                out.lines.push(working(progress));
            }
            None => out.lines.extend(commentary_lines(state, commentary, width)),
        },
        Some(c) => {
            let reading = findings::read(&c.text);
            let mut header = vec![Span::styled(
                "AI \u{b7} the whole change",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            if let Some(verdict) = guided.final_verdict() {
                let (word, color) = verdict_look(verdict);
                header.push(Span::styled(
                    format!(" \u{b7} {word}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            }
            if !reading.cited.is_empty() {
                header.push(Span::styled(
                    " \u{b7} click a finding to tick it off",
                    LABEL,
                ));
            }
            out.lines.push(Line::from(header));
            let wrap = width.saturating_sub(2);
            if let Some(summary) = &reading.summary {
                out.lines.extend(
                    crate::panel::markdown::render(summary, "  ", wrap)
                        .into_iter()
                        .map(|line| line.style(Style::default().fg(Color::White))),
                );
            }
            for (path, finding) in &reading.cited {
                let ticket = super::ticket_in(path, finding);
                let shown = Finding {
                    text: format!("{path}:L{} {}", finding.line, finding.text),
                    ..finding.clone()
                };
                let first = out.lines.len();
                out.lines.extend(finding_lines(
                    &shown,
                    guided.is_ticked(&ticket),
                    "  ",
                    width,
                ));
                out.findings.push((first..out.lines.len(), ticket));
            }
            for finding in &reading.findings {
                let shown = Finding {
                    text: format!("L{} {}", finding.line, finding.text),
                    ..finding.clone()
                };
                out.lines.extend(finding_lines(&shown, false, "  ", width));
            }
            let rest = reading.rest.join("\n");
            if !rest.trim().is_empty() {
                out.lines
                    .extend(crate::panel::markdown::render(&rest, "  ", wrap));
            }
            if let Some(progress) = progress {
                out.lines.push(working(progress));
            }
        }
    }
    out.lines.extend(exchange_lines(state, guided, &key, width));
    out.lines.push(Line::from(""));
    out.lines.push(Line::from(vec![
        Span::styled("\u{2192}", KEY),
        Span::raw(if guided.source.is_pull_request() {
            " submits the review"
        } else {
            " shows every note"
        }),
        Span::styled(" \u{b7} r reads the whole change again", LABEL),
    ]));
    out
}

/// How many rows `lines` take wrapped to `width`, near enough to bound a
/// scroll so the bottom can be reached and nothing past it can.
fn wrapped_rows(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum()
}

fn draw_commentary(
    state: &AppState,
    guided: &Guided,
    step: &GuidedFile,
    area: Rect,
    frame: &mut Frame,
) {
    let mut lines = step_commentary_lines(state, guided, step, area.width);
    lines.extend(exchange_lines(state, guided, &step.key(), area.width));
    // Pinned to the bottom when a new answer is asked for, like a chat.
    let max = (lines.len() as u16).saturating_sub(area.height);
    guided.side_max.set(max);
    let scroll = guided.side_scroll.min(max);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn step_commentary_lines(
    state: &AppState,
    guided: &Guided,
    step: &GuidedFile,
    width: u16,
) -> Vec<Line<'static>> {
    let commentary = guided.commentary.get(&step.key());
    let Some(c) = commentary.filter(|c| c.error.is_none() && !c.text.is_empty()) else {
        return commentary_lines(state, commentary, width);
    };
    let reading = findings::read(&c.text);
    let (placed, unplaced) = findings::place(step, &reading.findings);
    let mut header = vec![Span::styled(
        "AI",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(verdict) = guided.verdict(step) {
        let (word, color) = verdict_look(verdict);
        header.push(Span::styled(
            format!(" \u{b7} {word}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    let ticked = placed
        .iter()
        .filter(|(_, finding)| guided.is_ticked(&guided.ticket(finding)))
        .count();
    let open = placed.len() - ticked;
    if open > 0 {
        header.push(Span::styled(
            format!(" \u{b7} {open} in the diff, \u{2193} walks them"),
            LABEL,
        ));
    }
    if ticked > 0 {
        header.push(Span::styled(
            format!(" \u{b7} {ticked} ticked off"),
            Style::default().fg(Color::Green),
        ));
    }
    let mut lines = vec![Line::from(header)];
    let wrap = width.saturating_sub(2);
    if let Some(summary) = &reading.summary {
        lines.extend(
            crate::panel::markdown::render(summary, "  ", wrap)
                .into_iter()
                .map(|line| line.style(Style::default().fg(Color::White))),
        );
    }
    for fixed in &reading.fixed {
        lines.extend(marked_lines(fixed, "\u{2713}", Color::Green, wrap));
    }
    for finding in unplaced {
        let cited = Finding {
            text: format!("L{} {}", finding.line, finding.text),
            ..finding.clone()
        };
        let ticked = guided.is_ticked(&guided.ticket(finding));
        lines.extend(finding_lines(&cited, ticked, "  ", width));
    }
    if let Some(check) = &reading.check {
        lines.extend(marked_lines(check, "\u{2192}", Color::Cyan, wrap));
    }
    let rest = reading.rest.join("\n");
    if !rest.trim().is_empty() {
        lines.extend(crate::panel::markdown::render(&rest, "  ", wrap));
    }
    if !c.done {
        let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
        let doing = if c.checking {
            " checking its questions in the code\u{2026}"
        } else {
            ""
        };
        lines.push(Line::from(Span::styled(
            format!("  {spinner}{doing}"),
            Style::default().fg(Color::Cyan),
        )));
    }
    lines
}

fn marked_lines(text: &str, mark: &str, color: Color, width: u16) -> Vec<Line<'static>> {
    let style = Style::default().fg(color);
    crate::panel::markdown::word_wrap(text, usize::from(width.saturating_sub(2)).max(10))
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let head = if i == 0 {
                format!("  {mark} ")
            } else {
                "    ".to_string()
            };
            Line::from(vec![Span::styled(head, style), Span::styled(chunk, style)])
        })
        .collect()
}

fn commentary_lines(
    state: &AppState,
    commentary: Option<&Commentary>,
    width: u16,
) -> Vec<Line<'static>> {
    let header = |text: &str, color: Color| {
        Line::from(Span::styled(
            text.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
    };
    let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
    let Some(c) = commentary else {
        return vec![
            header("AI", Color::Cyan),
            Line::from(Span::styled(
                if state.ai_assist {
                    "  waiting for a turn\u{2026}"
                } else {
                    "  AI assist is off (Settings \u{2192} models.enabled)"
                },
                LABEL,
            )),
        ];
    };
    let mut lines = vec![header("AI", Color::Cyan)];
    if let Some(error) = &c.error {
        lines.push(Line::from(Span::styled(
            format!("  {error}  (r retries)"),
            Style::default().fg(Color::Red),
        )));
        return lines;
    }
    if c.text.is_empty() {
        lines.push(Line::from(Span::styled(
            if c.thinking {
                format!("  {spinner} thinking\u{2026}")
            } else {
                format!("  {spinner} reading\u{2026}")
            },
            Style::default().fg(Color::Cyan),
        )));
        return lines;
    }
    lines.extend(crate::panel::markdown::render(
        &c.text,
        "  ",
        width.saturating_sub(2),
    ));
    if !c.done {
        lines.push(Line::from(Span::styled(
            format!("  {spinner}"),
            Style::default().fg(Color::Cyan),
        )));
    }
    lines
}

fn exchange_lines(state: &AppState, guided: &Guided, key: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for exchange in guided.exchanges.get(key).into_iter().flatten() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("you \u{203a} ", KEY),
            Span::styled(exchange.question.clone(), Style::default().fg(Color::White)),
        ]));
        let answer = commentary_lines(state, Some(&exchange.answer), width);
        lines.extend(answer.into_iter().skip(1));
    }
    lines
}

/// Every note, file by file, and for a pull request the review they become.
fn draw_summary(
    guided: &Guided,
    event: usize,
    body: &crate::panel::text_input::TextInput,
    area: Rect,
    frame: &mut Frame,
) {
    let mut lines = Vec::new();
    let is_pr = matches!(guided.source, Source::PullRequest { .. });
    if is_pr {
        let mut choices = vec![Span::styled("Submit as  ", LABEL)];
        for (i, choice) in crate::github::ReviewEvent::ALL.iter().enumerate() {
            let style = if i == event % crate::github::ReviewEvent::ALL.len() {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            choices.push(Span::styled(format!(" {} ", choice.label()), style));
            choices.push(Span::raw(" "));
        }
        lines.push(Line::from(choices));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Summary (the review's body):",
            LABEL,
        )));
        let width = area.width.max(1) as usize;
        let (cursor_row, cursor_column) = body.line_and_column();
        // Rows the summary wraps onto above the cursor, so the cursor lands
        // on the row it is really on.
        let mut row = lines.len();
        for (at, text) in body.split('\n').enumerate() {
            let cells = text.chars().count() + 2;
            if at < cursor_row {
                row += cells.div_ceil(width).max(1);
            }
            lines.push(Line::from(Span::styled(
                format!("  {text}"),
                Style::default().fg(Color::White),
            )));
        }
        let at = cursor_column + 2;
        let (y, x) = (row + at / width, at % width);
        if y < area.height as usize {
            frame.set_cursor_position(Position::new(area.x + x as u16, area.y + y as u16));
        }
        lines.push(Line::from(""));
    }
    let count = guided.progress.notes.len();
    lines.push(Line::from(Span::styled(
        if is_pr {
            format!("{count} inline comment(s):")
        } else {
            format!("{count} note(s):")
        },
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    for text in super::notes_markdown(guided).lines().skip(1) {
        let style = if text.starts_with("## ") {
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        } else if text.trim_start().starts_with('>') {
            LABEL
        } else {
            NOTE
        };
        lines.push(Line::from(Span::styled(text.to_string(), style)));
    }
    let unreviewed = guided.total() - guided.reviewed_count();
    if unreviewed > 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("{unreviewed} file(s) not reviewed yet (u jumps to the next one)"),
            Style::default().fg(Color::Yellow),
        )));
    }
    let max = wrapped_rows(&lines, area.width).saturating_sub(area.height as usize) as u16;
    guided.side_max.set(max);
    let scroll = if is_pr {
        0
    } else {
        guided.side_scroll.min(max)
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

/// What the keyboard does now, or the field being typed into.
fn draw_bottom(guided: &Guided, area: Rect, frame: &mut Frame) {
    let (label, text) = match &guided.mode {
        Mode::Note { text, editing } => (
            if editing.is_some() {
                "edit note (empty deletes)"
            } else {
                "note"
            },
            text,
        ),
        Mode::Question { text } => ("ask", text),
        Mode::Fix { text, all_notes } => (
            if *all_notes {
                "claude: address every note (extra instructions optional)"
            } else {
                "claude: change"
            },
            text,
        ),
        Mode::Chat => {
            frame.render_widget(
                Paragraph::new(ui::key_hints(&keys::footer_pairs("Guided review: chat"))),
                area,
            );
            return;
        }
        Mode::Summary { .. } => {
            let section = if guided.source.is_pull_request() {
                "Guided review: submit"
            } else {
                "Guided review: notes"
            };
            frame.render_widget(
                Paragraph::new(ui::key_hints(&keys::footer_pairs(section))),
                area,
            );
            return;
        }
        Mode::Browse => {
            let mut hints = keys::footer_pairs("Guided review");
            // Changing the code needs a checkout of it, and only a pull
            // request has a page to open.
            let editable = guided.source.editable();
            let is_pr = guided.source.is_pull_request();
            hints.retain(|(key, _)| match *key {
                "e" | "f" => editable,
                "O" => is_pr,
                _ => true,
            });
            frame.render_widget(Paragraph::new(ui::key_hints(&hints)), area);
            return;
        }
    };
    let hint = keys::footer_pairs("Guided review: writing")
        .iter()
        .map(|(key, label)| format!("{key} {label}"))
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{label} \u{203a} "), KEY),
        Span::styled(hint, LABEL),
    ])];
    for part in text.split('\n') {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::raw(part.to_string()),
        ]));
    }
    // The cursor goes where the next character will, which is no longer
    // always the end: the arrows move it.
    let (row, column) = text.line_and_column();
    let skip = lines.len().saturating_sub(area.height as usize);
    let cursor_line = 1 + row;
    if cursor_line >= skip {
        let y = area.y + (cursor_line - skip) as u16;
        let x = area.x + 2 + column as u16;
        if y < area.y + area.height && x < area.x + area.width {
            frame.set_cursor_position(Position::new(x, y));
        }
    }
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()),
        area,
    );
}

/// The end of `text` that fits in `width` columns, marked as cut when it is.
fn truncate_left(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let tail: String = text.chars().skip(count - keep).collect();
    format!("\u{2026}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_path_keeps_its_file_name() {
        let cut = truncate_left("src/panel/guided/draw.rs", 12);

        assert!(cut.ends_with("draw.rs"));
        assert_eq!(cut.chars().count(), 12);
    }

    #[test]
    fn a_short_path_is_left_alone() {
        assert_eq!(truncate_left("a.rs", 12), "a.rs");
    }
}
