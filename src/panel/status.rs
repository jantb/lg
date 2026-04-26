use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{state::AppState, ui};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let active = state.activity_label().is_some();
    let block = ui::framed_with_activity(1, "Status", focused, None, state.animation_tick, active);
    let inner = block.inner(area);

    // Line 1: branch
    let branch_line = match &state.branch {
        Some(b) => Line::from(vec![
            Span::styled("\u{2713} ", Style::default().fg(Color::Green)),
            Span::styled(b.as_str(), Style::default().fg(Color::Green)),
        ]),
        None => Line::from(Span::styled(
            "\u{2a2f} detached",
            Style::default().fg(Color::Red),
        )),
    };

    // Line 2: activity while work is running, otherwise ahead/behind.
    let meta_line = if let Some(label) = state.activity_label() {
        let spinner =
            crate::state::SPINNER_FRAMES[state.animation_tick % crate::state::SPINNER_FRAMES.len()];
        Line::from(vec![
            Span::styled(
                spinner,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(label, Style::default().fg(Color::Cyan)),
        ])
    } else {
        match state.ahead_behind {
            Some((a, b)) => Line::from(Span::raw(format!("\u{2191}{a} \u{2193}{b}"))),
            None => Line::from(Span::styled(
                "\u{2191}- \u{2193}-",
                Style::default().fg(Color::DarkGray),
            )),
        }
    };

    // Line 3: file count by change type.
    let (staged, unstaged, untracked) = state.file_counts();
    let n = state.files.len();
    let files_style = if staged + unstaged + untracked > 0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    };
    let files_line = Line::from(vec![
        Span::styled(format!("{n} files  "), files_style),
        Span::styled(format!("S{staged} "), Style::default().fg(Color::Green)),
        Span::styled(format!("U{unstaged} "), Style::default().fg(Color::Yellow)),
        Span::styled(format!("?{untracked}"), Style::default().fg(Color::Cyan)),
    ]);

    let para = Paragraph::new(vec![branch_line, meta_line, files_line]).block(block);
    frame.render_widget(para, area);
    let _ = inner; // inner used implicitly via block.inner above
}
