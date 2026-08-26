//! The modal forms, the prompts they raise, and what confirming one will do.

use std::path::Path;

use crate::git::Branch;

use super::{AppState, FlowAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFollowup {
    /// A branch whose remote head the flow still has to merge into
    /// `push_branch` — the half of a release its conflict interrupted.
    pub merge_branch: Option<String>,
    pub push_branch: Option<String>,
    pub return_branch: Option<String>,
    pub safety_ref_cleanup: Option<SafetyRefCleanup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyRefCleanup {
    pub label: String,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    None,
    Commit,
    StageAllBeforeCommit,
    Push,
    Author,
    Model,
    Help,
    Flow,
    Conflict,
    DeleteBranch,
    Worktree,
    ReviewChat,
    ConfirmDestructive,
}

/// Rows of the new-worktree form. The path derives from the branch until the
/// user edits it, so it is a field rather than a preview line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeField {
    Branch,
    Base,
    Path,
}

impl WorktreeField {
    pub const ALL: [Self; 3] = [Self::Branch, Self::Base, Self::Path];

    pub fn next(self, forward: bool) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        }]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteBranchField {
    Local,
    Remote,
    Force,
}

/// Focused row in the settings modal. Model lives here too so one Tab cycle
/// walks every editable setting, and `Save` is a row so Enter on it commits the
/// whole form the same way Enter opens a value list on the rows above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Model,
    PrLanguage,
    CommentStyle,
    SubjectMax,
    BodyLines,
    Save,
}

impl SettingsField {
    pub const ALL: [Self; 6] = [
        Self::Model,
        Self::PrLanguage,
        Self::CommentStyle,
        Self::SubjectMax,
        Self::BodyLines,
        Self::Save,
    ];

    pub fn next(self, forward: bool) -> Self {
        let idx = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        let len = Self::ALL.len();
        let idx = if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        };
        Self::ALL[idx]
    }

    /// Rows whose value is chosen from a list; the rest are typed into.
    pub fn choices(self) -> &'static [&'static str] {
        match self {
            Self::Model => crate::config::LLM_MODEL_CHOICES,
            Self::PrLanguage => crate::config::PR_LANGUAGE_CHOICES,
            _ => &[],
        }
    }
}

/// Whether the settings modal is moving between rows or editing one row's
/// value. Editing is entered with Enter and confirmed with Enter, so arrow keys
/// mean "next row" in one mode and "next value" in the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsMode {
    Browse,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorField {
    Path,
    Name,
    Email,
}

/// A destructive action parked behind an explicit y/n confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmPrompt {
    pub title: String,
    pub question: String,
    pub detail: String,
    pub action: PendingAction,
    /// Whether the action can be walked back afterwards. A prompt that warns
    /// about everything gets skimmed, and then the warning is missing from the
    /// one that needed it.
    pub reversible: bool,
}

/// Which checkout to point lg at. A worktree can live outside the workspace, so
/// it carries an absolute path rather than a workspace-relative one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoTarget {
    /// The checkout lg was started in.
    Workspace,
    /// A repository found inside the workspace, by workspace-relative path.
    Nested(String),
    /// Any checkout, by absolute path.
    Path(std::path::PathBuf),
}

impl RepoTarget {
    /// Resolve to a directory. `workspace_root` is where relative targets are
    /// anchored; it is the workspace root, or the current repository when lg
    /// has not established one yet.
    pub fn resolve(&self, workspace_root: &Path) -> std::path::PathBuf {
        match self {
            Self::Workspace => workspace_root.to_path_buf(),
            Self::Nested(path) => workspace_root.join(path),
            Self::Path(path) => path.clone(),
        }
    }

    /// How to name this target in a status message.
    pub fn label(&self) -> String {
        match self {
            Self::Workspace => "workspace".to_string(),
            Self::Nested(path) => path.clone(),
            Self::Path(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    GenerateMessage,
    ReviewAssist(String),
    ReviewPrText,
    ReviewStyleFlags,
    ReviewChat(String),
    CopyToClipboard {
        label: String,
        text: String,
    },
    Commit,
    StageAllAndCommit,
    Push,
    Pull,
    MergeUpstream,
    MergeMainAllBranches,
    Flow(FlowAction),
    SaveAuthor {
        name: String,
        email: String,
    },
    ClearAuthor,
    SaveSubtreeAuthor {
        path: String,
        name: String,
        email: String,
    },
    ClearSubtreeAuthor {
        path: String,
    },
    SaveSettings {
        model: String,
        provider: crate::llm::LlmProvider,
        pr_language: String,
        comment_style: String,
        commit_subject_max_chars: String,
        commit_body_max_lines: String,
    },
    ClearSettings,
    EditCommitPrompt,
    StageAll,
    UnstageAll,
    StagePath(String),
    UnstagePath(String),
    RollbackPath {
        path: String,
        is_dir: bool,
    },
    DeletePath {
        path: String,
        is_dir: bool,
    },
    IgnorePath {
        path: String,
        is_dir: bool,
    },
    OpenProject,
    OpenProjectAt(String),
    OpenFile(String),
    DeleteBranch {
        name: String,
        delete_local: bool,
        delete_remote: bool,
        force: bool,
    },
    SetBranchUpstream {
        branch: String,
        upstream: String,
    },
    SwitchRepository {
        target: RepoTarget,
    },
    CreateWorktree {
        path: String,
        branch: String,
        base: String,
    },
    RemoveWorktree {
        path: String,
        force: bool,
    },
    /// Merge a worktree's branch into main, then remove both.
    LandWorktree {
        path: String,
        branch: String,
    },
    /// Merge main into a worktree's branch, in the worktree.
    SyncWorktree {
        path: String,
        branch: String,
    },
    /// Remove a worktree and check its branch out in the main checkout.
    BringWorktreeHome {
        path: String,
        branch: String,
    },
    PruneWorktrees,
    StartSession {
        path: String,
        label: String,
        sandboxed: bool,
        kind: crate::session::SessionKind,
        /// What the session opens on, when it is started to deal with something
        /// lg already knows about.
        prompt: Option<String>,
    },
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewChatRole {
    User,
    Assistant,
}

impl ReviewChatRole {
    pub fn as_chat_role(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewChatMessage {
    pub role: ReviewChatRole,
    pub content: String,
}

impl AppState {
    /// Quit, or ask first: leaving stops every running session, and that is not
    /// something to discover afterwards.
    pub fn request_quit(&mut self) {
        let running: Vec<&str> = self
            .sessions
            .iter()
            .filter(|session| session.is_running())
            .map(|session| session.label.as_str())
            .collect();
        if running.is_empty() {
            self.should_quit = true;
            return;
        }
        let count = running.len();
        let plural = if count == 1 { "session" } else { "sessions" };
        let detail = running.join(", ");
        self.confirm_action(
            "Quit lg",
            format!("Quit and stop {count} running {plural}?"),
            detail,
            PendingAction::Quit,
        );
    }

    /// Park a destructive action behind a y/n confirmation modal.
    pub fn confirm_action(
        &mut self,
        title: impl Into<String>,
        question: impl Into<String>,
        detail: impl Into<String>,
        action: PendingAction,
    ) {
        self.confirm = Some(ConfirmPrompt {
            title: title.into(),
            question: question.into(),
            detail: detail.into(),
            action,
            reversible: false,
        });
        self.modal = Modal::ConfirmDestructive;
    }

    /// Park an action behind the same y/n prompt, without the warning: this one
    /// can be undone by hand afterwards.
    pub fn confirm_reversible_action(
        &mut self,
        title: impl Into<String>,
        question: impl Into<String>,
        detail: impl Into<String>,
        action: PendingAction,
    ) {
        self.confirm_action(title, question, detail, action);
        if let Some(prompt) = self.confirm.as_mut() {
            prompt.reversible = true;
        }
    }

    /// Open the new-worktree form for the active repository. The path follows
    /// the branch name until the user edits it.
    pub fn open_worktree_modal(&mut self, base_ref: String) {
        self.worktree_repo_dir = self
            .worktrees
            .iter()
            .find(|worktree| worktree.is_main)
            .map(|worktree| worktree.path.clone())
            .or_else(|| self.repo_root.clone())
            .unwrap_or_default();
        self.worktree_branch_input.clear();
        self.worktree_base_input = base_ref;
        self.worktree_path_edited = false;
        self.worktree_field = WorktreeField::Branch;
        self.sync_worktree_path();
        self.modal = Modal::Worktree;
    }

    /// Re-derive the path from the branch name, unless the user took it over.
    pub fn sync_worktree_path(&mut self) {
        if self.worktree_path_edited {
            return;
        }
        let repo_dir = Path::new(&self.worktree_repo_dir);
        let branch = self.worktree_branch_input.trim();
        self.worktree_path_input = if branch.is_empty() {
            String::new()
        } else {
            crate::git::default_worktree_path(repo_dir, branch)
                .to_string_lossy()
                .into_owned()
        };
    }

    pub fn open_commit_modal(&mut self) {
        self.modal = Modal::Commit;
        self.commit_cursor = self.commit_message.chars().count();
        if self.commit_message.is_empty() && self.generation.is_none() {
            self.set_status("generating\u{2026}", false);
            self.pending_action = Some(PendingAction::GenerateMessage);
        }
    }

    pub fn open_commit_or_stage_all_prompt(&mut self) {
        let (staged, unstaged, untracked) = self.file_counts();
        if staged == 0 && unstaged == 0 && untracked == 0 {
            self.set_status("nothing to commit", false);
            self.modal = Modal::None;
        } else if staged == 0 && (unstaged > 0 || untracked > 0) {
            self.modal = Modal::StageAllBeforeCommit;
        } else {
            self.open_commit_modal();
        }
    }

    pub fn open_delete_branch_modal(&mut self, branch: &Branch) {
        self.delete_branch_target = branch.name.clone();
        self.delete_branch_local = true;
        // Default: also delete the remote when one is tracked, so a single
        // confirm cleans up both. Skip the toggle when there is no remote.
        self.delete_branch_remote_available = branch.upstream.is_some() && !branch.upstream_gone;
        self.delete_branch_remote = self.delete_branch_remote_available;
        self.delete_branch_force = false;
        self.delete_branch_field = if self.delete_branch_remote_available {
            DeleteBranchField::Local
        } else {
            DeleteBranchField::Force
        };
        self.modal = Modal::DeleteBranch;
    }
}
