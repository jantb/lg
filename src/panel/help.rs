use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
            ("Esc", "Dismiss an error, or cancel running LLM work"),
            ("Ctrl-C / q", "Quit"),
            ("1/2/3/4", "Focus Status/Files/Branches/Commits"),
            ("0", "Focus Diff"),
            ("Tab/Shift-Tab", "Cycle focus"),
            ("a", "Edit author settings"),
            ("c", "Open commit modal"),
            ("f", "Fetch remote updates"),
            ("L", "Choose LLM model"),
            ("p", "Pull current branch when behind"),
            ("P", "Push current branch"),
            ("R", "Enter review mode against main"),
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
            ("r", "Roll back selected file or folder (confirms first)"),
            ("i", "Add selected file or folder to .gitignore"),
            ("d", "Delete selected file or folder (confirms first)"),
            ("o", "Open file or project in IntelliJ/RustRover"),
            ("Enter", "Refresh diff"),
        ],
    },
    Section {
        title: "Nested Repos",
        pane: Some(Pane::Status),
        bindings: &[
            ("j/k", "Move up/down"),
            ("Enter", "Expand repo, collapse repo, or checkout branch"),
            ("o", "Open selected repository in editor"),
            ("r", "Toggle local/remote branches"),
            ("Esc/Backspace", "Collapse expanded repository"),
        ],
    },
    Section {
        title: "Branches",
        pane: Some(Pane::Branches),
        bindings: &[
            ("j/k", "Move up/down"),
            ("Enter", "Checkout branch or remote tracking branch"),
            ("o", "Open repository in editor"),
            ("u", "Set upstream to matching remote branch"),
            ("r", "Toggle local and remote branch views"),
            ("m", "Pull main or merge origin/main"),
            ("M", "Merge main into all branches and push"),
            ("d", "Delete selected local branch with no upstream"),
            ("D", "Delete branch with local/remote/force options"),
            ("F", "Branch action menu"),
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
        title: "Review mode",
        pane: Some(Pane::Main),
        bindings: &[
            ("j/k", "Move selected review item"),
            ("Enter / s", "Toggle source for selected item"),
            ("space", "Expand or collapse selected item"),
            ("d", "Drill into first child item"),
            ("n / N", "Jump next or previous inline review note"),
            ("o", "Open selected source file in IDE"),
            ("f", "Run style flag pass"),
            ("l", "Explain or generate PR text"),
            ("y", "Copy LLM/PR text"),
            ("C", "Chat with LLM about the full review"),
            ("g / G", "Top / bottom"),
            ("v", "Toggle unified or side-by-side diff"),
            ("R", "Rebuild assisted review"),
            ("Esc", "Cancel running LLM work"),
        ],
    },
    Section {
        title: "Diff pane",
        pane: Some(Pane::Main),
        bindings: &[
            ("j/k", "Scroll line"),
            ("Ctrl-d/Ctrl-u", "Scroll half page"),
            ("g / G", "Top / bottom"),
            ("R", "Enter review mode against main"),
            ("v", "Toggle unified or side-by-side diff"),
            ("o", "Open current source file in IDE"),
            ("wheel", "Scroll 3 lines (mouse)"),
            ("Shift+drag", "Select text (terminal native)"),
        ],
    },
    Section {
        title: "Review chat",
        pane: None,
        bindings: &[
            ("Enter", "Send prompt"),
            ("Esc", "Cancel the running answer, then close chat"),
            ("Ctrl+A / Ctrl+E", "Start / end of prompt"),
            ("Up / Down", "Scroll conversation"),
        ],
    },
    Section {
        title: "Commit modal",
        pane: None,
        bindings: &[
            ("Ctrl+S", "Commit"),
            ("Ctrl+P", "Commit and push"),
            ("Enter", "New line"),
            ("Ctrl+R", "Regenerate message"),
            ("Ctrl+U", "Clear message"),
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
        title: "Model settings",
        pane: None,
        bindings: &[
            ("Up / Down", "Pick known model"),
            ("p", "Cycle provider"),
            ("Enter", "Save selected or typed model"),
            ("Ctrl+U", "Clear saved LLM settings"),
            ("Esc", "Cancel"),
        ],
    },
    Section {
        title: "Confirm prompts",
        pane: None,
        bindings: &[
            ("y", "Confirm the destructive action"),
            ("n / Esc", "Cancel"),
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

/// Body lines the help text occupies, excluding the modal borders.
fn content_lines() -> u16 {
    SECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let blank = if i + 1 < SECTIONS.len() { 1u16 } else { 0u16 };
            1 + s.bindings.len() as u16 + blank
        })
        .sum()
}

/// Body height of the help overlay for `area`, i.e. how many lines are visible at once.
fn viewport_height(area: Rect) -> u16 {
    let overlay_height = content_lines()
        .saturating_add(2)
        .min(area.height.saturating_sub(2))
        .max(3.min(area.height));
    overlay_height.saturating_sub(2)
}

/// Largest scroll offset that still shows content in the last row.
pub fn max_offset(area: Rect) -> u16 {
    content_lines().saturating_sub(viewport_height(area))
}

pub fn scroll(state: &mut AppState, area: Rect, down: bool, amount: u16) {
    let max = max_offset(area);
    state.help_offset = if down {
        state.help_offset.saturating_add(amount).min(max)
    } else {
        state.help_offset.saturating_sub(amount)
    };
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let height = content_lines()
        .saturating_add(2)
        .min(area.height.saturating_sub(2))
        .max(3.min(area.height));
    let overlay = centered(area, 64, height);
    let offset = state.help_offset.min(max_offset(area));

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
                if max_offset(area) > 0 {
                    format!(
                        "j/k scroll \u{2022} {}%  \u{2022} q/Esc close",
                        scroll_percent(offset, max_offset(area))
                    )
                } else {
                    "q/Esc close".to_owned()
                },
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Right),
        );

    let para = Paragraph::new(lines).block(block).scroll((offset, 0));
    frame.render_widget(para, overlay);
}

fn scroll_percent(offset: u16, max: u16) -> u16 {
    if max == 0 {
        100
    } else {
        (u32::from(offset) * 100 / u32::from(max)) as u16
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent, area: Rect) -> Result<()> {
    let page = viewport_height(area).saturating_sub(1).max(1);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => scroll(state, area, true, 1),
        KeyCode::Char('k') | KeyCode::Up => scroll(state, area, false, 1),
        KeyCode::PageDown | KeyCode::Char(' ') => scroll(state, area, true, page),
        KeyCode::PageUp => scroll(state, area, false, page),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, area, true, page / 2)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, area, false, page / 2)
        }
        KeyCode::Char('g') => state.help_offset = 0,
        KeyCode::Char('G') => state.help_offset = max_offset(area),
        _ => {
            state.modal = Modal::None;
            state.help_offset = 0;
        }
    }
    Ok(())
}
