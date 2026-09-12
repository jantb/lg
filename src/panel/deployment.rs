//! Branch inclusion and explicit, reviewable environment promotions.
use crate::{
    state::{AppState, Modal, PendingAction},
    ui,
};
use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Paragraph, Wrap},
};
#[derive(Debug, Default)]
pub struct Environments {
    pub selected: usize,
    pub preview: Option<crate::git::environments::PromotionPreview>,
    pub notice: String,
}
pub fn open(state: &mut AppState) {
    state.environment_view = Environments::default();
    state.modal = Modal::Environments;
}
pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = ui::centered(
        area,
        area.width.saturating_sub(6).min(110),
        area.height.saturating_sub(4),
    );
    let inner = ui::modal_frame(
        frame,
        modal,
        "Environments — branch inclusion and promotion",
    );
    let config = crate::preferences::load().config.branches;
    let mut text = format!(
        "Integration: {} · Remote: {}\nSource: {}\n\n",
        config.base,
        config.remote,
        state.branch.as_deref().unwrap_or("detached HEAD")
    );
    if let Some(p) = &state.environment_view.preview {
        text.push_str(&format!(
            "{} ({}) → {}/{} ({})\nMethod: {} · Push: {}\n\n",
            p.source,
            &p.source_oid[..8],
            p.environment.remote,
            p.environment.branch,
            &p.target_oid[..8],
            p.rule.strategy,
            p.rule.push
        ));
        for c in p
            .commits
            .iter()
            .take(inner.height.saturating_sub(10) as usize)
        {
            text.push_str(c);
            text.push('\n');
        }
        text.push_str(&format!("\n{} commits in preview. Enter applies; Esc returns. Refs are checked again before merging.\n", p.commits.len()));
    } else {
        for (i, e) in config.environments.iter().enumerate() {
            let inclusion = state
                .current_branch_releases
                .environments
                .get(&e.id)
                .map(|s| {
                    if s.missing_commits == 0 {
                        "included in branch".into()
                    } else {
                        format!("{} commits missing", s.missing_commits)
                    }
                })
                .unwrap_or_else(|| "inclusion unknown".into());
            text.push_str(&format!(
                "{} {} · {}/{} · {} · deployment unknown\n",
                if i == state.environment_view.selected {
                    "›"
                } else {
                    " "
                },
                e.name,
                e.remote,
                if e.branch.is_empty() {
                    "(unmapped)"
                } else {
                    &e.branch
                },
                inclusion
            ));
            if !e.url.is_empty() {
                text.push_str(&format!("    {}\n", e.url));
            }
        }
        if config.environments.is_empty() {
            text.push_str("No environments configured. Press , to add mappings.\n");
        }
        text.push_str("\nj/k select · Enter preview promotion · a assign selected branch · , configure · Esc close\n");
    }
    text.push_str(&state.environment_view.notice);
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let config = crate::preferences::load().config.branches;
    match key.code {
        KeyCode::Esc => {
            if state.environment_view.preview.take().is_none() {
                state.modal = Modal::None;
            }
        }
        KeyCode::Char(',') => super::settings::open(state, 5),
        KeyCode::Down | KeyCode::Char('j') if state.environment_view.preview.is_none() => {
            state.environment_view.selected = (state.environment_view.selected + 1)
                .min(config.environments.len().saturating_sub(1))
        }
        KeyCode::Up | KeyCode::Char('k') if state.environment_view.preview.is_none() => {
            state.environment_view.selected = state.environment_view.selected.saturating_sub(1)
        }
        KeyCode::Char('a') => {
            let selected = state
                .branches
                .get(state.branches_idx)
                .map(|b| b.name.clone())
                .or_else(|| state.branch.clone());
            if let Some(branch) = selected {
                let index = state.environment_view.selected;
                super::settings::open(state, 5);
                if let Some(e) = state.settings_hub.draft["environments"].get_mut(index) {
                    e["branch"] = branch.into();
                }
                state.settings_hub.notice =
                    "Assignment is staged. Ctrl-S saves the mapping without moving any branch."
                        .into();
            }
        }
        KeyCode::Enter => {
            if let Some(preview) = state.environment_view.preview.take() {
                state.pending_action = Some(PendingAction::Promote(preview));
                state.modal = Modal::None;
            } else if let (Some(source), Some(env)) = (
                &state.branch,
                config.environments.get(state.environment_view.selected),
            ) {
                match crate::git::environments::preview_promotion(source, &env.id) {
                    Ok(p) => state.environment_view.preview = Some(p),
                    Err(e) => state.environment_view.notice = format!("{e:#}"),
                }
            }
        }
        _ => {}
    }
    Ok(())
}
