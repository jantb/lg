//! Painting the merge view and its rules of buttons.

use super::*;

pub(super) fn render_base(editor: &MergeEditor, area: Rect, frame: &mut Frame) {
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

/// A row's line number and marker. Rows of a conflict take its colour,
/// `Some((settled, selected))`: green once it is settled, amber while it is
/// not, bold for the selected conflict.
pub(super) fn gutter(
    number: Option<usize>,
    marker: char,
    state: Option<(bool, bool)>,
) -> Span<'static> {
    let mut style = Style::default().fg(match state {
        Some((true, _)) => Color::LightGreen,
        Some((false, _)) => Color::Yellow,
        None => Color::DarkGray,
    });
    if state.is_some_and(|(_, selected)| selected) {
        style = style.add_modifier(Modifier::BOLD);
    }
    Span::styled(
        match number {
            Some(number) => format!("{number:>4}{marker} "),
            None => format!("    {marker} "),
        },
        style,
    )
}

/// Draw the editor; `notes` is how the local model settled each conflict,
/// when it was the local model that did.
pub(crate) fn render(editor: &MergeEditor, notes: &[String], area: Rect, frame: &mut Frame) {
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
                "settled"
            } else {
                "take a side (>> <<), both, or edit the result"
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
                    gutter(Some(numbers[pane]), ' ', None),
                    Span::styled(text.clone(), Style::default().fg(Color::DarkGray)),
                ]),
                Row::Rule { .. } => Line::default(),
                Row::Hunk { hunk, lines } => {
                    let hunk_view = &view.doc.hunks[*hunk];
                    let selected = Some((editor.hunks[*hunk].accepted, *hunk == editor.selected));
                    let equal = lines[0]
                        .zip(lines[1])
                        .zip(lines[2])
                        .is_some_and(|((a, b), c)| {
                            hunk_view.texts[0][a] == hunk_view.texts[1][b]
                                && hunk_view.texts[1][b] == hunk_view.texts[2][c]
                        });
                    let Some(index) = lines[pane] else {
                        // The result's first row says what an empty region
                        // is: a conflict still to settle, or settled as nothing.
                        if pane == 1
                            && hunk_view.texts[1].is_empty()
                            && view
                                .doc
                                .rows
                                .get(hunk_view.first)
                                .is_some_and(|r| std::ptr::eq(r, row))
                        {
                            let accepted = editor.hunks[*hunk].accepted;
                            return Line::from(vec![
                                gutter(None, '┆', selected),
                                Span::styled(
                                    if accepted {
                                        "╌╌ settled as nothing ╌╌".to_string()
                                    } else {
                                        "╌╌ unresolved · take a side or edit ╌╌".to_string()
                                    },
                                    Style::default()
                                        .fg(if accepted {
                                            Color::DarkGray
                                        } else {
                                            Color::Yellow
                                        })
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]);
                        }
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
    render_rules(editor, notes, area, &view, frame);
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
pub(super) fn render_rules(
    editor: &MergeEditor,
    notes: &[String],
    area: Rect,
    view: &View,
    frame: &mut Frame,
) {
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
                let hunk_state = &editor.hunks[*hunk];
                let selected = *hunk == editor.selected;
                // The banner says at a glance where the conflict stands: what
                // it still needs, or what settled it.
                let (text, background) = if hunk_state.accepted {
                    let how = match hunk_state.applied.as_slice() {
                        _ if hunk_state.at_start() && notes.get(*hunk).is_some() => {
                            format!("local model {}", notes[*hunk])
                        }
                        [Side::Ours] => "ours".to_string(),
                        [Side::Theirs] => "theirs".to_string(),
                        [Side::Ours, Side::Theirs] => "ours + theirs".to_string(),
                        [Side::Theirs, Side::Ours] => "theirs + ours".to_string(),
                        _ if hunk_state.at_start() => "as found".to_string(),
                        _ => "edited".to_string(),
                    };
                    (
                        format!(
                            " ✓ Conflict {}/{} · resolved: {how} ",
                            hunk + 1,
                            editor.hunks.len()
                        ),
                        Color::Rgb(60, 140, 80),
                    )
                } else {
                    (
                        format!(
                            " ? Conflict {}/{} · unresolved ",
                            hunk + 1,
                            editor.hunks.len()
                        ),
                        Color::Rgb(170, 130, 30),
                    )
                };
                let width = (text.chars().count() as u16).min(rule.width);
                frame.render_widget(
                    Paragraph::new(text).style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(background)
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Rect::new(rule.x, y, width, 1),
                );
            }
        }
    }
    // Clear the cells the gutter buttons sit in, so neither a line's tail
    // nor its number runs under them.
    for hunk in 0..view.doc.hunks.len() {
        if let Some((ours, theirs)) = gutter_slots(area, view, hunk) {
            for slot in [ours, theirs] {
                frame.render_widget(
                    Paragraph::new(" ".repeat(slot.width as usize))
                        .style(Style::default().bg(Color::Rgb(28, 35, 49))),
                    slot,
                );
            }
        }
    }
    for button in buttons(area, view) {
        let Some(hunk) = button.hunk else {
            continue;
        };
        let hovered = editor.hovered == Some((hunk, button.action));
        let text = if button.framed {
            format!("[ {} ]", button.label)
        } else {
            button.label.to_string()
        };
        let style = if button.framed {
            button_style(hovered)
        } else if hovered {
            Style::default().fg(Color::Black).bg(Color::LightCyan)
        } else {
            Style::default()
                .fg(if button.action == MergeAction::Reset {
                    Color::LightRed
                } else {
                    Color::LightCyan
                })
                .add_modifier(Modifier::BOLD)
        };
        frame.render_widget(Paragraph::new(text).style(style), button.area);
    }
}
