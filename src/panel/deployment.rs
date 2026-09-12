//! Environments: where each branch deploys, drawn as the pipeline a change
//! travels, and the explicit, reviewable promotions that move it along.
//!
//! The modal answers three questions at a glance: which branch am I on, which
//! environments can it be promoted into and by what rule, and how far behind
//! each of them is. The picture on the left carries the first two — a marker
//! walks the route a promotion would take — and the pane on the right spells
//! out the third for whichever environment is selected.
use crate::state::{AppState, Modal, PendingAction};
use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

mod draw;
mod pipeline;

pub use draw::{render, render_with};

#[derive(Debug, Default)]
pub struct Environments {
    pub selected: usize,
    pub preview: Option<crate::git::environments::PromotionPreview>,
    pub notice: String,
    /// Whether the notice reports a failure rather than a hint.
    pub notice_error: bool,
}
pub fn open(state: &mut AppState) {
    state.environment_view = Environments::default();
    state.modal = Modal::Environments;
}

fn select(state: &mut AppState, index: usize, count: usize) {
    state.environment_view.selected = index.min(count.saturating_sub(1));
    state.environment_view.notice.clear();
    state.environment_view.notice_error = false;
}

/// Previews the promotion into the selected environment, or applies the
/// preview already on screen.
fn confirm(state: &mut AppState, config: &crate::preferences::Branches) {
    if let Some(preview) = state.environment_view.preview.take() {
        state.pending_action = Some(PendingAction::Promote(preview));
        state.modal = Modal::None;
    } else if let (Some(source), Some(env)) = (
        &state.branch,
        config.environments.get(state.environment_view.selected),
    ) {
        match crate::git::environments::preview_promotion(source, &env.id) {
            Ok(p) => {
                state.environment_view.preview = Some(p);
                state.environment_view.notice.clear();
                state.environment_view.notice_error = false;
            }
            Err(e) => {
                state.environment_view.notice = format!("{e:#}");
                state.environment_view.notice_error = true;
            }
        }
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let config = crate::preferences::load().config.branches;
    let count = config.environments.len();
    let choosing = state.environment_view.preview.is_none();
    match key.code {
        KeyCode::Esc => {
            if state.environment_view.preview.take().is_none() {
                state.modal = Modal::None;
            }
        }
        KeyCode::Char(',') => super::settings::open(state, 5),
        KeyCode::Down | KeyCode::Char('j') if choosing => {
            select(state, state.environment_view.selected + 1, count)
        }
        KeyCode::Up | KeyCode::Char('k') if choosing => select(
            state,
            state.environment_view.selected.saturating_sub(1),
            count,
        ),
        KeyCode::Home | KeyCode::Char('g') if choosing => select(state, 0, count),
        KeyCode::End | KeyCode::Char('G') if choosing => select(state, usize::MAX, count),
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
        KeyCode::Enter => confirm(state, &config),
        _ => {}
    }
    Ok(())
}

/// A click on a stage selects its environment, a second click on the selected
/// one previews the promotion, and the wheel moves the selection.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) {
    if state.environment_view.preview.is_some() {
        return;
    }
    let config = crate::preferences::load().config.branches;
    let count = config.environments.len();
    let at = Position::new(m.column, m.row);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(env) = draw::environment_at(&config, state, area, at) {
                if env == state.environment_view.selected {
                    confirm(state, &config);
                } else {
                    select(state, env, count);
                }
            }
        }
        MouseEventKind::ScrollDown => select(state, state.environment_view.selected + 1, count),
        MouseEventKind::ScrollUp => select(
            state,
            state.environment_view.selected.saturating_sub(1),
            count,
        ),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        git::{BranchReleaseStatus, ReleaseTargetStatus},
        preferences::{Branches, detect_branches},
    };
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn detected() -> Branches {
        detect_branches(&["main".into(), "develop".into(), "test".into()])
    }

    fn on_branch(branch: &str) -> AppState {
        let mut state = AppState::default();
        state.branch = Some(branch.into());
        state.modal = Modal::Environments;
        state
    }

    fn drawn(state: &AppState, config: &Branches, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_with(state, config, frame.area(), frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Everything a person needs to orient themselves is on the first screen:
    /// the branch they are on, every environment, the branch each deploys and
    /// the rule that leads there.
    #[test]
    fn the_branch_every_environment_and_every_rule_are_on_screen() {
        let mut state = on_branch("feature/x");
        state.decorative_animations = false;
        let config = detected();
        let screen = text(&drawn(&state, &config, 130, 36));
        assert!(screen.contains("feature/x"), "{screen}");
        for env in &config.environments {
            assert!(screen.contains(&env.name), "{}: {screen}", env.name);
            assert!(
                screen.contains(&format!("origin/{}", env.branch)),
                "{}: {screen}",
                env.branch
            );
        }
        assert!(
            screen.contains("merge, push"),
            "the rule is drawn: {screen}"
        );
        assert!(
            screen.contains("no rule"),
            "an unreachable environment says so: {screen}"
        );
    }

    /// The detail pane says how the branch stands against the environment
    /// in words, not a code.
    #[test]
    fn how_far_behind_an_environment_is_is_said_in_words() {
        let mut state = on_branch("feature/x");
        state.decorative_animations = false;
        let config = detected();
        state.current_branch_releases = BranchReleaseStatus {
            environments: [(
                "dev".to_string(),
                ReleaseTargetStatus {
                    released_at: String::new(),
                    missing_commits: 3,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        state.environment_view.selected = config
            .environments
            .iter()
            .position(|e| e.id == "dev")
            .unwrap();
        let screen = text(&drawn(&state, &config, 130, 36));
        assert!(screen.contains("3 commits behind"), "{screen}");
        assert!(screen.contains("not in Development yet"), "{screen}");
    }

    /// An environment nothing promotes into is explained, with the fix.
    #[test]
    fn a_missing_promotion_rule_is_explained() {
        let mut state = on_branch("feature/x");
        state.decorative_animations = false;
        let config = detected();
        state.environment_view.selected = config
            .environments
            .iter()
            .position(|e| e.id == "prod")
            .unwrap();
        let screen = text(&drawn(&state, &config, 130, 36));
        assert!(
            screen.contains("No rule promotes feature into Production"),
            "{screen}"
        );
        assert!(screen.contains("to = prod"), "{screen}");
    }

    /// The route to the selected environment moves while animations are on
    /// and holds still when they are off.
    #[test]
    fn the_promotion_route_is_animated_only_when_animations_are_on() {
        let mut state = on_branch("feature/x");
        let config = detected();
        state.environment_view.selected = config
            .environments
            .iter()
            .position(|e| e.id == "dev")
            .unwrap();
        state.decorative_animations = false;
        state.animation_ms = 0;
        let still_a = text(&drawn(&state, &config, 130, 36));
        state.animation_ms = 660;
        let still_b = text(&drawn(&state, &config, 130, 36));
        assert_eq!(still_a, still_b, "a still picture must not move");

        state.decorative_animations = true;
        state.animation_ms = 0;
        let moving_a = text(&drawn(&state, &config, 130, 36));
        state.animation_ms = 660;
        let moving_b = text(&drawn(&state, &config, 130, 36));
        assert_ne!(moving_a, moving_b, "the marker must be seen to travel");
        assert!(moving_a.contains('\u{25c6}') || moving_b.contains('\u{25c6}'));
    }

    /// The picture still fits, and still names everything, in a small window.
    #[test]
    fn a_small_window_still_shows_every_environment() {
        let mut state = on_branch("feature/x");
        state.decorative_animations = false;
        let config = detected();
        let screen = text(&drawn(&state, &config, 80, 16));
        for env in &config.environments {
            assert!(screen.contains(&env.name), "{}: {screen}", env.name);
        }
    }

    /// Escape leaves a preview for the list first, and the list for the app.
    #[test]
    fn escape_steps_back_out_of_a_preview_before_closing() {
        let mut state = on_branch("feature/x");
        state.environment_view.preview = Some(crate::git::environments::PromotionPreview {
            source: "feature/x".into(),
            source_oid: "0123456789abcdef".into(),
            target_oid: "fedcba9876543210".into(),
            environment: detected().environments[0].clone(),
            rule: Default::default(),
            commits: vec!["0123456 first".into()],
        });
        let esc = KeyEvent::from(KeyCode::Esc);
        handle_key(&mut state, esc).unwrap();
        assert!(state.environment_view.preview.is_none());
        assert_eq!(state.modal, Modal::Environments);
        handle_key(&mut state, esc).unwrap();
        assert_eq!(state.modal, Modal::None);
    }

    /// The preview lists the commits that would move and where they land.
    #[test]
    fn a_preview_lists_the_commits_that_would_move() {
        let mut state = on_branch("feature/x");
        state.decorative_animations = false;
        let config = detected();
        let dev = config
            .environments
            .iter()
            .position(|e| e.id == "dev")
            .unwrap();
        state.environment_view.selected = dev;
        state.environment_view.preview = Some(crate::git::environments::PromotionPreview {
            source: "feature/x".into(),
            source_oid: "0123456789abcdef".into(),
            target_oid: "fedcba9876543210".into(),
            environment: config.environments[dev].clone(),
            rule: config.promotions[0].clone(),
            commits: vec![
                "0123456 add the thing".into(),
                "89abcde fix the thing".into(),
            ],
        });
        let screen = text(&drawn(&state, &config, 130, 36));
        assert!(screen.contains("2 commits would move"), "{screen}");
        assert!(screen.contains("add the thing"), "{screen}");
        assert!(screen.contains("fix the thing"), "{screen}");
        assert!(screen.contains("origin/develop"), "{screen}");
    }
}
