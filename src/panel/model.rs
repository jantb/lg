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
    config::LLM_MODEL_CHOICES,
    state::{AppState, Modal, PendingAction, SettingsField, SettingsMode},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = 92.min(area.width);
    let h = 24.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);
    if modal.width < 40 || modal.height < 16 {
        frame.render_widget(
            Paragraph::new("Terminal too small for settings").block(ui::bordered("Settings")),
            modal,
        );
        return;
    }

    let mode = if crate::llm::env_model_active() || crate::llm::env_provider_active() {
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
            Span::styled("Repo:  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                state.settings_dir.clone(),
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
                crate::llm::endpoint_for_provider(state.llm_provider),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Active:", Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::styled(
                state.llm_model.clone(),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        field_line(state, SettingsField::Model, "Model", &state.llm_model_input),
        field_line(
            state,
            SettingsField::PrLanguage,
            "Language",
            &state.settings_pr_language_input,
        ),
        field_line(
            state,
            SettingsField::CommentStyle,
            "Message shape",
            &style_display(&state.settings_comment_style_input),
        ),
        field_line(
            state,
            SettingsField::SubjectMax,
            "Subject max",
            &limit_display(&state.settings_subject_max_input),
        ),
        field_line(
            state,
            SettingsField::BodyLines,
            "Body lines",
            &limit_display(&state.settings_body_lines_input),
        ),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Commit prompt ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if state.settings_prompt_is_custom {
                    "custom (Ctrl+E to edit)"
                } else {
                    "built-in default (Ctrl+E to edit)"
                },
                Style::default().fg(Color::Gray),
            ),
        ]),
        save_line(state),
        Line::from(""),
    ];

    let editing = state.settings_mode == SettingsMode::Edit;
    let choices = state.settings_field.choices();
    if editing && !choices.is_empty() {
        let selected_idx = choice_index(state);
        for (idx, choice) in choices.iter().enumerate() {
            let selected = Some(idx) == selected_idx;
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
                Span::styled(*choice, style),
            ]));
        }
    } else {
        lines.push(Line::from(vec![Span::styled(
            hint_for(state),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines.push(Line::from(""));

    let keys: &[(&str, Color, &str)] = if editing {
        &[
            ("Up/Down", Color::Yellow, "value"),
            ("type", Color::Yellow, "edit"),
            ("Enter", Color::Green, "confirm"),
            ("Esc", Color::Gray, "discard field"),
        ]
    } else {
        &[
            ("Up/Down", Color::Yellow, "select"),
            ("Enter", Color::Green, "open/save"),
            ("Ctrl+S", Color::Green, "save"),
            ("Ctrl+E", Color::Cyan, "prompt"),
            ("Ctrl+U", Color::Red, "reset"),
            ("Esc", Color::Gray, "cancel"),
        ]
    };
    lines.extend(key_hint_lines(keys, modal.width.saturating_sub(2)));

    frame.render_widget(Paragraph::new(lines).block(ui::bordered("Settings")), modal);
}

/// Lays the key hints out across as many lines as the modal is wide enough for,
/// so no binding is clipped off the right edge on a narrow terminal.
fn key_hint_lines(keys: &[(&str, Color, &str)], width: u16) -> Vec<Line<'static>> {
    const GAP: &str = "   ";
    let width = width.max(20) as usize;
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for (key, color, label) in keys {
        let entry = key.len() + 1 + label.len();
        let extra = if spans.is_empty() { 0 } else { GAP.len() };
        if !spans.is_empty() && used + extra + entry > width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }
        if !spans.is_empty() {
            spans.push(Span::raw(GAP));
            used += GAP.len();
        }
        spans.push(Span::styled(
            (*key).to_string(),
            Style::default().fg(*color),
        ));
        spans.push(Span::raw(format!(" {label}")));
        used += entry;
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

fn field_line(state: &AppState, field: SettingsField, label: &str, value: &str) -> Line<'static> {
    let focused = state.settings_field == field;
    let editing = focused && state.settings_mode == SettingsMode::Edit;
    let value_style = if focused {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Line::from(vec![
        Span::styled(
            if focused { "> " } else { "  " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(format!("{label:<14}"), Style::default().fg(Color::Yellow)),
        Span::styled(value.to_string(), value_style),
        Span::styled(
            if editing { "_" } else { "" },
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

fn save_line(state: &AppState) -> Line<'static> {
    let focused = state.settings_field == SettingsField::Save;
    let style = if focused {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Line::from(vec![
        Span::styled(
            if focused { "> " } else { "  " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("Save settings", style),
    ])
}

fn hint_for(state: &AppState) -> &'static str {
    match state.settings_field {
        SettingsField::Model => "Enter to pick from the list or type a value.",
        SettingsField::PrLanguage => {
            "Language for generated commit messages, PR text, and review prose."
        }
        SettingsField::CommentStyle => {
            "Format the project's commit messages follow, derived from history. Free text."
        }
        SettingsField::SubjectMax => "Max characters in the commit subject line. 0 means no limit.",
        SettingsField::BodyLines => "Max body lines after the blank line. 0 means no limit.",
        SettingsField::Save => "Enter writes these settings for this checkout.",
    }
}

/// Index of the current value within the focused row's choice list, if it is one
/// of them. A typed value that matches nothing leaves the list unmarked.
fn choice_index(state: &AppState) -> Option<usize> {
    let value = current_value(state);
    state
        .settings_field
        .choices()
        .iter()
        .position(|choice| choice.eq_ignore_ascii_case(value.trim()))
}

fn current_value(state: &AppState) -> &str {
    match state.settings_field {
        SettingsField::Model => &state.llm_model_input,
        SettingsField::PrLanguage => &state.settings_pr_language_input,
        SettingsField::CommentStyle => &state.settings_comment_style_input,
        SettingsField::SubjectMax => &state.settings_subject_max_input,
        SettingsField::BodyLines => &state.settings_body_lines_input,
        SettingsField::Save => "",
    }
}

fn value_mut(state: &mut AppState) -> Option<&mut String> {
    match state.settings_field {
        SettingsField::Model => Some(&mut state.llm_model_input),
        SettingsField::PrLanguage => Some(&mut state.settings_pr_language_input),
        SettingsField::CommentStyle => Some(&mut state.settings_comment_style_input),
        SettingsField::SubjectMax => Some(&mut state.settings_subject_max_input),
        SettingsField::BodyLines => Some(&mut state.settings_body_lines_input),
        SettingsField::Save => None,
    }
}

/// A blank house style is shown as such, so an empty row does not look broken.
fn style_display(value: &str) -> String {
    if value.trim().is_empty() {
        "(none)".to_string()
    } else {
        value.to_string()
    }
}

/// Empty input reads as unlimited, which is clearer spelled out than as a blank.
fn limit_display(value: &str) -> String {
    if value.is_empty() || value == "0" {
        "0 (unlimited)".to_string()
    } else {
        value.to_string()
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('e') if ctrl => {
            state.pending_action = Some(PendingAction::EditCommitPrompt);
        }
        KeyCode::Char('u') if ctrl => {
            state.pending_action = Some(PendingAction::ClearSettings);
        }
        KeyCode::Char('s') if ctrl => save(state),
        _ if state.settings_mode == SettingsMode::Edit => edit_key(state, key, ctrl),
        _ => browse_key(state, key),
    }
    Ok(())
}

fn browse_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Tab | KeyCode::Down => state.settings_field = state.settings_field.next(true),
        KeyCode::BackTab | KeyCode::Up => state.settings_field = state.settings_field.next(false),
        KeyCode::Enter => {
            if state.settings_field == SettingsField::Save {
                save(state);
            } else {
                begin_edit(state);
            }
        }
        _ => {}
    }
}

fn edit_key(state: &mut AppState, key: KeyEvent, ctrl: bool) {
    match key.code {
        KeyCode::Enter => state.settings_mode = SettingsMode::Browse,
        KeyCode::Esc => {
            let backup = state.settings_edit_backup.clone();
            if let Some(value) = value_mut(state) {
                *value = backup;
            }
            sync_selection_to_input(state);
            state.settings_mode = SettingsMode::Browse;
        }
        KeyCode::Tab => {
            state.settings_mode = SettingsMode::Browse;
            state.settings_field = state.settings_field.next(true);
        }
        KeyCode::BackTab => {
            state.settings_mode = SettingsMode::Browse;
            state.settings_field = state.settings_field.next(false);
        }
        KeyCode::Up => step_value(state, false),
        KeyCode::Down => step_value(state, true),
        KeyCode::Backspace if !ctrl => {
            if let Some(value) = value_mut(state) {
                value.pop();
            }
            sync_selection_to_input(state);
        }
        KeyCode::Char(c) if !ctrl => match state.settings_field {
            SettingsField::Model => {
                state.llm_model_input.push(c);
                sync_selection_to_input(state);
            }
            SettingsField::PrLanguage => state.settings_pr_language_input.push(c),
            SettingsField::CommentStyle => state.settings_comment_style_input.push(c),
            SettingsField::SubjectMax if c.is_ascii_digit() => {
                push_limit_digit(&mut state.settings_subject_max_input, c);
            }
            SettingsField::BodyLines if c.is_ascii_digit() => {
                push_limit_digit(&mut state.settings_body_lines_input, c);
            }
            _ => {}
        },
        _ => {}
    }
}

fn begin_edit(state: &mut AppState) {
    state.settings_edit_backup = current_value(state).to_string();
    state.settings_mode = SettingsMode::Edit;
}

fn save(state: &mut AppState) {
    state.settings_mode = SettingsMode::Browse;
    state.pending_action = Some(PendingAction::SaveSettings {
        model: state.llm_model_input.clone(),
        provider: state.llm_provider,
        pr_language: state.settings_pr_language_input.clone(),
        comment_style: state.settings_comment_style_input.clone(),
        commit_subject_max_chars: state.settings_subject_max_input.clone(),
        commit_body_max_lines: state.settings_body_lines_input.clone(),
    });
}

/// Up/Down inside a row walks its choice list, or nudges a numeric limit.
fn step_value(state: &mut AppState, next: bool) {
    let choices = state.settings_field.choices();
    if !choices.is_empty() {
        let idx = match choice_index(state) {
            Some(idx) if next => (idx + 1) % choices.len(),
            Some(idx) => (idx + choices.len() - 1) % choices.len(),
            None => 0,
        };
        let picked = choices[idx].to_string();
        if let Some(value) = value_mut(state) {
            *value = picked;
        }
        sync_selection_to_input(state);
        return;
    }
    // Up raises a number and Down lowers it, matching how a spinner reads.
    if let Some(value) = value_mut(state) {
        let current: usize = value.parse().unwrap_or(0);
        let stepped = if next {
            current.saturating_sub(1)
        } else {
            (current + 1).min(9999)
        };
        *value = stepped.to_string();
    }
}

/// Limits are at most four digits; a longer number is never a real cap and only
/// overflows the field.
fn push_limit_digit(input: &mut String, digit: char) {
    if input == "0" {
        input.clear();
    }
    if input.len() < 4 {
        input.push(digit);
    }
}

fn sync_selection_to_input(state: &mut AppState) {
    if let Some(idx) = LLM_MODEL_CHOICES
        .iter()
        .position(|model| *model == state.llm_model_input)
    {
        state.llm_model_idx = idx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, ctrl: bool) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: if ctrl {
                KeyModifiers::CONTROL
            } else {
                KeyModifiers::NONE
            },
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn state_with_fields() -> AppState {
        let mut state = AppState::new();
        state.settings_pr_language_input = "English".to_string();
        state.settings_subject_max_input = "72".to_string();
        state.settings_body_lines_input = "8".to_string();
        state
    }

    fn focus(state: &mut AppState, field: SettingsField) {
        state.settings_field = field;
        state.settings_mode = SettingsMode::Browse;
    }

    #[test]
    fn tab_cycles_through_every_row() {
        let mut state = state_with_fields();
        for expected in [
            SettingsField::PrLanguage,
            SettingsField::CommentStyle,
            SettingsField::SubjectMax,
            SettingsField::BodyLines,
            SettingsField::Save,
            SettingsField::Model,
        ] {
            handle_key(&mut state, key(KeyCode::Tab, false)).unwrap();
            assert_eq!(state.settings_field, expected);
        }
    }

    #[test]
    fn arrow_keys_move_between_rows_while_browsing() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Down, false)).unwrap();
        assert_eq!(state.settings_field, SettingsField::CommentStyle);
        handle_key(&mut state, key(KeyCode::Up, false)).unwrap();
        assert_eq!(state.settings_field, SettingsField::PrLanguage);
    }

    #[test]
    fn enter_opens_a_row_for_editing_and_confirms_it() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        assert_eq!(state.settings_mode, SettingsMode::Edit);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        assert_eq!(state.settings_mode, SettingsMode::Browse);
        assert!(state.pending_action.is_none());
    }

    #[test]
    fn browsing_ignores_typed_characters() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Char('x'), false)).unwrap();
        assert_eq!(state.settings_pr_language_input, "English");
    }

    #[test]
    fn up_down_picks_a_language_while_editing() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        handle_key(&mut state, key(KeyCode::Down, false)).unwrap();
        assert_eq!(state.settings_pr_language_input, "Norwegian");
    }

    #[test]
    fn typing_edits_only_the_focused_field() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        state.settings_pr_language_input.clear();
        for c in "Norsk".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), false)).unwrap();
        }
        assert_eq!(state.settings_pr_language_input, "Norsk");
        assert_eq!(state.settings_subject_max_input, "72");
    }

    #[test]
    fn esc_restores_the_value_the_row_had_before_editing() {
        let mut state = state_with_fields();
        state.modal = Modal::Model;
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        handle_key(&mut state, key(KeyCode::Down, false)).unwrap();
        handle_key(&mut state, key(KeyCode::Esc, false)).unwrap();
        assert_eq!(state.settings_pr_language_input, "English");
        assert_eq!(state.settings_mode, SettingsMode::Browse);
        assert_eq!(state.modal, Modal::Model);
    }

    #[test]
    fn esc_while_browsing_closes_the_modal() {
        let mut state = state_with_fields();
        state.modal = Modal::Model;
        focus(&mut state, SettingsField::PrLanguage);
        handle_key(&mut state, key(KeyCode::Esc, false)).unwrap();
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn limit_fields_accept_digits_only() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::SubjectMax);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        state.settings_subject_max_input.clear();
        for c in "5a0".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), false)).unwrap();
        }
        assert_eq!(state.settings_subject_max_input, "50");
    }

    #[test]
    fn up_down_nudges_a_numeric_limit_while_editing() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::BodyLines);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        handle_key(&mut state, key(KeyCode::Up, false)).unwrap();
        assert_eq!(state.settings_body_lines_input, "9");
        handle_key(&mut state, key(KeyCode::Down, false)).unwrap();
        handle_key(&mut state, key(KeyCode::Down, false)).unwrap();
        assert_eq!(state.settings_body_lines_input, "7");
    }

    #[test]
    fn a_leading_zero_is_replaced_by_the_next_digit() {
        let mut input = "0".to_string();
        push_limit_digit(&mut input, '4');
        assert_eq!(input, "4");
    }

    #[test]
    fn limit_digits_are_capped_in_length() {
        let mut input = "1234".to_string();
        push_limit_digit(&mut input, '5');
        assert_eq!(input, "1234");
    }

    #[test]
    fn enter_on_the_save_row_saves_both_llm_and_repo_settings() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::Save);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();

        assert!(matches!(
            &state.pending_action,
            Some(PendingAction::SaveSettings { pr_language, .. }) if pr_language == "English"
        ));
    }

    #[test]
    fn ctrl_s_saves_from_any_row() {
        let mut state = state_with_fields();
        focus(&mut state, SettingsField::Model);
        handle_key(&mut state, key(KeyCode::Enter, false)).unwrap();
        handle_key(&mut state, key(KeyCode::Char('s'), true)).unwrap();
        assert_eq!(state.settings_mode, SettingsMode::Browse);
        assert!(matches!(
            state.pending_action,
            Some(PendingAction::SaveSettings { .. })
        ));
    }

    #[test]
    fn ctrl_e_opens_the_commit_prompt_for_editing() {
        let mut state = state_with_fields();
        handle_key(&mut state, key(KeyCode::Char('e'), true)).unwrap();
        assert!(matches!(
            state.pending_action,
            Some(PendingAction::EditCommitPrompt)
        ));
    }

    #[test]
    fn ctrl_u_resets_llm_and_repo_settings() {
        let mut state = state_with_fields();
        handle_key(&mut state, key(KeyCode::Char('u'), true)).unwrap();
        assert!(matches!(
            state.pending_action,
            Some(PendingAction::ClearSettings)
        ));
    }

    #[test]
    fn key_hints_wrap_instead_of_clipping() {
        let keys: &[(&str, Color, &str)] = &[
            ("Up/Down", Color::Yellow, "select"),
            ("Enter", Color::Green, "open/save"),
            ("Ctrl+U", Color::Red, "reset"),
        ];
        let lines = key_hint_lines(keys, 30);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(line.width() <= 30);
        }
    }

    #[test]
    fn unlimited_limits_read_as_unlimited() {
        assert_eq!(limit_display("0"), "0 (unlimited)");
        assert_eq!(limit_display(""), "0 (unlimited)");
        assert_eq!(limit_display("72"), "72");
    }
}
