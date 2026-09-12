use super::app::{InputMode, Panel, TuiApp};
use ratatui::prelude::*;
use ratatui::widgets::*;

pub fn draw(frame: &mut Frame, app: &TuiApp) {
    let area = frame.area();

    // Vertical: main (fill) + help bar (3 lines)
    let [main_area, help_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).areas(area);

    // Horizontal: allowed (50%) | blocked (50%)
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(main_area);

    draw_allowed(frame, app, left_area);
    draw_blocked(frame, app, right_area);
    draw_help(frame, app, help_area);

    if let InputMode::AddDomain(ref input) = app.input_mode {
        draw_input_popup(frame, input, area);
    }
}

fn draw_allowed(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let active = app.panel == Panel::Allowed;
    let border_style = if active {
        Style::default().fg(Color::Green)
    } else {
        Style::default()
    };

    let items: Vec<ListItem> = app
        .allowed
        .iter()
        .map(|d| ListItem::new(d.as_str()))
        .collect();

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Allowed ")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if active && !app.allowed.is_empty() {
        state.select(Some(app.allowed_cursor));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_blocked(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let active = app.panel == Panel::Blocked;
    let border_style = if active {
        Style::default().fg(Color::Green)
    } else {
        Style::default()
    };

    // Split: list on top, detail area (3 lines) on bottom
    let selected = if active && !app.blocked.is_empty() {
        Some(app.blocked_cursor)
    } else {
        None
    };
    let has_detail = selected.is_some_and(|i| !app.blocked[i].last_url.is_empty());
    let [list_area, detail_area] = if has_detail {
        Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(0), Constraint::Length(0)]).areas(area)
    };

    let items: Vec<ListItem> = app
        .blocked
        .iter()
        .map(|e| ListItem::new(format!("{}  ({}x)", e.domain, e.count)))
        .collect();

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Blocked ")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if let Some(idx) = selected {
        state.select(Some(idx));
    }
    frame.render_stateful_widget(list, list_area, &mut state);

    if has_detail {
        let entry = &app.blocked[selected.unwrap()];
        let detail_text = format!("{} {}", entry.last_method, entry.last_url);
        let detail = Paragraph::new(detail_text)
            .block(
                Block::bordered()
                    .title(" Last Request ")
                    .border_style(border_style),
            )
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(detail, detail_area);
    }
}

fn draw_help(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let text = match app.input_mode {
        InputMode::Normal => {
            "  Tab/h/l: switch panel   j/k: navigate   Enter: move to other panel   \
             a: add domain   d/x: delete   q/Esc/Ctrl-C: quit"
        }
        InputMode::AddDomain(_) => "  Type domain name   Enter: confirm   Esc: cancel",
    };
    let paragraph = Paragraph::new(text)
        .block(Block::bordered().title(" Help "))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

fn draw_input_popup(frame: &mut Frame, input: &str, area: Rect) {
    // Center popup: 50% wide, 3 lines tall
    let popup_width = area.width / 2;
    let popup_height = 3u16;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    let paragraph = Paragraph::new(input)
        .block(
            Block::bordered()
                .title(" Add Domain ")
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, popup_area);

    // Show cursor inside the popup
    frame.set_cursor_position(Position::new(
        popup_area.x + 1 + input.len() as u16,
        popup_area.y + 1,
    ));
}
