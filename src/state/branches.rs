//! What the branch lists hold and how the selected branch stands against main.

use crate::git::{ReleaseEnv, RemoteBranch};

use super::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchView {
    Local,
    Remote,
}

impl AppState {
    pub fn branch_exists(&self, name: &str) -> bool {
        self.branches.iter().any(|branch| branch.name == name)
    }

    pub fn branch_list_len(&self) -> usize {
        match self.branch_view {
            BranchView::Local => self.branches.len(),
            BranchView::Remote => self.visible_remote_branches().count(),
        }
    }

    pub fn branch_list_idx_mut(&mut self) -> &mut usize {
        match self.branch_view {
            BranchView::Local => &mut self.branches_idx,
            BranchView::Remote => &mut self.remote_branches_idx,
        }
    }

    pub fn selected_branch_ref(&self) -> Option<&str> {
        match self.branch_view {
            BranchView::Local => self
                .branches
                .get(self.branches_idx)
                .map(|branch| branch.name.as_str()),
            BranchView::Remote => self
                .visible_remote_branches()
                .nth(self.remote_branches_idx)
                .map(|branch| branch.name.as_str()),
        }
    }

    pub fn nested_repo_branch_list_idx_mut(&mut self) -> &mut usize {
        match self.nested_repo_branch_view {
            BranchView::Local => &mut self.nested_repo_branches_idx,
            BranchView::Remote => &mut self.nested_repo_remote_branches_idx,
        }
    }

    pub fn selected_nested_repo_branch_ref(&self) -> Option<&str> {
        match self.nested_repo_branch_view {
            BranchView::Local => self
                .nested_repo_branches
                .get(self.nested_repo_branches_idx)
                .map(|branch| branch.name.as_str()),
            BranchView::Remote => self
                .visible_nested_repo_remote_branches()
                .nth(self.nested_repo_remote_branches_idx)
                .map(|branch| branch.name.as_str()),
        }
    }

    pub fn visible_nested_repo_remote_branches(&self) -> impl Iterator<Item = &RemoteBranch> {
        self.nested_repo_remote_branches
            .iter()
            .filter(|branch| !self.nested_repo_remote_branch_checked_out_locally(branch))
    }

    pub fn nested_repo_remote_branch_checked_out_locally(&self, remote: &RemoteBranch) -> bool {
        self.nested_repo_branches.iter().any(|local| {
            local.name == remote.local_name
                || local.upstream.as_deref() == Some(remote.name.as_str())
        })
    }

    pub fn visible_remote_branches(&self) -> impl Iterator<Item = &RemoteBranch> {
        self.remote_branches
            .iter()
            .filter(|branch| !self.remote_branch_checked_out_locally(branch))
    }

    pub fn remote_branch_checked_out_locally(&self, remote: &RemoteBranch) -> bool {
        self.branches.iter().any(|local| {
            local.name == remote.local_name
                || local.upstream.as_deref() == Some(remote.name.as_str())
        })
    }

    /// The branch that deploys `env` in this checkout, if it has one. Before
    /// the first refresh snapshot lands, the environments are read from the
    /// configuration and checked against the local branch list so the panels
    /// are right from the start.
    pub fn release_branch(&self, env: ReleaseEnv) -> Option<&str> {
        if self.release_branches.configured || self.release_branches.any() {
            return self.release_branches.branch(env);
        }
        let detected = crate::git::configured_release_branches(&self.environments());
        let name = detected.branch(env)?;
        self.branches
            .iter()
            .map(|b| b.name.as_str())
            .find(|b| *b == name)
    }

    /// The environments in force: the saved ones, or the ones this checkout's
    /// branch list implies when nothing is saved.
    fn environments(&self) -> Vec<crate::preferences::Environment> {
        let loaded = crate::preferences::load();
        if loaded.sources.contains_key("branches") {
            return loaded.config.branches.environments;
        }
        let names: Vec<String> = self.branches.iter().map(|b| b.name.clone()).collect();
        crate::preferences::detect_branches(&names).environments
    }

    /// Whether the branch deploys an environment other than production, so it
    /// is released into rather than treated as a feature branch.
    pub fn is_deploy_branch(&self, name: &str) -> bool {
        let base = crate::preferences::base_branch();
        self.environments()
            .iter()
            .any(|e| !e.branch.is_empty() && e.branch != base && e.branch == name)
    }

    /// Whether the checkout is a feature branch: neither the trunk nor a
    /// deploy branch. Housekeeping that belongs to the trunk is kept off the
    /// menu there, so what is listed is what concerns this branch.
    pub fn on_feature_branch(&self) -> bool {
        let base = crate::preferences::base_branch();
        self.branch
            .as_deref()
            .is_some_and(|branch| branch != base && !self.is_deploy_branch(branch))
    }

    pub fn branch_actions_available(&self) -> bool {
        self.branch.is_some() || !self.branches.is_empty()
    }

    pub fn merge_main_available(&self) -> bool {
        let configured_base = crate::preferences::base_branch();
        let Some(branch) = self.branch.as_deref() else {
            return false;
        };
        match branch {
            value if value == configured_base.as_str() => false,
            _ if self.is_deploy_branch(branch) => {
                self.current_branch_behind_main().is_some_and(|n| n > 0)
            }
            _ => true,
        }
    }

    pub fn current_branch_behind_main(&self) -> Option<u32> {
        let branch = self.branch.as_deref()?;
        self.branches
            .iter()
            .find(|candidate| candidate.is_current || candidate.name == branch)
            .map(|candidate| candidate.behind_main)
    }

    pub fn pull_available(&self) -> bool {
        self.branch.is_some()
            && self
                .current_branch_ahead_behind()
                .is_some_and(|(_, behind)| behind > 0)
    }

    pub fn current_branch_ahead_behind(&self) -> Option<(u32, u32)> {
        self.ahead_behind.or_else(|| {
            let branch = self.branch.as_deref()?;
            self.branches
                .iter()
                .find(|candidate| candidate.is_current || candidate.name == branch)
                .map(|candidate| (candidate.ahead, candidate.behind))
        })
    }

    pub fn branch_diverged_from_remote(&self) -> bool {
        self.current_branch_ahead_behind()
            .is_some_and(|(ahead, behind)| ahead > 0 && behind > 0)
    }

    pub fn branch_behind_remote(&self) -> bool {
        self.current_branch_ahead_behind()
            .is_some_and(|(_, behind)| behind > 0)
    }

    pub fn has_unpushed_commits(&self) -> bool {
        !self.unpushed_shas.is_empty()
            || self
                .current_branch_ahead_behind()
                .is_some_and(|(ahead, _)| ahead > 0)
            || (self.branch.is_some() && !self.commits.is_empty() && self.ahead_behind.is_none())
    }
}
