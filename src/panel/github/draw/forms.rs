//! The merge and new pull request forms.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::panel::github::lookup::selected_pr;
use crate::panel::github::{CreateField, CreateForm, MergeForm, MergeRow};
use crate::panel::text_input::TextInput;
use crate::state::AppState;
use crate::ui::palette;

use super::LABEL;

fn toggle(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

pub(super) fn draw_merge(state: &AppState, form: &MergeForm, area: Rect, frame: &mut Frame) {
    let title = selected_pr(state)
        .map(|pr| pr.title.clone())
        .unwrap_or_default();
    let row_style = |row: MergeRow| {
        if form.row == row {
            palette::selection()
        } else {
            Style::default()
        }
    };
    let methods = form
        .methods
        .iter()
        .map(|method| method.label())
        .collect::<Vec<_>>()
        .join(", ");
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("Merge #{} ", form.number),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(title, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Method         ", LABEL),
            Span::styled(
                format!("\u{2039} {} \u{203a}", form.method.label()),
                row_style(MergeRow::Method).fg(Color::Cyan),
            ),
        ]),
        Line::from(Span::styled(
            format!("               allowed here: {methods}"),
            LABEL,
        )),
        Line::from(vec![
            Span::styled("Delete branch  ", LABEL),
            Span::styled(
                format!(
                    "{} local and remote, once merged",
                    toggle(form.delete_branch)
                ),
                row_style(MergeRow::DeleteBranch),
            ),
        ]),
        Line::from(vec![
            Span::styled("Auto-merge     ", LABEL),
            Span::styled(
                format!("{} wait for required checks and reviews", toggle(form.auto)),
                row_style(MergeRow::Auto),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        },
    );
}

pub(super) fn draw_create(form: &CreateForm, area: Rect, frame: &mut Frame) {
    let label = |field: CreateField, text: &'static str| {
        if form.field == field {
            Span::styled(
                text,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(text, LABEL)
        }
    };
    let value = |field: CreateField| {
        if form.field == field {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        }
    };
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    let value_width = inner.width.saturating_sub(8) as usize;
    let title = window(&form.title, value_width);
    let base = window(&form.base, value_width);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("From    ", LABEL),
            Span::styled(form.branch.clone(), Style::default().fg(Color::Yellow)),
            Span::styled("  pushed to the remote first if it has to be", LABEL),
        ]),
        Line::from(vec![
            label(CreateField::Title, "Title   "),
            Span::styled(title.0.clone(), value(CreateField::Title)),
        ]),
        Line::from(vec![
            label(CreateField::Base, "Base    "),
            Span::styled(base.0.clone(), value(CreateField::Base)),
        ]),
        Line::from(vec![
            label(CreateField::Draft, "Draft   "),
            Span::styled(
                format!("{} open as a draft", toggle(form.draft)),
                value(CreateField::Draft),
            ),
        ]),
        Line::from(label(CreateField::Body, "Body")),
    ];
    let body_top = lines.len() as u16;
    let body_width = inner.width.saturating_sub(2).max(1) as usize;
    let body_rows = wrap_body(&form.body, body_width);
    let room = (inner.height.saturating_sub(body_top) as usize).max(1);
    // Where the cursor is among the wrapped rows: the rows every line above
    // it takes, then the row of its own line it falls on.
    let (line, column) = form.body.line_and_column();
    let cursor_row = form
        .body
        .split('\n')
        .take(line)
        .map(|text| text.chars().count().div_ceil(body_width).max(1))
        .sum::<usize>()
        + column / body_width;
    // Keep the cursor's row in sight: that is where the typing happens.
    let skip = cursor_row.saturating_sub(room - 1);
    for row in body_rows.iter().skip(skip) {
        lines.push(Line::from(Span::styled(
            format!("  {row}"),
            value(CreateField::Body),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    let cursor = match form.field {
        CreateField::Title => Some((8 + title.1, 1)),
        CreateField::Base => Some((8 + base.1, 2)),
        CreateField::Body => Some((
            2 + column % body_width,
            body_top as usize + cursor_row - skip,
        )),
        CreateField::Draft => None,
    };
    if let Some((x, y)) = cursor
        && (y as u16) < inner.height
    {
        frame.set_cursor_position(Position::new(inner.x + x as u16, inner.y + y as u16));
    }
}

/// The body cut to rows of `width`, breaking only where it has to: the text
/// is shown as written, line breaks included.
fn wrap_body(body: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in body.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
            continue;
        }
        for chunk in chars.chunks(width) {
            rows.push(chunk.iter().collect());
        }
    }
    rows
}

/// The part of a value that fits in `width` columns, with the column the
/// cursor is drawn at. A value too long to show whole is shown around the
/// cursor, the cut marked.
fn window(input: &TextInput, width: usize) -> (String, usize) {
    let chars: Vec<char> = input.chars().collect();
    let cursor = input.cursor.min(chars.len());
    if chars.len() < width {
        return (input.to_string(), cursor);
    }
    let room = width.saturating_sub(1).max(1);
    let start = (cursor + 1).saturating_sub(room);
    if start == 0 {
        return (chars[..room.min(chars.len())].iter().collect(), cursor);
    }
    let end = (start + room).min(chars.len());
    let shown: String = chars[start..end].iter().collect();
    (format!("\u{2026}{shown}"), 1 + cursor - start)
}
