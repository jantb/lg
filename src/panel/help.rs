use anyhow::Result;
use ratatui::crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::{
    config::BORDER_COLOR,
    state::{AppState, Modal, Pane},
    ui::centered,
};

struct Section {
    title: &'static str,
    pane: Option<Pane>,
    bindings: &'static [(&'static str, &'static str)],
}

const SECTIONS: &[Section] = &[
    Section {
        title: "Global",
        pane: None,
        bindings: &[
            ("?", "Toggle help"),
            ("Ctrl-C / q", "Quit"),
            ("1/2/3/4", "Focus Status/Files/Branches/Commits"),
            ("0", "Focus Diff"),
            ("Tab/Shift-Tab", "Cycle focus"),
            ("a", "Edit author settings"),
            ("c", "Open commit modal"),
            ("f", "Fetch remote updates"),
            ("p", "Pull current branch when behind"),
            ("P", "Push current branch"),
            ("R", "Build assisted review against main"),
            ("click pane", "Focus pane"),
            ("drag divider", "Resize columns or rows"),
        ],
    },
    Section {
        title: "Files",
        pane: Some(Pane::Files),
        bindings: &[
            ("j/k", "Move up/down"),
            ("space / y", "Stage selected"),
            ("u", "Unstage selected"),
            ("A / U", "Stage all / unstage all"),
            ("o", "Open file in IntelliJ/RustRover"),
            ("Enter", "Refresh diff"),
        ],
    },
    Section {
        title: "Branches",
        pane: Some(Pane::Branches),
        bindings: &[
            ("j/k", "Move up/down"),
            ("Enter", "Checkout branch"),
            ("select", "Commit log follows branch"),
        ],
    },
    Section {
        title: "Commits",
        pane: Some(Pane::Commits),
        bindings: &[
            ("j/k", "Move up/down (auto-diff)"),
            ("Enter", "Focus diff pane"),
        ],
    },
    Section {
        title: "Diff pane",
        pane: Some(Pane::Main),
        bindings: &[
            ("j/k", "Scroll line"),
            ("Ctrl-d/Ctrl-u", "Scroll half page"),
            ("g / G", "Top / bottom"),
            ("R", "Build assisted review against main"),
            ("o", "Open current source file in IDE"),
            ("wheel", "Scroll 3 lines (mouse)"),
            ("Shift+drag", "Select text (terminal native)"),
        ],
    },
    Section {
        title: "Review",
        pane: Some(Pane::Main),
        bindings: &[
            ("j/k", "Move selected review item"),
            ("Enter / space", "Expand or collapse selected item"),
            ("d", "Drill into first child item"),
            ("s", "Toggle full source with inline diff"),
            ("o", "Open selected source file in IDE"),
            ("l", "Ask Ollama to explain selected subtree"),
            ("g / G", "Top / bottom"),
            ("R", "Rebuild assisted review"),
        ],
    },
    Section {
        title: "Commit modal",
        pane: None,
        bindings: &[
            ("Ctrl+S", "Commit"),
            ("Enter", "New line"),
            ("Ctrl+R", "Regenerate message"),
            ("Backspace", "Delete char"),
            ("Esc", "Cancel"),
        ],
    },
    Section {
        title: "Author settings",
        pane: None,
        bindings: &[
            ("Tab / arrows", "Switch field"),
            ("Enter", "Save subtree rule"),
            ("Ctrl+L", "Save repo-local author"),
            ("Ctrl+U", "Clear subtree rule"),
            ("Ctrl+X", "Clear repo-local author"),
            ("Esc", "Cancel"),
        ],
    },
    Section {
        title: "Push modal",
        pane: None,
        bindings: &[("Enter", "Push to origin"), ("Esc", "Cancel")],
    },
];

fn pane_name(p: Pane) -> &'static str {
    match p {
        Pane::Status => "Status",
        Pane::Files => "Files",
        Pane::Branches => "Branches",
        Pane::Commits => "Commits",
        Pane::Main => "Diff",
    }
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    // Compute height: 1 heading + bindings count + 1 blank per section (except last)
    let total_lines: u16 = SECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let blank = if i + 1 < SECTIONS.len() { 1u16 } else { 0u16 };
            1 + s.bindings.len() as u16 + blank
        })
        .sum::<u16>()
        + 2; // border lines

    let height = total_lines.min(area.height.saturating_sub(2));
    let overlay = centered(area, 64, height);

    frame.render_widget(Clear, overlay);

    let mut lines: Vec<Line> = Vec::new();
    for (i, section) in SECTIONS.iter().enumerate() {
        let is_active = section.pane == Some(state.prev_focus);
        let heading_style = if is_active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let prefix = if is_active { "\u{25b6} " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{prefix}{}", section.title),
            heading_style,
        )));
        for (key, desc) in section.bindings {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<14}", key), Style::default().fg(Color::Yellow)),
                Span::raw(*desc),
            ]));
        }
        if i + 1 < SECTIONS.len() {
            lines.push(Line::from(""));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(format!("Help \u{2014} {}", pane_name(state.prev_focus)))
        .title_bottom(
            Line::from(Span::styled(
                "any key to close",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Right),
        );

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, overlay);
}

pub fn handle_key(state: &mut AppState, _key: KeyEvent) -> Result<()> {
    state.modal = Modal::None;
    Ok(())
}
