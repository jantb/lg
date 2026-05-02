use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn highlight_log_line(line: &str) -> Line<'_> {
    let (graph, content) = split_log_graph_prefix(line);
    let mut spans = log_graph_spans(graph);
    spans.extend(log_content_spans(content));
    Line::from(spans)
}

fn split_log_graph_prefix(line: &str) -> (&str, &str) {
    let split_at = line
        .char_indices()
        .find_map(|(idx, ch)| (!is_log_graph_char(ch)).then_some(idx))
        .unwrap_or(line.len());
    line.split_at(split_at)
}

fn is_log_graph_char(ch: char) -> bool {
    matches!(ch, ' ' | '*' | '|' | '/' | '\\' | '_' | '-')
}

fn log_graph_spans(graph: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (col, ch) in graph.chars().enumerate() {
        let color = if ch == '*' {
            Color::Yellow
        } else if ch == ' ' {
            Color::DarkGray
        } else {
            log_graph_color(col)
        };
        let style = if ch == '*' {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans
}

fn log_content_spans(content: &str) -> Vec<Span<'static>> {
    if let Some(rest) = content.strip_prefix("commit ") {
        let mut spans = vec![Span::styled(
            "commit ".to_string(),
            Style::default().fg(Color::DarkGray),
        )];
        if let Some((sha, decoration)) = rest.split_once(' ') {
            spans.push(Span::styled(
                sha.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                decoration.to_string(),
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        return spans;
    }

    if let Some(rest) = content.strip_prefix("Merge:") {
        return vec![
            Span::styled(
                "Merge:".to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::LightBlue)),
        ];
    }
    if let Some(rest) = content.strip_prefix("Author:") {
        return vec![
            Span::styled(
                "Author:".to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::Gray)),
        ];
    }
    if let Some(rest) = content.strip_prefix("Date:") {
        return vec![
            Span::styled(
                "Date:".to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::Gray)),
        ];
    }
    vec![Span::styled(content.to_string(), Style::default())]
}

fn log_graph_color(col: usize) -> Color {
    const COLORS: &[Color] = &[
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::LightGreen,
        Color::Yellow,
        Color::Cyan,
        Color::Magenta,
        Color::Green,
    ];
    COLORS[col % COLORS.len()]
}
