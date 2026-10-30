//! The model's read of a file, and the questions asked about it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::git::guided::GuidedFile;
use crate::panel::guided::findings::Finding;
use crate::panel::guided::{Commentary, Guided, findings};
use crate::state::{AppState, SPINNER_FRAMES};

use super::diff::finding_lines;
use super::{KEY, LABEL, verdict_look};

pub(super) fn draw_commentary(
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

pub(super) fn commentary_lines(
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

pub(super) fn exchange_lines(
    state: &AppState,
    guided: &Guided,
    key: &str,
    width: u16,
) -> Vec<Line<'static>> {
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
