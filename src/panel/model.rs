use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{
    config::OLLAMA_MODEL_CHOICES,
    ollama::LlmProvider,
    state::{AppState, Modal, PendingAction},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = 84.min(area.width);
    let h = 16.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);
    if modal.width < 32 || modal.height < 12 {
        frame.render_widget(
            Paragraph::new("Terminal too small for LLM settings").block(ui::bordered("LLM")),
            modal,
        );
        return;
    }

    let mode = if crate::ollama::env_model_active() || crate::ollama::env_provider_active() {
        "env override"
    } else {
        "saved/default"
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Mode:  ", Style::default().fg(Color::Yellow)),
            Span::styled(mode, Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("Store: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                state.llm_config_path.clone(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Provider ", Style::default().fg(Color::Yellow)),
            Span::styled(
                state.llm_provider.label(),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled("Endpoint ", Style::default().fg(Color::Yellow)),
            Span::styled(
                crate::ollama::endpoint_for_provider(state.llm_provider),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Active:", Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::styled(
                state.ollama_model.clone(),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Model ", Style::default().fg(Color::Yellow)),
            Span::styled(
                state.ollama_model_input.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];

    for (idx, model) in OLLAMA_MODEL_CHOICES.iter().enumerate() {
        let selected = idx == state.ollama_model_idx;
        let marker = if selected { ">" } else { " " };
        let style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), style),
            Span::styled(*model, style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
        Span::raw(" pick    "),
        Span::styled("p", Style::default().fg(Color::Yellow)),
        Span::raw(" provider    "),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::raw(" save    "),
        Span::styled("Ctrl+U", Style::default().fg(Color::Red)),
        Span::raw(" clear saved    "),
        Span::styled("Esc", Style::default().fg(Color::Gray)),
        Span::raw(" cancel"),
    ]));

    frame.render_widget(
        Paragraph::new(lines).block(ui::bordered("LLM Settings")),
        modal,
    );
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Up => pick_model(state, false),
        KeyCode::Tab | KeyCode::Down => pick_model(state, true),
        KeyCode::Char('p') if !ctrl => pick_provider(state),
        KeyCode::Enter => {
            state.pending_action = Some(PendingAction::SaveLlmSettings {
                model: state.ollama_model_input.clone(),
                provider: state.llm_provider,
            });
        }
        KeyCode::Char('u') if ctrl => {
            state.pending_action = Some(PendingAction::ClearLlmSettings);
        }
        KeyCode::Backspace if !ctrl => {
            state.ollama_model_input.pop();
            sync_selection_to_input(state);
        }
        KeyCode::Char(c) if !ctrl => {
            state.ollama_model_input.push(c);
            sync_selection_to_input(state);
        }
        _ => {}
    }
    Ok(())
}

fn pick_provider(state: &mut AppState) {
    state.llm_provider_idx = (state.llm_provider_idx + 1) % LlmProvider::ALL.len();
    state.llm_provider = LlmProvider::ALL[state.llm_provider_idx];
}

fn pick_model(state: &mut AppState, next: bool) {
    if OLLAMA_MODEL_CHOICES.is_empty() {
        return;
    }
    state.ollama_model_idx = if next {
        (state.ollama_model_idx + 1) % OLLAMA_MODEL_CHOICES.len()
    } else {
        state
            .ollama_model_idx
            .checked_sub(1)
            .unwrap_or(OLLAMA_MODEL_CHOICES.len() - 1)
    };
    state.ollama_model_input = OLLAMA_MODEL_CHOICES[state.ollama_model_idx].to_string();
}

fn sync_selection_to_input(state: &mut AppState) {
    if let Some(idx) = OLLAMA_MODEL_CHOICES
        .iter()
        .position(|model| *model == state.ollama_model_input)
    {
        state.ollama_model_idx = idx;
    }
}
