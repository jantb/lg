//! Aligning the three sides and working out where every row and button sits.

use super::*;

/// Align two sequences by unchanged lines. Bound the table so a generated
/// conflict cannot stall the UI or allocate quadratic amounts of memory.
pub(super) fn align(left: &[&str], right: &[&str]) -> Vec<(Option<usize>, Option<usize>)> {
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

pub(super) fn aligned_rows(
    ours: &[&str],
    result: &[&str],
    theirs: &[&str],
) -> Vec<[Option<usize>; 3]> {
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

pub(super) struct Button {
    /// The conflict this button acts on; `None` for the file-level toolbar.
    pub(super) hunk: Option<usize>,
    pub(super) action: MergeAction,
    pub(super) label: &'static str,
    pub(super) area: Rect,
    /// Drawn as `[ label ]`; gutter buttons are drawn bare.
    pub(super) framed: bool,
}

pub(super) fn button_width(label: &str) -> u16 {
    label.chars().count() as u16 + 4
}

pub(super) fn toolbar(area: Rect) -> Vec<Button> {
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
            framed: true,
        });
        x = x.saturating_add(width + 1);
    }
    buttons
}

pub(super) fn content_area(area: Rect) -> Rect {
    let top = toolbar(area)
        .last()
        .map_or(area.y, |button| button.area.bottom());
    Rect::new(area.x, top, area.width, area.bottom().saturating_sub(top))
}

pub(super) fn side_by_side(area: Rect) -> bool {
    area.width >= 84
}

pub(super) fn panes(area: Rect) -> [Rect; 3] {
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

pub(super) fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

/// Where a conflict's rule is drawn: once across all three panes when they sit
/// side by side, otherwise inside each stacked pane.
pub(super) fn rule_areas(area: Rect) -> Vec<Rect> {
    let panes = panes(area);
    if side_by_side(area) {
        let body = inner(content_area(area));
        vec![Rect::new(body.x, panes[1].y + 1, body.width, body.height)]
    } else {
        panes.iter().map(|pane| inner(*pane)).collect()
    }
}

/// Room at the left of a conflict's rule for its title.
pub(super) const RULE_TITLE_WIDTH: u16 = 62;

/// Where each button of a conflict's rule sits: (line, x offset) within a
/// rule `width` wide. Buttons wrap to further lines when the rule is narrow.
pub(super) fn rule_slots(width: u16) -> Vec<(usize, u16, MergeAction, &'static str)> {
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

pub(super) fn rule_height(width: u16) -> usize {
    rule_slots(width).last().map_or(1, |slot| slot.0 + 1)
}

pub(super) fn display(text: &str) -> String {
    text.replace('\t', "    ")
}

/// One screen row of the laid-out file, identical in every pane.
pub(super) enum Row {
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

pub(super) struct HunkView {
    pub(super) texts: [Vec<String>; 3],
    /// First file line number of the conflict in each pane.
    pub(super) start: [usize; 3],
    /// Lines the result holds; the display may add one empty line after a
    /// trailing newline for the cursor to sit on.
    pub(super) result_lines: usize,
    /// Row of the first rule line, and of the first row of the conflict
    /// itself under it.
    pub(super) rule: usize,
    pub(super) first: usize,
}

pub(super) struct Doc {
    pub(super) rows: Vec<Row>,
    pub(super) hunks: Vec<HunkView>,
}

pub(super) fn document(editor: &MergeEditor, results: &[&str], rule_height: usize) -> Doc {
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
                // sit on, as does the one line of an empty result; away from
                // the cursor they are only empty rows.
                if !(editor.editing && editor.selected == index) {
                    if result.is_empty() {
                        result_text.clear();
                    } else if result.ends_with('\n') {
                        result_text.pop();
                    }
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
                let first = rows.len();
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
                    first,
                });
            }
        }
    }
    Doc { rows, hunks }
}

pub(super) struct View {
    pub(super) doc: Doc,
    pub(super) scroll: usize,
    pub(super) horizontal: usize,
    pub(super) cursor_row: usize,
    pub(super) cursor_width: usize,
    /// Rows the result pane can show.
    pub(super) height: usize,
}

/// Lay the file out for `area`, with one conflict's result swapped for a
/// hover preview when there is one.
pub(super) fn view(editor: &MergeEditor, area: Rect, preview: Option<(usize, &str)>) -> View {
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

/// The six cells the gutter buttons of conflict `hunk` take at its first
/// row, when that row is on screen: at the right edge of ours, and in the
/// line-number gutter of theirs.
pub(super) fn gutter_slots(area: Rect, view: &View, hunk: usize) -> Option<(Rect, Rect)> {
    let first = view.doc.hunks[hunk].first;
    if first < view.scroll || first >= view.scroll + view.height {
        return None;
    }
    let panes = panes(area);
    let (ours, theirs) = (inner(panes[0]), inner(panes[2]));
    if ours.width < 12 || theirs.width < 12 {
        return None;
    }
    let offset = (first - view.scroll) as u16;
    Some((
        Rect::new(ours.right() - 6, ours.y + offset, 6, 1),
        Rect::new(theirs.x, theirs.y + offset, 6, 1),
    ))
}

/// The gutter buttons beside conflict `hunk`: `X >>` at the right edge of
/// ours, `<< X` in the gutter of theirs.
pub(super) fn gutter_buttons(area: Rect, view: &View, hunk: usize) -> Vec<Button> {
    let Some((ours, theirs)) = gutter_slots(area, view, hunk) else {
        return Vec::new();
    };
    let button = |action, label, x, y, width| Button {
        hunk: Some(hunk),
        action,
        label,
        area: Rect::new(x, y, width, 1),
        framed: false,
    };
    vec![
        button(MergeAction::Reset, "X", ours.x + 1, ours.y, 1),
        button(MergeAction::AcceptOurs, ">>", ours.x + 3, ours.y, 2),
        button(MergeAction::AcceptTheirs, "<<", theirs.x, theirs.y, 2),
        button(MergeAction::Reset, "X", theirs.x + 3, theirs.y, 1),
    ]
}

/// Every button on screen: the toolbar, then each visible conflict's rule
/// and gutter.
pub(super) fn buttons(area: Rect, view: &View) -> Vec<Button> {
    let mut buttons = toolbar(area);
    for hunk in 0..view.doc.hunks.len() {
        buttons.extend(gutter_buttons(area, view, hunk));
    }
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
                    framed: true,
                });
            }
        }
    }
    buttons
}

pub(super) fn button_style(hovered: bool) -> Style {
    if hovered {
        Style::default().fg(Color::Black).bg(Color::LightCyan)
    } else {
        Style::default()
            .fg(Color::LightCyan)
            .bg(Color::Rgb(28, 35, 49))
    }
}

/// The preview a hovered button would produce, if it changes the result.
pub(super) fn hover_preview(editor: &MergeEditor) -> Option<(usize, String)> {
    let (hunk, action) = editor.hovered?;
    let text = action.preview(editor.hunks.get(hunk)?)?;
    Some((hunk, text))
}
