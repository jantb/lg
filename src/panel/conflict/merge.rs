use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{state::MergeEditor, ui};

/// Align two sequences by unchanged lines. Bound the table so a generated
/// conflict cannot stall the UI or allocate quadratic amounts of memory.
fn align(left: &[&str], right: &[&str]) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (left.len(), right.len());
    if n.saturating_mul(m) > 250_000 {
        return (0..n.max(m))
            .map(|i| ((i < n).then_some(i), (i < m).then_some(i)))
            .collect();
    }
    let mut lengths = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i * (m + 1) + j] = if left[i] == right[j] {
                1 + lengths[(i + 1) * (m + 1) + j + 1]
            } else {
                lengths[(i + 1) * (m + 1) + j].max(lengths[i * (m + 1) + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut rows = Vec::new();
    while i < n || j < m {
        if i < n && j < m && left[i] == right[j] {
            rows.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if i < n
            && (j == m || lengths[(i + 1) * (m + 1) + j] >= lengths[i * (m + 1) + j + 1])
        {
            rows.push((Some(i), None));
            i += 1;
        } else {
            rows.push((None, Some(j)));
            j += 1;
        }
    }
    // Pair deletions and insertions between shared anchors as replacements,
    // rather than pushing the two versions of one changed line apart.
    let mut aligned = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        if rows[index].0.is_some() && rows[index].1.is_some() {
            aligned.push(rows[index]);
            index += 1;
            continue;
        }
        let mut deleted = Vec::new();
        let mut inserted = Vec::new();
        while index < rows.len() && (rows[index].0.is_none() || rows[index].1.is_none()) {
            if let Some(i) = rows[index].0 {
                deleted.push(i);
            }
            if let Some(j) = rows[index].1 {
                inserted.push(j);
            }
            index += 1;
        }
        for offset in 0..deleted.len().max(inserted.len()) {
            aligned.push((deleted.get(offset).copied(), inserted.get(offset).copied()));
        }
    }
    aligned
}

fn aligned_rows(ours: &[&str], result: &[&str], theirs: &[&str]) -> Vec<[Option<usize>; 3]> {
    let left = align(result, ours);
    let right = align(result, theirs);
    let (mut l, mut r) = (0, 0);
    let mut rows = Vec::new();
    for center in 0..=result.len() {
        while left.get(l).is_some_and(|(anchor, _)| anchor.is_none())
            || right.get(r).is_some_and(|(anchor, _)| anchor.is_none())
        {
            let a = if left.get(l).is_some_and(|(anchor, _)| anchor.is_none()) {
                let value = left[l].1;
                l += 1;
                value
            } else {
                None
            };
            let b = if right.get(r).is_some_and(|(anchor, _)| anchor.is_none()) {
                let value = right[r].1;
                r += 1;
                value
            } else {
                None
            };
            rows.push([a, None, b]);
        }
        if center < result.len() {
            rows.push([
                left.get(l).and_then(|row| row.1),
                Some(center),
                right.get(r).and_then(|row| row.1),
            ]);
            l += 1;
            r += 1;
        }
    }
    rows
}

use crate::state::MergeAction;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

const ACTIONS: [(MergeAction, &str, &str); 11] = [
    (
        MergeAction::ReplaceOurs,
        "→ Ours",
        "Replace result with ours",
    ),
    (
        MergeAction::InsertOurs,
        "+ Ours",
        "Insert ours at the result cursor",
    ),
    (
        MergeAction::ReplaceTheirs,
        "Theirs ←",
        "Replace result with theirs",
    ),
    (
        MergeAction::InsertTheirs,
        "+ Theirs",
        "Insert theirs at the result cursor",
    ),
    (
        MergeAction::Both,
        "Both",
        "Replace result with ours followed by theirs",
    ),
    (MergeAction::Keep, "✓ Keep", "Accept the current result"),
    (MergeAction::Edit, "Edit", "Edit the current result"),
    (
        MergeAction::Save,
        "Save",
        "Save this file after every conflict is accepted",
    ),
    (MergeAction::Base, "Base", "Toggle the common ancestor"),
    (MergeAction::Previous, "‹ Prev", "Previous conflict"),
    (MergeAction::Next, "Next ›", "Next conflict"),
];

struct Button {
    action: MergeAction,
    label: &'static str,
    area: Rect,
}

fn buttons(area: Rect) -> Vec<Button> {
    let (mut x, mut y) = (area.x, area.y.saturating_add(1));
    let mut buttons = Vec::new();
    for (action, label, _) in ACTIONS {
        let width = (label.chars().count() as u16 + 4).min(area.width);
        if x > area.x && x.saturating_add(width) > area.right() {
            x = area.x;
            y = y.saturating_add(1);
        }
        if y >= area.bottom() {
            break;
        }
        buttons.push(Button {
            action,
            label,
            area: Rect::new(x, y, width, 1),
        });
        x = x.saturating_add(width + 1);
    }
    buttons
}

fn content_area(area: Rect) -> Rect {
    let top = buttons(area)
        .last()
        .map_or(area.y, |button| button.area.bottom());
    Rect::new(area.x, top, area.width, area.bottom().saturating_sub(top))
}

fn panes(area: Rect) -> [Rect; 3] {
    let body = content_area(area);
    let panes = Layout::default()
        .direction(if area.width >= 84 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(body);
    [panes[0], panes[1], panes[2]]
}

fn display(text: &str) -> String {
    text.replace('\t', "    ")
}

struct View {
    texts: [Vec<String>; 3],
    rows: Vec<[Option<usize>; 3]>,
    before: Vec<String>,
    after: Vec<String>,
    scroll: usize,
    horizontal: usize,
    cursor_row: usize,
    cursor_width: usize,
}

fn view(editor: &MergeEditor, area: Rect, result: &str, preview: bool) -> View {
    let hunk = editor.current();
    let texts = [
        hunk.source.ours.lines().map(display).collect::<Vec<_>>(),
        result
            .split('\n')
            .map(|line| display(line.trim_end_matches('\r')))
            .collect(),
        hunk.source.theirs.lines().map(display).collect(),
    ];
    let references: Vec<Vec<_>> = texts
        .iter()
        .map(|lines| lines.iter().map(String::as_str).collect())
        .collect();
    let rows = aligned_rows(&references[0], &references[1], &references[2]);
    let (before, after) = editor.context();
    let before: Vec<_> = before.lines().map(display).collect();
    let after: Vec<_> = after.lines().map(display).collect();
    let (cursor_line, _) = hunk.cursor_position();
    let cursor_row = rows
        .iter()
        .position(|row| row[1] == Some(cursor_line))
        .unwrap_or(0)
        + before.len();
    let height = panes(area)[1].height.saturating_sub(2).max(1) as usize;
    let scroll = if editor.editing && editor.follow_cursor && !preview {
        if cursor_row < editor.scroll {
            cursor_row
        } else if cursor_row >= editor.scroll + height {
            cursor_row + 1 - height
        } else {
            editor.scroll
        }
    } else {
        editor
            .scroll
            .min((before.len() + rows.len() + after.len()).saturating_sub(height))
            .max(before.len().saturating_sub(height.saturating_sub(1)))
    };
    let cursor_prefix = hunk.result[..hunk.cursor]
        .rsplit('\n')
        .next()
        .unwrap_or_default();
    let cursor_width = Line::from(display(cursor_prefix)).width();
    let text_width = panes(area)[1].width.saturating_sub(8).max(1) as usize;
    let horizontal = if editor.editing && editor.follow_cursor && !preview {
        cursor_width.saturating_sub(text_width - 1)
    } else {
        editor.horizontal
    };
    View {
        texts,
        rows,
        before,
        after,
        scroll,
        horizontal,
        cursor_row,
        cursor_width,
    }
}

pub(super) fn render(editor: &MergeEditor, area: Rect, frame: &mut Frame) {
    if area.width < 12 || area.height < 4 {
        return;
    }
    let hunk = editor.current();
    let candidate = editor.hovered.and_then(|action| action.preview(hunk));
    let decided = editor.hunks.iter().filter(|h| h.accepted).count();
    let status = if let Some(action) = editor.hovered {
        let description = ACTIONS
            .iter()
            .find(|entry| entry.0 == action)
            .map_or("", |entry| entry.2);
        format!(
            "{}: {description}{}",
            if candidate.is_some() {
                "PREVIEW"
            } else {
                "Click"
            },
            if candidate.is_some() {
                " · click to apply; draft unchanged"
            } else {
                ""
            }
        )
    } else {
        format!(
            "Conflict {}/{} · {decided} accepted · {}{}",
            editor.selected + 1,
            editor.hunks.len(),
            if hunk.accepted {
                "result accepted"
            } else {
                "choose a side or edit the result"
            },
            if editor.dirty() {
                " · unsaved"
            } else if editor.saved {
                " · saved"
            } else {
                ""
            }
        )
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().fg(Color::Yellow)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    for button in buttons(area) {
        let hovered = editor.hovered == Some(button.action);
        frame.render_widget(
            Paragraph::new(format!("[ {} ]", button.label)).style(if hovered {
                Style::default().fg(Color::Black).bg(Color::LightCyan)
            } else {
                Style::default()
                    .fg(Color::LightCyan)
                    .bg(Color::Rgb(28, 35, 49))
            }),
            button.area,
        );
    }
    if editor.show_base && candidate.is_none() {
        let (before, after) = editor.context();
        let (title, text) = if let Some(base) = &hunk.source.base {
            (
                "Ancestor · selected conflict · Base returns",
                format!("{before}{base}{after}"),
            )
        } else {
            (
                "Ancestor · full file (hunk could not be mapped) · Base returns",
                editor.snapshot.base.clone(),
            )
        };
        let body = content_area(area);
        let lines: Vec<_> = text
            .lines()
            .enumerate()
            .skip(editor.scroll)
            .take(body.height.saturating_sub(2) as usize)
            .map(|(i, line)| Line::from(format!("{:>4}  {}", i + 1, display(line))))
            .collect();
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((0, editor.horizontal.min(u16::MAX as usize) as u16))
                .block(ui::bordered(title)),
            body,
        );
        return;
    }
    let panes = panes(area);
    let view = view(
        editor,
        area,
        candidate.as_deref().unwrap_or(&hunk.result),
        candidate.is_some(),
    );
    let titles = [
        format!("Ours · {}", hunk.source.ours_label),
        if candidate.is_some() {
            "Result · HOVER PREVIEW".into()
        } else if editor.editing {
            "Result · EDITING · Esc finishes".into()
        } else {
            "Result · click or Enter to edit".into()
        },
        format!("Theirs · {}", hunk.source.theirs_label),
    ];
    for pane in 0..3 {
        let mut lines: Vec<Line<'static>> = view
            .before
            .iter()
            .map(|line| {
                Line::styled(
                    format!("      {line}"),
                    Style::default().fg(Color::DarkGray),
                )
            })
            .collect();
        for row in &view.rows {
            let equal = row[0].zip(row[1]).zip(row[2]).is_some_and(|((a, b), c)| {
                view.texts[0][a] == view.texts[1][b] && view.texts[1][b] == view.texts[2][c]
            });
            let line = if let Some(index) = row[pane] {
                let color = if equal {
                    Color::Gray
                } else {
                    [Color::LightBlue, Color::LightGreen, Color::LightMagenta][pane]
                };
                let background = if equal {
                    Color::Reset
                } else {
                    [
                        Color::Rgb(20, 32, 48),
                        Color::Rgb(20, 42, 30),
                        Color::Rgb(43, 24, 43),
                    ][pane]
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:>4}{} ", index + 1, if equal { ' ' } else { '│' }),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        view.texts[pane][index].clone(),
                        Style::default().fg(color).bg(background),
                    ),
                ])
            } else {
                Line::styled("     ·", Style::default().fg(Color::DarkGray))
            };
            lines.push(line);
        }
        lines.extend(view.after.iter().map(|line| {
            Line::styled(
                format!("      {line}"),
                Style::default().fg(Color::DarkGray),
            )
        }));
        let visible: Vec<_> = lines
            .into_iter()
            .skip(view.scroll)
            .take(panes[pane].height.saturating_sub(2) as usize)
            .collect();
        frame.render_widget(
            Paragraph::new(visible)
                .scroll((0, view.horizontal.min(u16::MAX as usize) as u16))
                .block(
                    ui::bordered(&titles[pane]).border_style(Style::default().fg(
                        if pane == 1 && candidate.is_some() {
                            Color::Yellow
                        } else if pane == 1 && editor.editing {
                            Color::LightGreen
                        } else {
                            Color::DarkGray
                        },
                    )),
                ),
            panes[pane],
        );
    }
    let height = panes[1].height.saturating_sub(2) as usize;
    if editor.editing
        && candidate.is_none()
        && panes[1].width > 8
        && height > 0
        && (view.scroll..view.scroll + height).contains(&view.cursor_row)
    {
        frame.set_cursor_position(Position::new(
            panes[1].x
                + 1
                + (6 + view.cursor_width.saturating_sub(view.horizontal))
                    .min(panes[1].width.saturating_sub(3) as usize) as u16,
            panes[1].y + 1 + (view.cursor_row - view.scroll) as u16,
        ));
    }
}

pub(super) fn mouse(
    editor: &mut MergeEditor,
    area: Rect,
    event: &MouseEvent,
) -> Option<MergeAction> {
    let point = Position::new(event.column, event.row);
    let hovered = buttons(area)
        .iter()
        .find(|button| button.area.contains(point))
        .map(|button| button.action);
    match event.kind {
        MouseEventKind::Moved => editor.hovered = hovered,
        MouseEventKind::Down(MouseButton::Left) => {
            editor.hovered = None;
            if hovered.is_some() {
                return hovered;
            }
            let result_area = panes(area)[1];
            let inner = Rect::new(
                result_area.x + 1,
                result_area.y + 1,
                result_area.width.saturating_sub(2),
                result_area.height.saturating_sub(2),
            );
            if !editor.show_base && inner.contains(point) {
                let view = view(editor, area, &editor.current().result, false);
                let row = view.scroll + (event.row - inner.y) as usize;
                let aligned = row
                    .saturating_sub(view.before.len())
                    .min(view.rows.len().saturating_sub(1));
                let line_index = view.rows[aligned..]
                    .iter()
                    .find_map(|row| row[1])
                    .unwrap_or(view.texts[1].len().saturating_sub(1));
                let column =
                    ((event.column - inner.x) as usize + view.horizontal).saturating_sub(6);
                let hunk = editor.current_mut();
                let start: usize = hunk
                    .result
                    .split_inclusive('\n')
                    .take(line_index)
                    .map(str::len)
                    .sum();
                let line = hunk.result[start..]
                    .split('\n')
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('\r');
                let mut width = 0;
                let mut byte = line.len();
                for (index, c) in line.char_indices() {
                    let next = Line::from(display(&c.to_string())).width();
                    if width + next > column {
                        byte = index;
                        break;
                    }
                    width += next;
                }
                hunk.cursor = start + byte;
                editor.editing = true;
                editor.follow_cursor = true;
            }
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            editor.hovered = None;
            let current = view(editor, area, &editor.current().result, false);
            let max = if editor.show_base {
                editor
                    .current()
                    .source
                    .base
                    .as_ref()
                    .unwrap_or(&editor.snapshot.base)
                    .lines()
                    .count()
                    .saturating_sub(1)
            } else {
                current.before.len() + current.rows.len() + current.after.len()
            };
            editor.scroll = if event.kind == MouseEventKind::ScrollDown {
                current.scroll.saturating_add(3).min(max)
            } else {
                current.scroll.saturating_sub(3)
            };
            editor.follow_cursor = false;
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_retains_every_line_and_matches_common_anchors() {
        let ours = ["same", "left", "tail"];
        let result = ["same", "left", "right", "tail"];
        let theirs = ["same", "right", "tail"];
        let rows = aligned_rows(&ours, &result, &theirs);
        for (pane, count) in [ours.len(), result.len(), theirs.len()]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                rows.iter().filter_map(|row| row[pane]).collect::<Vec<_>>(),
                (0..count).collect::<Vec<_>>()
            );
        }
        assert_eq!(rows.first(), Some(&[Some(0), Some(0), Some(0)]));
        assert_eq!(rows.last(), Some(&[Some(2), Some(3), Some(2)]));
    }
}
