//! The branch flow actions and when each one is offered.

use crate::config::BRANCH_MAIN;
use crate::git::ReleaseEnv;

use super::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowAction {
    MergeMain,
    ReleaseDev,
    ReleaseTest,
    ResetDev,
    ResetTest,
    DiscardCheckout,
    NewFeature,
    TransferDiff,
    CleanOrphans,
}

impl FlowAction {
    pub const ALL: [Self; 9] = [
        Self::MergeMain,
        Self::ReleaseDev,
        Self::ReleaseTest,
        Self::ResetDev,
        Self::ResetTest,
        Self::DiscardCheckout,
        Self::NewFeature,
        Self::TransferDiff,
        Self::CleanOrphans,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::MergeMain => "Merge origin/main into current branch",
            Self::ReleaseDev => "Release current branch into develop",
            Self::ReleaseTest => "Release current branch into test",
            Self::ResetDev => "Reset develop from origin/main",
            Self::ResetTest => "Reset test from origin/main",
            Self::DiscardCheckout => "Discard current checkout and reload from remote",
            Self::NewFeature => "Start new feature from origin/main",
            Self::TransferDiff => "Transfer selected feature diff to new branch",
            Self::CleanOrphans => "Clean local branches without upstream",
        }
    }

    /// The environment a release or reset action targets. `None` for actions
    /// that do not touch a deploy branch.
    pub fn release_env(self) -> Option<ReleaseEnv> {
        match self {
            Self::ReleaseDev | Self::ResetDev => Some(ReleaseEnv::Dev),
            Self::ReleaseTest | Self::ResetTest => Some(ReleaseEnv::Test),
            _ => None,
        }
    }

    pub fn needs_confirmation(self) -> bool {
        !matches!(self, Self::NewFeature | Self::TransferDiff)
    }

    pub fn needs_input(self) -> bool {
        matches!(self, Self::NewFeature | Self::TransferDiff)
    }
}

impl AppState {
    /// Whether this checkout deploys from any branch at all. One deploy branch
    /// is enough — the release actions for the missing one stay hidden.
    pub fn flow_available(&self) -> bool {
        self.release_branch(ReleaseEnv::Dev).is_some()
            || self.release_branch(ReleaseEnv::Test).is_some()
    }

    /// The label for a flow action, naming the deploy branch this checkout
    /// actually uses instead of the default spelling.
    pub fn flow_action_label(&self, action: FlowAction) -> String {
        let Some(branch) = action
            .release_env()
            .and_then(|env| self.release_branch(env))
        else {
            return action.label().to_string();
        };
        match action {
            FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
                format!("Release current branch into {branch}")
            }
            FlowAction::ResetDev | FlowAction::ResetTest => {
                format!("Reset {branch} from origin/{BRANCH_MAIN}")
            }
            _ => action.label().to_string(),
        }
    }
}
