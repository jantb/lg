use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{state::AppState, ui};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let block = ui::framed(1, "Status", focused, None);
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

    // Line 2: ahead/behind
    let ab_line = match state.ahead_behind {
        Some((a, b)) => Line::from(Span::raw(format!("\u{2191}{a} \u{2193}{b}"))),
        None => Line::from(Span::styled(
            "\u{2191}- \u{2193}-",
            Style::default().fg(Color::DarkGray),
        )),
    };

    // Line 3: file count
    let n = state.files.len();
    let files_style = if n > 0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    };
    let files_line = Line::from(Span::styled(format!("{n} files"), files_style));

    let para = Paragraph::new(vec![branch_line, ab_line, files_line]).block(block);
    frame.render_widget(para, area);
    let _ = inner; // inner used implicitly via block.inner above
}
