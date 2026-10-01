//! Drawing a guided review: the steps down the left, the hunk on screen with
//! its notes, and the model's read of it beneath.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::{Commentary, Guided, Mode, OVERVIEW, Source, Verdict};
use crate::git::guided::{GuidedHunk, Side};
use crate::state::{AppState, SPINNER_FRAMES};
use crate::ui;

const LABEL: Style = Style::new().fg(Color::DarkGray);
const KEY: Style = Style::new().fg(Color::Yellow);
const NOTE: Style = Style::new().fg(Color::LightMagenta);
const CURSOR_BG: Color = Color::Rgb(40, 46, 60);

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
    let list_width = (inner.width / 4).clamp(24, 44);
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
            items.push(ListItem::new(Line::from(Span::styled(
                truncate_left(&step.path, width),
                Style::default().fg(Color::Gray),
            ))));
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
        let verdict = match guided.commentary.get(&step.key()) {
            Some(c) if c.error.is_some() => Span::styled(" \u{26a0}", LABEL),
            Some(c) => match c.verdict() {
                Some(Verdict::Issue) => Span::styled(" \u{25cf}", Style::default().fg(Color::Red)),
                Some(Verdict::Nit) => Span::styled(" \u{25cf}", Style::default().fg(Color::Yellow)),
                Some(Verdict::Ok) => Span::styled(" \u{25cf}", Style::default().fg(Color::Green)),
                None if !c.done => Span::styled(" \u{2026}", LABEL),
                None => Span::raw(""),
            },
            None => Span::raw(""),
        };
        let notes = guided.notes_for(step).len();
        let base = if here {
            Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::raw("  "),
            mark,
            Span::styled(
                format!(
                    "{} {}",
                    hunk_label(step),
                    step.header
                        .rsplit("@@")
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("")
                ),
                base,
            ),
            verdict,
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
    let diff_height = (step.lines.len() as u16 + 2 + notes_height(guided, step))
        .min(area.height * 3 / 5)
        .max(area.height.min(6));
    let (parts, gaps) = ui::modal_rows(
        frame,
        area,
        &[Constraint::Length(diff_height), Constraint::Min(3)],
    );
    dividers.extend(gaps);
    draw_hunk(guided, step, parts[0], frame);
    draw_commentary(state, guided, &step.key(), parts[1], frame);
}

fn notes_height(guided: &Guided, step: &GuidedHunk) -> u16 {
    guided
        .notes_for(step)
        .iter()
        .map(|(_, note)| note.body.lines().count().max(1) as u16)
        .sum()
}

fn draw_hunk(guided: &Guided, step: &GuidedHunk, area: Rect, frame: &mut Frame) {
    let mut lines = Vec::new();
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
    if guided.is_reviewed(step) {
        title.push(Span::styled(
            "  reviewed",
            Style::default().fg(Color::Green),
        ));
    }
    lines.push(Line::from(title));
    lines.push(Line::from(Span::styled(step.header.clone(), LABEL)));
    let notes = guided.notes_for(step);
    let mut cursor_row = 0usize;
    for (index, line) in step.lines.iter().enumerate() {
        let gutter = format!(
            "{:>5} {:>5} ",
            line.old.map(|n| n.to_string()).unwrap_or_default(),
            line.new.map(|n| n.to_string()).unwrap_or_default()
        );
        let mut spans = vec![Span::styled(gutter, LABEL)];
        spans.extend(
            ui::highlight_diff_line_for_path(&line.text, &step.path)
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style)),
        );
        let mut rendered = Line::from(spans);
        if index == guided.cursor {
            cursor_row = lines.len();
            rendered = rendered.style(Style::default().bg(CURSOR_BG));
        }
        lines.push(rendered);
        if let Some(anchor) = line.anchor() {
            for (_, note) in notes
                .iter()
                .filter(|(_, note)| (note.line, note.side) == anchor)
            {
                for (i, text) in note.body.lines().enumerate() {
                    let lead = if i == 0 {
                        "      \u{270e} "
                    } else {
                        "        "
                    };
                    lines.push(Line::from(vec![
                        Span::styled(lead, NOTE),
                        Span::styled(text.to_string(), NOTE),
                    ]));
                }
            }
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
    let height = area.height as usize;
    let scroll = cursor_row.saturating_sub(height.saturating_sub(3));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), area);
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

fn draw_commentary(state: &AppState, guided: &Guided, key: &str, area: Rect, frame: &mut Frame) {
    let mut lines = commentary_lines(state, guided.commentary.get(key), area.width);
    lines.extend(exchange_lines(state, guided, key, area.width));
    // Pinned to the bottom when a new answer is asked for, like a chat.
    let max = (lines.len() as u16).saturating_sub(area.height);
    guided.side_max.set(max);
    let scroll = guided.side_scroll.min(max);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
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
fn draw_summary(guided: &Guided, event: usize, body: &str, area: Rect, frame: &mut Frame) {
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
        for text in body.split('\n') {
            lines.push(Line::from(Span::styled(
                format!("  {text}"),
                Style::default().fg(Color::White),
            )));
        }
        if let Some(last) = lines.last_mut() {
            last.spans
                .push(Span::styled("\u{2588}", Style::default().fg(Color::Gray)));
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
        Mode::Summary { .. } => {
            let hints: &[(&str, &str)] = if guided.source.is_pull_request() {
                &[
                    ("Tab", "verdict"),
                    ("type", "summary"),
                    ("Enter", "submit"),
                    ("Ctrl-y", "copy"),
                    ("Esc", "back"),
                ]
            } else {
                &[
                    ("y", "copy notes"),
                    ("f", "claude fixes all"),
                    ("j/k", "scroll"),
                    ("Esc", "back"),
                ]
            };
            frame.render_widget(Paragraph::new(ui::key_hints(hints)), area);
            return;
        }
        Mode::Browse => {
            let mut hints = vec![
                ("\u{2192}/\u{2190}", "step"),
                ("[/]", "file"),
                ("j/k", "line"),
                ("c", "note"),
                ("a", "ask"),
            ];
            if guided.source.editable() {
                hints.extend([("e", "edit"), ("f", "claude fix")]);
            }
            hints.extend([
                ("m", "mark"),
                ("u", "unreviewed"),
                (
                    "s",
                    if guided.source.is_pull_request() {
                        "submit"
                    } else {
                        "notes"
                    },
                ),
                ("R", "reload"),
                ("Esc", "leave"),
            ]);
            frame.render_widget(Paragraph::new(ui::key_hints(&hints)), area);
            return;
        }
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{label} \u{203a} "), KEY),
        Span::styled(
            "Enter sends \u{b7} Alt-Enter new line \u{b7} Esc cancels",
            LABEL,
        ),
    ])];
    for (i, part) in text.split('\n').enumerate() {
        let mut spans = vec![Span::raw("  "), Span::raw(part.to_string())];
        if i == text.split('\n').count() - 1 {
            spans.push(Span::styled("\u{2588}", Style::default().fg(Color::Gray)));
        }
        lines.push(Line::from(spans));
    }
    let skip = lines.len().saturating_sub(area.height as usize);
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
