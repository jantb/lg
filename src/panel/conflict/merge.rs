//! The three-way merge view: the whole file laid out once, every conflict
//! under its own rule of buttons, with ours and theirs aligned beside the
//! result being written.

use ratatui::{
    Frame,
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    state::{MergeAction, MergeEditor, MergePart},
    ui,
};

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

/// Actions on the file as a whole; they sit in the toolbar above the panes.
const FILE_ACTIONS: [(MergeAction, &str, &str); 3] = [
    (
        MergeAction::Save,
        "Save",
        "Save this file after every conflict is accepted",
    ),
    (MergeAction::Previous, "‹ Prev", "Previous conflict"),
    (MergeAction::Next, "Next ›", "Next conflict"),
];

/// Actions on one conflict; they sit in the rule above that conflict.
const HUNK_ACTIONS: [(MergeAction, &str, &str); 8] = [
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
        MergeAction::Both,
        "Both",
        "Replace result with ours followed by theirs",
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
    (MergeAction::Keep, "✓ Keep", "Accept the current result"),
    (MergeAction::Edit, "Edit", "Edit the current result"),
    (MergeAction::Base, "Base", "Toggle the common ancestor"),
];

fn describe(action: MergeAction) -> &'static str {
    FILE_ACTIONS
        .iter()
        .chain(HUNK_ACTIONS.iter())
        .find(|entry| entry.0 == action)
        .map_or("", |entry| entry.2)
}

struct Button {
    /// The conflict this button acts on; `None` for the file-level toolbar.
    hunk: Option<usize>,
    action: MergeAction,
    label: &'static str,
    area: Rect,
}

fn button_width(label: &str) -> u16 {
    label.chars().count() as u16 + 4
}

fn toolbar(area: Rect) -> Vec<Button> {
    let (mut x, mut y) = (area.x, area.y.saturating_add(1));
    let mut buttons = Vec::new();
    for (action, label, _) in FILE_ACTIONS {
        let width = button_width(label).min(area.width);
        if x > area.x && x.saturating_add(width) > area.right() {
            x = area.x;
            y = y.saturating_add(1);
        }
        if y >= area.bottom() {
            break;
        }
        buttons.push(Button {
            hunk: None,
            action,
            label,
            area: Rect::new(x, y, width, 1),
        });
        x = x.saturating_add(width + 1);
    }
    buttons
}

fn content_area(area: Rect) -> Rect {
    let top = toolbar(area)
        .last()
        .map_or(area.y, |button| button.area.bottom());
    Rect::new(area.x, top, area.width, area.bottom().saturating_sub(top))
}

fn side_by_side(area: Rect) -> bool {
    area.width >= 84
}

fn panes(area: Rect) -> [Rect; 3] {
    let body = content_area(area);
    let panes = Layout::default()
        .direction(if side_by_side(area) {
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

fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

/// Where a conflict's rule is drawn: once across all three panes when they sit
/// side by side, otherwise inside each stacked pane.
fn rule_areas(area: Rect) -> Vec<Rect> {
    let panes = panes(area);
    if side_by_side(area) {
        let body = inner(content_area(area));
        vec![Rect::new(body.x, panes[1].y + 1, body.width, body.height)]
    } else {
        panes.iter().map(|pane| inner(*pane)).collect()
    }
}

/// Room at the left of a conflict's rule for its title.
const RULE_TITLE_WIDTH: u16 = 22;

/// Where each button of a conflict's rule sits: (line, x offset) within a
/// rule `width` wide. Buttons wrap to further lines when the rule is narrow.
fn rule_slots(width: u16) -> Vec<(usize, u16, MergeAction, &'static str)> {
    let mut slots = Vec::new();
    let (mut line, mut x) = (0usize, RULE_TITLE_WIDTH.min(width));
    for (action, label, _) in HUNK_ACTIONS {
        let w = button_width(label);
        if x > 0 && x.saturating_add(w) > width {
            line += 1;
            x = 0;
        }
        slots.push((line, x, action, label));
        x = x.saturating_add(w + 1);
    }
    slots
}

fn rule_height(width: u16) -> usize {
    rule_slots(width).last().map_or(1, |slot| slot.0 + 1)
}

fn display(text: &str) -> String {
    text.replace('\t', "    ")
}

/// One screen row of the laid-out file, identical in every pane.
enum Row {
    /// Text git merged on its own, with its line number in each version.
    Context { text: String, numbers: [usize; 3] },
    /// One line of the rule of buttons above a conflict.
    Rule { hunk: usize, line: usize },
    /// One aligned row of a conflict: the line index shown in each pane.
    Hunk {
        hunk: usize,
        lines: [Option<usize>; 3],
    },
}

struct HunkView {
    texts: [Vec<String>; 3],
    /// First file line number of the conflict in each pane.
    start: [usize; 3],
    /// Lines the result holds; the display may add one empty line after a
    /// trailing newline for the cursor to sit on.
    result_lines: usize,
    /// Row of the first rule line.
    rule: usize,
}

struct Doc {
    rows: Vec<Row>,
    hunks: Vec<HunkView>,
}

fn document(editor: &MergeEditor, results: &[&str], rule_height: usize) -> Doc {
    let mut counters = [0usize; 3];
    let mut rows = Vec::new();
    let mut hunks = Vec::with_capacity(editor.hunks.len());
    for part in editor.parts() {
        match part {
            MergePart::Kept(text) => {
                for line in text.lines() {
                    for counter in &mut counters {
                        *counter += 1;
                    }
                    rows.push(Row::Context {
                        text: display(line),
                        numbers: counters,
                    });
                }
            }
            MergePart::Conflict(index) => {
                let hunk = &editor.hunks[index];
                let result = results[index];
                let mut result_text: Vec<_> = result
                    .split('\n')
                    .map(|line| display(line.trim_end_matches('\r')))
                    .collect();
                // The line after a trailing newline exists for the cursor to
                // sit on; away from the cursor it is only an empty row.
                if result.ends_with('\n') && !(editor.editing && editor.selected == index) {
                    result_text.pop();
                }
                let texts = [
                    hunk.source.ours.lines().map(display).collect::<Vec<_>>(),
                    result_text,
                    hunk.source.theirs.lines().map(display).collect(),
                ];
                let references: Vec<Vec<_>> = texts
                    .iter()
                    .map(|lines| lines.iter().map(String::as_str).collect())
                    .collect();
                let rule = rows.len();
                for line in 0..rule_height {
                    rows.push(Row::Rule { hunk: index, line });
                }
                for lines in aligned_rows(&references[0], &references[1], &references[2]) {
                    rows.push(Row::Hunk { hunk: index, lines });
                }
                let result_lines = result.split_inclusive('\n').count();
                let start = counters.map(|counter| counter + 1);
                counters[0] += texts[0].len();
                counters[1] += result_lines;
                counters[2] += texts[2].len();
                hunks.push(HunkView {
                    texts,
                    start,
                    result_lines,
                    rule,
                });
            }
        }
    }
    Doc { rows, hunks }
}

struct View {
    doc: Doc,
    scroll: usize,
    horizontal: usize,
    cursor_row: usize,
    cursor_width: usize,
    /// Rows the result pane can show.
    height: usize,
}

/// Lay the file out for `area`, with one conflict's result swapped for a
/// hover preview when there is one.
fn view(editor: &MergeEditor, area: Rect, preview: Option<(usize, &str)>) -> View {
    let results: Vec<&str> = editor
        .hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| match preview {
            Some((target, text)) if target == index => text,
            _ => hunk.result.as_str(),
        })
        .collect();
    let rule_width = rule_areas(area)
        .first()
        .map_or(area.width, |rule| rule.width);
    let doc = document(editor, &results, rule_height(rule_width));
    let hunk = editor.current();
    let (cursor_line, _) = hunk.cursor_position();
    let selected = &doc.hunks[editor.selected];
    let cursor_row = doc
        .rows
        .iter()
        .position(|row| {
            matches!(row, Row::Hunk { hunk, lines } if *hunk == editor.selected && lines[1] == Some(cursor_line))
        })
        .unwrap_or(selected.rule);
    let height = inner(panes(area)[1]).height.max(1) as usize;
    let mut scroll = if editor.reveal {
        // Two lines of context above the rule keep the conflict placed.
        selected.rule.saturating_sub(2)
    } else {
        editor.scroll
    };
    if editor.follow_cursor {
        if cursor_row < scroll {
            scroll = cursor_row;
        } else if cursor_row >= scroll + height {
            scroll = cursor_row + 1 - height;
        }
    }
    let scroll = scroll.min(doc.rows.len().saturating_sub(height));
    let cursor_prefix = hunk.result[..hunk.cursor]
        .rsplit('\n')
        .next()
        .unwrap_or_default();
    let cursor_width = Line::from(display(cursor_prefix)).width();
    let text_width = panes(area)[1].width.saturating_sub(8).max(1) as usize;
    let horizontal = if editor.editing && editor.follow_cursor && preview.is_none() {
        cursor_width.saturating_sub(text_width - 1)
    } else {
        editor.horizontal
    };
    View {
        doc,
        scroll,
        horizontal,
        cursor_row,
        cursor_width,
        height,
    }
}

/// Every button on screen: the toolbar, then each visible conflict's rule.
fn buttons(area: Rect, view: &View) -> Vec<Button> {
    let mut buttons = toolbar(area);
    for rule in rule_areas(area) {
        let slots = rule_slots(rule.width);
        for (offset, row) in view
            .doc
            .rows
            .iter()
            .skip(view.scroll)
            .take(rule.height as usize)
            .enumerate()
        {
            let Row::Rule { hunk, line } = row else {
                continue;
            };
            let y = rule.y + offset as u16;
            for (slot_line, x, action, label) in &slots {
                if slot_line != line || *x >= rule.width {
                    continue;
                }
                buttons.push(Button {
                    hunk: Some(*hunk),
                    action: *action,
                    label,
                    area: Rect::new(rule.x + x, y, button_width(label).min(rule.width - x), 1),
                });
            }
        }
    }
    buttons
}

fn button_style(hovered: bool) -> Style {
    if hovered {
        Style::default().fg(Color::Black).bg(Color::LightCyan)
    } else {
        Style::default()
            .fg(Color::LightCyan)
            .bg(Color::Rgb(28, 35, 49))
    }
}

/// The preview a hovered button would produce, if it changes the result.
fn hover_preview(editor: &MergeEditor) -> Option<(usize, String)> {
    let (hunk, action) = editor.hovered?;
    let text = action.preview(editor.hunks.get(hunk)?)?;
    Some((hunk, text))
}

fn render_base(editor: &MergeEditor, area: Rect, frame: &mut Frame) {
    let hunk = editor.current();
    let (title, text) = if let Some(base) = &hunk.source.base {
        (
            format!(
                "Ancestor · conflict {}/{} · Base returns",
                editor.selected + 1,
                editor.hunks.len()
            ),
            base.clone(),
        )
    } else {
        (
            "Ancestor · full file (conflict could not be mapped) · Base returns".to_string(),
            editor.snapshot.base.clone(),
        )
    };
    let body = content_area(area);
    let lines: Vec<_> = text
        .lines()
        .enumerate()
        .skip(editor.base_scroll)
        .take(body.height.saturating_sub(2) as usize)
        .map(|(i, line)| Line::from(format!("{:>4}  {}", i + 1, display(line))))
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((0, editor.horizontal.min(u16::MAX as usize) as u16))
            .block(ui::bordered(&title)),
        body,
    );
}

fn gutter(number: Option<usize>, marker: char, selected: bool) -> Span<'static> {
    Span::styled(
        match number {
            Some(number) => format!("{number:>4}{marker} "),
            None => format!("    {marker} "),
        },
        Style::default().fg(if selected {
            Color::Yellow
        } else {
            Color::DarkGray
        }),
    )
}

pub(super) fn render(editor: &MergeEditor, area: Rect, frame: &mut Frame) {
    if area.width < 12 || area.height < 4 {
        return;
    }
    let hunk = editor.current();
    let candidate = hover_preview(editor);
    let decided = editor.hunks.iter().filter(|h| h.accepted).count();
    let status = if let Some((target, action)) = editor.hovered {
        if candidate.is_some() {
            format!(
                "PREVIEW conflict {}: {} · click to apply; draft unchanged",
                target + 1,
                describe(action)
            )
        } else {
            format!("Click: {}", describe(action))
        }
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
    for button in toolbar(area) {
        let hovered = editor
            .hovered
            .is_some_and(|(_, action)| action == button.action);
        frame.render_widget(
            Paragraph::new(format!("[ {} ]", button.label)).style(button_style(hovered)),
            button.area,
        );
    }
    if editor.show_base && candidate.is_none() {
        render_base(editor, area, frame);
        return;
    }
    let panes = panes(area);
    let view = view(
        editor,
        area,
        candidate
            .as_ref()
            .map(|(hunk, text)| (*hunk, text.as_str())),
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
        let visible = view
            .doc
            .rows
            .iter()
            .skip(view.scroll)
            .take(inner(panes[pane]).height as usize);
        let lines: Vec<Line<'static>> = visible
            .map(|row| match row {
                Row::Context { text, numbers } => Line::from(vec![
                    gutter(Some(numbers[pane]), ' ', false),
                    Span::styled(text.clone(), Style::default().fg(Color::DarkGray)),
                ]),
                Row::Rule { .. } => Line::default(),
                Row::Hunk { hunk, lines } => {
                    let hunk_view = &view.doc.hunks[*hunk];
                    let selected = *hunk == editor.selected;
                    let equal = lines[0]
                        .zip(lines[1])
                        .zip(lines[2])
                        .is_some_and(|((a, b), c)| {
                            hunk_view.texts[0][a] == hunk_view.texts[1][b]
                                && hunk_view.texts[1][b] == hunk_view.texts[2][c]
                        });
                    let Some(index) = lines[pane] else {
                        return Line::from(gutter(None, '·', selected));
                    };
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
                    let number = (pane != 1 || index < hunk_view.result_lines)
                        .then_some(hunk_view.start[pane] + index);
                    Line::from(vec![
                        gutter(number, if equal { ' ' } else { '│' }, selected),
                        Span::styled(
                            hunk_view.texts[pane][index].clone(),
                            Style::default().fg(color).bg(background),
                        ),
                    ])
                }
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines)
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
    render_rules(editor, area, &view, frame);
    if editor.editing
        && candidate.is_none()
        && panes[1].width > 8
        && view.height > 0
        && (view.scroll..view.scroll + view.height).contains(&view.cursor_row)
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

/// Draw every visible conflict's rule: a title naming and grading it, then
/// its buttons, over the panes' otherwise empty rows.
fn render_rules(editor: &MergeEditor, area: Rect, view: &View, frame: &mut Frame) {
    for rule in rule_areas(area) {
        for (offset, row) in view
            .doc
            .rows
            .iter()
            .skip(view.scroll)
            .take(rule.height as usize)
            .enumerate()
        {
            let Row::Rule { hunk, line } = row else {
                continue;
            };
            let y = rule.y + offset as u16;
            frame.render_widget(
                Paragraph::new("─".repeat(rule.width as usize))
                    .style(Style::default().fg(Color::DarkGray)),
                Rect::new(rule.x, y, rule.width, 1),
            );
            if *line == 0 {
                let accepted = editor.hunks[*hunk].accepted;
                let selected = *hunk == editor.selected;
                let title = Line::from(vec![
                    Span::styled(
                        format!(" Conflict {}/{} ", hunk + 1, editor.hunks.len()),
                        Style::default()
                            .fg(if selected { Color::Yellow } else { Color::Gray })
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        if accepted { "✓ " } else { "? " },
                        Style::default().fg(if accepted {
                            Color::LightGreen
                        } else {
                            Color::Yellow
                        }),
                    ),
                ]);
                frame.render_widget(
                    Paragraph::new(title),
                    Rect::new(rule.x, y, RULE_TITLE_WIDTH.min(rule.width), 1),
                );
            }
        }
    }
    for button in buttons(area, view) {
        let Some(hunk) = button.hunk else {
            continue;
        };
        let hovered = editor.hovered == Some((hunk, button.action));
        frame.render_widget(
            Paragraph::new(format!("[ {} ]", button.label)).style(button_style(hovered)),
            button.area,
        );
    }
}

/// Scroll the view by `delta` rows from where it is on screen, and stop
/// following the cursor or the selected conflict.
pub(super) fn scroll_by(editor: &mut MergeEditor, delta: isize) {
    if editor.show_base {
        let lines = editor
            .current()
            .source
            .base
            .as_ref()
            .unwrap_or(&editor.snapshot.base)
            .lines()
            .count();
        editor.base_scroll = editor
            .base_scroll
            .saturating_add_signed(delta)
            .min(lines.saturating_sub(1));
        return;
    }
    let area = editor.viewport.unwrap_or(Rect::new(0, 0, 120, 40));
    let view = view(editor, area, None);
    editor.scroll = view
        .scroll
        .saturating_add_signed(delta)
        .min(view.doc.rows.len().saturating_sub(view.height));
    editor.reveal = false;
    editor.follow_cursor = false;
}

/// Place the result cursor where the user clicked on `row` of the result pane.
fn click_result(editor: &mut MergeEditor, view: &View, row: usize, column: usize) {
    let Some(Row::Hunk { hunk, .. }) = view.doc.rows.get(row) else {
        return;
    };
    let hunk_index = *hunk;
    let hunk_view = &view.doc.hunks[hunk_index];
    // A padding row carries no result line; take the next one in the conflict.
    let line_index = view.doc.rows[row..]
        .iter()
        .take_while(|row| matches!(row, Row::Hunk { hunk, .. } if *hunk == hunk_index))
        .find_map(|row| match row {
            Row::Hunk { lines, .. } => lines[1],
            _ => None,
        })
        .unwrap_or(hunk_view.texts[1].len().saturating_sub(1));
    let column = (column + view.horizontal).saturating_sub(6);
    editor.select(hunk_index);
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

pub(super) fn mouse(
    editor: &mut MergeEditor,
    area: Rect,
    event: &MouseEvent,
) -> Option<(usize, MergeAction)> {
    if area.width < 12 || area.height < 4 {
        return None;
    }
    editor.viewport = Some(area);
    let point = Position::new(event.column, event.row);
    // Hit-test against what is drawn: a preview may change how many rows a
    // conflict takes, and the buttons below it move with them.
    let preview = hover_preview(editor);
    let view = view(
        editor,
        area,
        preview.as_ref().map(|(hunk, text)| (*hunk, text.as_str())),
    );
    let hit = buttons(area, &view)
        .into_iter()
        .find(|button| button.area.contains(point))
        .map(|button| (button.hunk.unwrap_or(editor.selected), button.action));
    match event.kind {
        MouseEventKind::Moved => editor.hovered = hit,
        MouseEventKind::Down(MouseButton::Left) => {
            editor.hovered = None;
            // The user acts on what is on screen; keep the view where it is.
            editor.scroll = view.scroll;
            editor.reveal = false;
            if hit.is_some() {
                return hit;
            }
            if editor.show_base {
                return None;
            }
            let result = inner(panes(area)[1]);
            if !result.contains(point) {
                return None;
            }
            let row = view.scroll + (event.row - result.y) as usize;
            match view.doc.rows.get(row) {
                Some(Row::Rule { hunk, .. }) => editor.select(*hunk),
                Some(Row::Hunk { .. }) => {
                    click_result(editor, &view, row, (event.column - result.x) as usize)
                }
                _ => {}
            }
        }
        MouseEventKind::ScrollDown => {
            editor.hovered = None;
            scroll_by(editor, 3);
        }
        MouseEventKind::ScrollUp => {
            editor.hovered = None;
            scroll_by(editor, -3);
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

    #[test]
    fn rule_buttons_wrap_instead_of_running_past_a_narrow_rule() {
        let wide = rule_slots(160);
        assert!(wide.iter().all(|slot| slot.0 == 0));
        let narrow = rule_slots(40);
        assert!(rule_height(40) > 1);
        for (line, x, _, label) in narrow {
            assert!(x + button_width(label) <= 40 || (line > 0 && x == 0));
        }
    }
}
