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
            Self::DiscardCheckout => "Discard checkout, reload from remote",
            Self::NewFeature => "Start new feature from origin/main",
            Self::TransferDiff => "Move selected diff to a new branch",
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

/// What a branch action runs against: the action, the branch it acts on, and
/// the names it was given.
///
/// A flow checks other branches out while it runs, so anything drawn about a
/// running one has to read the names it started with rather than whatever
/// happens to be checked out at the moment. The menu resolves one of these too,
/// so the picture it draws and the picture the run draws are built the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRun {
    pub action: FlowAction,
    /// The branch the action acts on — for a diff transfer the selected branch,
    /// otherwise the checkout.
    pub branch: String,
    /// The deploy branch a release or reset lands on.
    pub target: Option<String>,
    /// The name typed for the actions that create a branch.
    pub input: Option<String>,
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
