//! Drawing a guided review: the steps down the left, the file on screen with
//! its notes, and the model's read of it beneath; on the final step, the whole
//! changeset with the model's read of it as one change.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::git::guided::GuidedFile;
use crate::state::{AppState, SPINNER_FRAMES};
use crate::ui;

use super::chat::chat_session;
use super::findings::Severity;
use super::found::ticket_in;
use super::notes::notes_markdown;
use super::{Guided, Mode, OVERVIEW, OVERVIEW_KEY, Progress, Verdict, triage_rank};
use bottom::{draw_bottom, draw_summary};
use list::{draw_reset_button, draw_steps, scope_badge, walk_badge};
use panes::draw_step;

mod bottom;
mod commentary;
mod diff;
mod final_review;
mod list;
mod overview;
mod panes;

pub(super) use list::{
    RowTarget, list_width, reset_button_area, row_targets, scope_badge_area, steps_area,
    walk_badge_area,
};
#[cfg(test)]
pub(super) use panes::step_areas;
pub(super) use panes::{chat_screen_area, diff_area, finding_at, selectable_areas};

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

/// A long read's progress as words, with a bar `width` wide where there is a
/// measure: how long reading the prompt has taken against how long it
/// should, then how many files the plan has placed.
fn progress_text(progress: Progress, width: usize) -> String {
    match progress {
        Progress::Queued { ahead: 0 } => "starting".to_string(),
        Progress::Queued { ahead } => format!("waiting for {ahead} file(s) to be read"),
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
    let (settled, total) = guided.files_settled();
    if state.ai_assist && guided.plan().is_some() && settled < total {
        spans.push(Span::styled(
            format!("  {spinner} reading {settled}/{total} file(s)"),
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

/// How many rows `lines` take wrapped to `width`, near enough to bound a
/// scroll so the bottom can be reached and nothing past it can.
fn wrapped_rows(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum()
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
