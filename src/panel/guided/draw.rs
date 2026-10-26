//! Drawing a guided review: the steps down the left, the hunk on screen with
//! its notes, and the model's read of it beneath.

use ratatui::{
    Frame,
    layout::{Constraint, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::findings::{self, Finding, Severity};
use super::layout::{self, Row};
use super::{Commentary, Guided, Mode, OVERVIEW, Source, Verdict};
use crate::git::guided::{GuidedHunk, HunkLine, LineKind, Side, TickedFinding};
use crate::panel::keys;
use crate::state::{AppState, DiffViewMode, SPINNER_FRAMES};
use crate::ui::{self, DiffContentKind};

const LABEL: Style = Style::new().fg(Color::DarkGray);
const KEY: Style = Style::new().fg(Color::Yellow);
const NOTE: Style = Style::new().fg(Color::LightMagenta);
const CURSOR_BG: Color = Color::Rgb(40, 46, 60);
const ISSUE: Color = Color::LightRed;
const NIT: Color = Color::Yellow;

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
    let list_width = list_width(inner.width, requested_list_width);
    let (panes, _) = ui::modal_column_areas(
        rows[1],
        &[Constraint::Length(list_width), Constraint::Min(30)],
    );
    Some((panes[0], panes[1]))
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

/// The step each row of the step list stands for, from its first row: the
/// overview, then each step under the name of its file, which has a row of
/// its own and stands for none. With the first row shown, as [`draw_steps`]
/// scrolls the list to keep the current step in the middle.
pub(super) fn step_rows(guided: &Guided, height: u16) -> (Vec<Option<usize>>, usize) {
    let mut rows = vec![Some(OVERVIEW)];
    let mut selected = 0usize;
    let mut last_file = None;
    for (index, step) in guided.steps.iter().enumerate() {
        if last_file != Some(step.file_index) {
            last_file = Some(step.file_index);
            rows.push(None);
        }
        if guided.current == index + 1 {
            selected = rows.len();
        }
        rows.push(Some(index + 1));
    }
    let offset = selected.saturating_sub(height as usize / 2);
    (rows, offset)
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
        Span::styled(
            if guided.current == OVERVIEW {
                format!("overview \u{b7} {total} steps ")
            } else {
                format!("step {}/{total} ", guided.current)
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

/// How many steps the model called an issue, a nit and ok, worst first, each
/// in its colour, so how iffy the change is shows before walking it.
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

/// What the walk covers, in a colour of its own, so a review of the whole
/// branch is never mistaken for one of only what is not committed yet.
fn scope_badge(source: &Source) -> Span<'static> {
    let (text, color) = match source {
        Source::Branch { base_ref, .. } => (format!(" WHOLE BRANCH vs {base_ref} "), Color::Blue),
        Source::Changes { .. } => (" UNCOMMITTED ONLY ".to_string(), Color::Magenta),
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

/// The steps, one row each, grouped under their file.
fn draw_steps(guided: &Guided, area: Rect, frame: &mut Frame) {
    let width = area.width as usize;
    let mut items = Vec::with_capacity(guided.steps.len() + 1);
    let mut selected = 0usize;
    let overview_style = if guided.current == OVERVIEW {
        Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    items.push(ListItem::new(Line::from(Span::styled(
        "\u{2605} Overview",
        overview_style.fg(Color::LightCyan),
    ))));
    let mut last_file = None;
    for (index, step) in guided.steps.iter().enumerate() {
        if last_file != Some(step.file_index) {
            last_file = Some(step.file_index);
            items.push(ListItem::new(file_row(guided, &step.path, width)));
        }
        let number = index + 1;
        let here = guided.current == number;
        if here {
            selected = items.len();
        }
        let mark = if guided.is_reviewed(step) {
            Span::styled("\u{2713} ", Style::default().fg(Color::Green))
        } else if here {
            Span::styled("\u{25b6} ", Style::default().fg(Color::LightCyan))
        } else {
            Span::styled("\u{b7} ", LABEL)
        };
        // In the first column, where a narrow list cannot cut it off.
        let verdict = match guided.commentary.get(&step.key()) {
            Some(c) if c.error.is_some() => Span::styled("\u{26a0} ", LABEL),
            Some(c) => match guided.verdict(step) {
                Some(verdict) => {
                    Span::styled("\u{25cf} ", Style::default().fg(verdict_look(verdict).1))
                }
                None if !c.done => Span::styled("\u{2026} ", LABEL),
                None => Span::raw("  "),
            },
            None => Span::raw("  "),
        };
        let notes = guided.notes_for(step).len();
        // What is left to look at stands out; what is done steps back.
        let base = match (here, guided.is_reviewed(step)) {
            (true, _) => Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD),
            (false, true) => LABEL,
            (false, false) => Style::default().fg(Color::White),
        };
        let mut spans = vec![
            verdict,
            mark,
            Span::styled(
                format!(
                    "{} {}",
                    change_size(step),
                    step.header
                        .rsplit("@@")
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("")
                ),
                base,
            ),
        ];
        if notes > 0 {
            spans.push(Span::styled(format!(" \u{270e}{notes}"), NOTE));
        }
        items.push(ListItem::new(Line::from(spans)).style(base));
    }
    let offset = selected.saturating_sub(area.height as usize / 2);
    let list = List::new(items.into_iter().skip(offset).collect::<Vec<_>>());
    frame.render_widget(list, area);
}

/// A file's row in the step list: its path, and what is still open in it,
/// or a tick once every hunk of it is read and nothing is left open.
fn file_row(guided: &Guided, path: &str, width: usize) -> Line<'static> {
    let (issues, nits) = guided.open_in_file(path);
    let read = guided
        .steps
        .iter()
        .filter(|step| step.path == path)
        .all(|step| guided.verdict(step).is_some());
    let mut tally: Vec<Span<'static>> = Vec::new();
    if issues > 0 {
        tally.push(Span::styled(
            format!(" \u{25cf}{issues}"),
            Style::default().fg(ISSUE),
        ));
    }
    if nits > 0 {
        tally.push(Span::styled(
            format!(" \u{25cb}{nits}"),
            Style::default().fg(NIT),
        ));
    }
    if tally.is_empty() && read {
        tally.push(Span::styled(" \u{2713}", Style::default().fg(Color::Green)));
    }
    let used: usize = tally.iter().map(Span::width).sum();
    let mut spans = vec![Span::styled(
        truncate_left(path, width.saturating_sub(used)),
        Style::default().fg(Color::Gray),
    )];
    spans.extend(tally);
    Line::from(spans)
}

/// What a step changes, as the step list shows it: the lines it adds and
/// removes, or what happened to the file when there is no text diff.
fn change_size(step: &GuidedHunk) -> String {
    if step.lines.is_empty() {
        step.file_note
            .clone()
            .unwrap_or_else(|| "no text diff".into())
    } else {
        format!("+{} -{}", step.added(), step.removed())
    }
}

/// How a verdict reads and the colour it is shown in, wherever it is shown.
fn verdict_look(verdict: Verdict) -> (&'static str, Color) {
    match verdict {
        Verdict::Ok => ("ok", Color::Green),
        Verdict::Nit => ("nit", NIT),
        Verdict::Issue => ("issue", Color::Red),
    }
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Nit => NIT,
        Severity::Issue => ISSUE,
    }
}

fn hunk_label(step: &GuidedHunk) -> String {
    let (added, removed) = (step.added(), step.removed());
    if step.lines.is_empty() {
        step.file_note
            .clone()
            .unwrap_or_else(|| "no text diff".into())
    } else {
        format!(
            "{}/{} +{added} -{removed}",
            step.hunk_index + 1,
            step.hunks_in_file
        )
    }
}

/// The step on screen: its hunk above, the model's read of it below.
fn draw_step(
    state: &AppState,
    guided: &Guided,
    area: Rect,
    frame: &mut Frame,
    dividers: &mut Vec<Rect>,
) {
    let Some(step) = guided.step() else {
        draw_overview(state, guided, area, frame);
        return;
    };
    let mode = state.diff_view_mode;
    let (parts, gaps) = step_areas(guided, step, area, mode);
    ui::draw_dividers(frame, &gaps);
    dividers.extend(gaps);
    draw_hunk(guided, step, mode, parts[0], frame);
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

/// The regions of the pane beside the step list whose text the mouse can
/// select: the hunk and the commentary under it, or the overview whole. The
/// chat is a program's screen, and takes the mouse for itself.
pub(super) fn selectable_areas(guided: &Guided, side: Rect, mode: DiffViewMode) -> Vec<Rect> {
    let Some(step) = guided.step() else {
        return vec![side];
    };
    let (parts, _) = step_areas(guided, step, side, mode);
    let commentary = if guided.chat.is_some() {
        ui::modal_column_areas(parts[1], &CHAT_SPLIT).0[0]
    } else {
        parts[1]
    };
    vec![parts[0], commentary]
}

/// The commentary on the left under the hunk, the chat on the right.
const CHAT_SPLIT: [Constraint; 2] = [Constraint::Percentage(40), Constraint::Percentage(60)];

/// Where the chat's program draws, beside the commentary under the hunk and
/// below its title row; `None` when there is no chat or no hunk on screen.
pub(super) fn chat_screen_area(guided: &Guided, side: Rect, mode: DiffViewMode) -> Option<Rect> {
    guided.chat?;
    let step = guided.step()?;
    let (parts, _) = step_areas(guided, step, side, mode);
    let (columns, _) = ui::modal_column_areas(parts[1], &CHAT_SPLIT);
    let chat = columns[1];
    Some(Rect {
        y: chat.y + 1,
        height: chat.height.saturating_sub(1),
        ..chat
    })
}

/// The chat: a title row saying which file it was last told about and how to
/// reach it, and the program's screen under it.
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

/// The hunk and the commentary under it, as [`draw_step`] splits `area`,
/// with the divider between them. The findings sit in the hunk, so it gets
/// most of the room and the commentary what is left.
pub(super) fn step_areas(
    guided: &Guided,
    step: &GuidedHunk,
    area: Rect,
    mode: DiffViewMode,
) -> (Vec<Rect>, Vec<Rect>) {
    let rows = hunk_lines(guided, step, mode, area.width).lines.len();
    // A chat under the hunk wants room to be read and typed in.
    let most = if guided.chat.is_some() {
        area.height / 2
    } else {
        area.height * 2 / 3
    };
    let diff_height = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .min(most)
        .max(area.height.min(6));
    ui::modal_row_areas(area, &[Constraint::Length(diff_height), Constraint::Min(3)])
}

fn draw_hunk(
    guided: &Guided,
    step: &GuidedHunk,
    mode: DiffViewMode,
    area: Rect,
    frame: &mut Frame,
) {
    let hunk = hunk_lines(guided, step, mode, area.width);
    let scroll = hunk.scroll(area.height);
    frame.render_widget(Paragraph::new(hunk.lines).scroll((scroll as u16, 0)), area);
}

/// The finding drawn at `column`, `row` of the pane beside the step list,
/// named the way a tick names it; `None` off the hunk or off a finding.
pub(super) fn finding_at(
    guided: &Guided,
    side: Rect,
    mode: DiffViewMode,
    column: u16,
    row: u16,
) -> Option<TickedFinding> {
    let step = guided.step()?;
    let (parts, _) = step_areas(guided, step, side, mode);
    let area = parts[0];
    if !crate::panel::pointer::inside(area, column, row) {
        return None;
    }
    let hunk = hunk_lines(guided, step, mode, area.width);
    let at = hunk.scroll(area.height) + usize::from(row - area.y);
    hunk.findings
        .into_iter()
        .find(|(rows, _)| rows.contains(&at))
        .map(|(_, finding)| finding)
}

/// A hunk drawn for the pane: every row, which of them the cursor's line and
/// what is pinned under it take up, and which each finding takes up.
struct HunkLines {
    lines: Vec<Line<'static>>,
    cursor: std::ops::Range<usize>,
    findings: Vec<(std::ops::Range<usize>, TickedFinding)>,
}

impl HunkLines {
    /// How far down a pane `height` rows tall starts: the cursor line and
    /// what is pinned under it stay in view, with a little of what follows,
    /// unless that would push the line itself off the top.
    fn scroll(&self, height: u16) -> usize {
        (self.cursor.end + 2)
            .saturating_sub(usize::from(height))
            .min(self.cursor.start)
    }
}

/// The hunk `width` wide, laid out as `mode` says, with each line's notes and
/// findings under the row it is on.
fn hunk_lines(guided: &Guided, step: &GuidedHunk, mode: DiffViewMode, width: u16) -> HunkLines {
    let mut lines = vec![
        hunk_title(guided, step),
        Line::from(Span::styled(step.header.clone(), LABEL)),
    ];
    let notes = guided.notes_for(step);
    let lead = inline_lead(mode, width);
    let flagged = inline_findings(guided, step, &lead, width);
    let mut cursor = 0..0;
    let mut ticks = Vec::new();
    for row in layout::rows(step, mode) {
        let start = lines.len();
        // A bar in the worst open flag's colour marks a row the model flagged.
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
            Row::Whole(at) => vec![unified_line(step, at, at == guided.cursor)],
            Row::Pair(left, right) => {
                paired_lines(step, left, right, guided.cursor, width.saturating_sub(1))
            }
        };
        lines.extend(body.into_iter().enumerate().map(|(i, line)| {
            let lead = if i == 0 { bar.clone() } else { Span::raw(" ") };
            Line::from(std::iter::once(lead).chain(line.spans).collect::<Vec<_>>())
                .style(line.style)
        }));
        for at in row.lines() {
            let here = at == guided.cursor;
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
                ticks.push((first..lines.len(), guided.ticket(&inline.finding)));
            }
        }
        if row.lines().any(|at| at == guided.cursor) {
            cursor = start..lines.len();
        }
    }
    // Notes pinned to lines this hunk no longer shows, said once at the end.
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
    HunkLines {
        lines,
        cursor,
        findings: ticks,
    }
}

/// The file, where the hunk sits in it, and what has been said about it.
fn hunk_title(guided: &Guided, step: &GuidedHunk) -> Line<'static> {
    let mut title = vec![Span::styled(
        step.path.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    title.push(Span::styled(format!("  {}", hunk_label(step)), LABEL));
    if let Some(note) = &step.file_note {
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

/// Line `at` across the whole row, both its numbers in the gutter.
fn unified_line(step: &GuidedHunk, at: usize, here: bool) -> Line<'static> {
    let line = &step.lines[at];
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

/// The old file's line beside the new file's, the cursor's side lit up.
fn paired_lines(
    step: &GuidedHunk,
    left: Option<usize>,
    right: Option<usize>,
    cursor: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let cell = |at: Option<usize>, number: fn(&HunkLine) -> Option<usize>| {
        let line = &step.lines[at?];
        let kind = match line.kind {
            LineKind::Context => DiffContentKind::Context,
            LineKind::Added => DiffContentKind::Added,
            LineKind::Removed => DiffContentKind::Removed,
            LineKind::Meta => return None,
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
    let lit = match (left == Some(cursor), right == Some(cursor)) {
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

/// `line` with the cursor's background on the spans that start in `columns`.
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

/// A note under the line it is pinned to.
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

/// A finding drawn under the hunk line it is about.
struct Inline {
    /// The line it is about, by its index in the hunk.
    at: usize,
    finding: Finding,
    ticked: bool,
    lines: Vec<Line<'static>>,
}

/// Where the text pinned under a line starts, in a hunk `width` wide: past
/// the line numbers, or side by side just right of the middle, under the new
/// code it is nearly always about.
fn inline_lead(mode: DiffViewMode, width: u16) -> String {
    let column = match mode {
        DiffViewMode::Unified => 7,
        // The bar's column, then the right side's start in what is left.
        DiffViewMode::SideBySide => {
            1 + usize::from(ui::side_by_side_columns(width.saturating_sub(1)).1)
        }
    };
    " ".repeat(column)
}

/// The findings drawn under each flagged line of the hunk, behind `lead` and
/// wrapped to `width`.
fn inline_findings(guided: &Guided, step: &GuidedHunk, lead: &str, width: u16) -> Vec<Inline> {
    let Some(commentary) = guided.commentary.get(&step.key()) else {
        return Vec::new();
    };
    let reading = findings::read(&commentary.text);
    findings::place(step, &reading.findings)
        .0
        .into_iter()
        .map(|(at, finding)| {
            let ticked = guided.is_ticked(&guided.ticket(finding));
            Inline {
                at,
                finding: finding.clone(),
                ticked,
                lines: finding_lines(finding, ticked, lead, width),
            }
        })
        .collect()
}

/// One finding, marked by how much it matters, or as ticked off, and wrapped
/// under its mark.
fn finding_lines(finding: &Finding, ticked: bool, lead: &str, width: u16) -> Vec<Line<'static>> {
    let (mark, style) = match (ticked, finding.severity) {
        (true, _) => (
            "\u{2713} ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::CROSSED_OUT),
        ),
        (false, Severity::Issue) => ("\u{25cf} ", Style::default().fg(ISSUE)),
        (false, Severity::Nit) => ("\u{25cb} ", Style::default().fg(NIT)),
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
    lines.extend(commentary_lines(
        state,
        guided.commentary.get(super::OVERVIEW_KEY),
        area.width,
    ));
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
        Span::raw(" starts the walk at the first hunk"),
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
    step: &GuidedHunk,
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

/// The model's read of a step less the findings drawn in the hunk: the
/// verdict, what the hunk does, findings on lines it does not show, and
/// anything else that was said.
fn step_commentary_lines(
    state: &AppState,
    guided: &Guided,
    step: &GuidedHunk,
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
            format!(" \u{b7} {open} in the hunk, \u{2193} walks them"),
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
        lines.push(Line::from(Span::styled(
            format!("  {spinner}"),
            Style::default().fg(Color::Cyan),
        )));
    }
    lines
}

/// `text` behind `mark`, wrapped to `width` under itself, all in `color`.
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
                format!("  {spinner} reading this hunk\u{2026}")
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
            format!("{unreviewed} step(s) not reviewed yet (u jumps to the next one)"),
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
