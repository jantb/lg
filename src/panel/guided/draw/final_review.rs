//! The final step: the whole changeset with the final review's findings under
//! their lines, and its read of the change below.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::panel::guided::findings::Finding;
use crate::panel::guided::{Guided, Progress, findings};
use crate::state::{AppState, DiffViewMode, SPINNER_FRAMES};
use crate::ui;

use super::commentary::commentary_lines;
use super::diff::{DiffLines, file_title, final_inline, finding_lines, inline_lead, push_diff};
use super::panes::{diff_room, draw_chat, split_chat, split_for_diff};
use super::{CURSOR_BG, KEY, LABEL, progress_text, verdict_look};

/// The final step: the whole changeset above, every file one after another
/// with the final review's findings under their lines, and the model's read
/// of it as one change below.
pub(super) fn draw_final(
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
    let DiffLines { lines, cursor, .. } = final_diff_lines(guided, mode, diff.width);
    let max = (lines.len() as u16).saturating_sub(diff.height);
    guided.final_max.set(max);
    if guided.final_reveal.replace(false) && !cursor.is_empty() {
        guided.final_scroll.set(reveal(
            guided.final_scroll.get().min(max),
            &cursor,
            diff.height,
        ));
    }
    let scroll = guided.final_scroll.get().min(max);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), diff);

    let commentary = match super::chat_session(state) {
        Some(id) => {
            let (columns, gaps) = split_chat(frame, guided, below);
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

/// `scroll` moved as little as brings `rows` into a pane `height` high, with
/// the code line a finding sits under shown above it.
fn reveal(scroll: u16, rows: &std::ops::Range<usize>, height: u16) -> u16 {
    let first = u16::try_from(rows.start.saturating_sub(2)).unwrap_or(u16::MAX);
    let end = u16::try_from(rows.end).unwrap_or(u16::MAX);
    if first < scroll {
        first
    } else if end > scroll.saturating_add(height) {
        end.saturating_sub(height).min(first)
    } else {
        scroll
    }
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
pub(super) fn final_diff_lines(guided: &Guided, mode: DiffViewMode, width: u16) -> DiffLines {
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
    mark_picked(guided, &mut out);
    out
}

/// The picked finding's rows, lit as the cursor line is and made `lines.cursor`.
fn mark_picked(guided: &Guided, lines: &mut DiffLines) {
    let Some((path, finding)) = guided.picked() else {
        return;
    };
    let picked = super::ticket_in(&path, &finding);
    let Some(rows) = lines
        .findings
        .iter()
        .find(|(_, ticket)| *ticket == picked)
        .map(|(rows, _)| rows.clone())
    else {
        return;
    };
    for line in &mut lines.lines[rows.clone()] {
        *line =
            std::mem::take(line).style(Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD));
    }
    lines.cursor = rows;
}

/// The model's read of the whole change: its summary, then the findings it
/// cites by file, each ticked off by a click.
pub(super) fn final_commentary_lines(state: &AppState, guided: &Guided, width: u16) -> DiffLines {
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
                    " \u{b7} \u{2193}/\u{2191} pick a finding \u{b7} Enter go to it \u{b7} t or click ticks it off",
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
    mark_picked(guided, &mut out);
    out.lines.push(Line::from(""));
    out.lines.push(Line::from(vec![
        Span::styled("\u{2192}", KEY),
        Span::raw(if guided.source.is_pull_request() {
            " submits the review"
        } else {
            " shows every note"
        }),
        Span::styled(
            " \u{b7} f fixes the picked finding, F every one open \u{b7} C chats about the whole change \u{b7} r reads it again",
            LABEL,
        ),
    ]));
    out
}
