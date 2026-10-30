//! The line under the walk, and the summary of every note.

use super::*;

/// Every note, file by file, and for a pull request the review they become.
pub(super) fn draw_summary(
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
pub(super) fn draw_bottom(guided: &Guided, area: Rect, frame: &mut Frame) {
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
            if *all_notes && guided.on_final() {
                "claude: address every open finding and note (extra instructions optional)"
            } else if *all_notes {
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
