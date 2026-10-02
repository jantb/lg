//! The modal forms, the prompts they raise, and what confirming one will do.

use std::path::Path;

use crate::git::Branch;

use super::{AppState, FlowAction};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictFollowup {
    /// A branch whose remote head the flow still has to merge into
    /// `push_branch` — the half of a release its conflict interrupted.
    pub merge_branch: Option<String>,
    pub push_branch: Option<String>,
    pub return_branch: Option<String>,
    pub safety_ref_cleanup: Option<SafetyRefCleanup>,
    /// What to run again once the conflict is settled, for a flow whose
    /// remaining work is more than a push and a checkout. Syncing every branch
    /// stops at the first one that will not merge, leaving that branch unpushed
    /// and the branches after it untouched; committing the merge is the start
    /// of finishing that, not the end.
    pub resume: Option<Box<PendingAction>>,
}

impl ConflictFollowup {
    /// What a sync of every branch still owes when it stops on a conflict.
    ///
    /// The sync stops at the first branch that will not merge, and that branch
    /// is then unpushed with every branch after it untouched, so continuing
    /// runs the sync again. `started_on` is the branch the sync was launched
    /// from: it is where the auto-stash was taken and where the work belongs,
    /// and the conflict left the checkout on another branch entirely.
    pub fn for_branch_sync(started_on: Option<String>) -> Self {
        Self {
            return_branch: started_on,
            resume: Some(Box::new(PendingAction::MergeMainAllBranches)),
            ..Self::default()
        }
    }
}

impl AppState {
    /// Put a conflict to rest once it has been validated or aborted, and queue
    /// whatever flow it was holding up.
    ///
    /// Only a validation picks the flow back up. Aborting is giving up on it,
    /// and starting it again would be the opposite of what was asked for.
    pub fn settle_conflict(&mut self, validated: bool) {
        let followup = self.conflict.followup.take();
        self.conflict.files.clear();
        self.conflict.preview = None;
        self.conflict.resolved.clear();
        self.conflict.model_notes.clear();
        self.modal = Modal::None;
        if validated && let Some(resume) = followup.and_then(|followup| followup.resume) {
            self.pending_action = Some(*resume);
        }
    }

    /// Put the conflicted files git reports on screen, forgetting what an
    /// earlier conflict's local pass had settled. A file resolved during the
    /// last conflict says nothing about a file of the same name in this one.
    pub fn set_conflicts(&mut self, conflicts: Vec<String>) {
        self.conflict.files = conflicts;
        self.conflict.preview = None;
        self.conflict.idx = 0;
        self.conflict.scroll_offset = 0;
        self.conflict.resolved.clear();
        self.conflict.model_notes.clear();
    }

    /// Open the picker that says which agent to start in the selected checkout.
    ///
    /// `sandboxed` is carried on the state rather than asked again on the way
    /// out: `s` and `S` differ only in that, and having chosen it once the
    /// picker is about the agent and nothing else.
    pub fn open_agent_picker(&mut self, sandboxed: bool) {
        self.agent_profiles = crate::panel::environments::selected_checkout(self)
            .map(|(path, _)| {
                crate::git::with_repo(path, || crate::preferences::load().config.agents)
            })
            .unwrap_or_else(|| crate::preferences::load().config.agents);
        self.agent_pick_sandboxed = sandboxed;
        self.agent_pick_idx = crate::session::SessionKind::AGENTS
            .iter()
            .position(|kind| *kind == self.preferred_agent)
            .unwrap_or(0);
        if let Some(index) = self.agent_profiles.iter().position(|p| p.default) {
            self.agent_pick_idx = index;
        }
        for profile in &mut self.agent_profiles {
            if !sandboxed || profile.adapter == crate::preferences::Adapter::Terminal {
                profile.confinement = crate::preferences::default_confinement(profile.adapter);
            }
        }
        self.modal = Modal::Agent;
    }

    /// The agent the picker has landed on.
    pub fn picked_agent(&self) -> crate::session::SessionKind {
        let agents = crate::session::SessionKind::AGENTS;
        agents[self.agent_pick_idx.min(agents.len() - 1)]
    }

    /// The conflicted files the local model has not settled — what is left for
    /// somebody to read.
    pub fn unresolved_conflicts(&self) -> Vec<String> {
        self.conflict
            .files
            .iter()
            .filter(|path| !self.conflict.resolved.contains(*path))
            .cloned()
            .collect()
    }
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
    Settings,
    Environments,
    Commands,
    Help,
    Flow,
    /// Which coding agent to start in the selected checkout.
    Agent,
    /// What can be done to the selected row of the repository tree.
    RepoActions,
    Conflict,
    DeleteBranch,
    Worktree,
    ReviewChat,
    ConfirmDestructive,
    /// Pull requests of the checkout on screen, and repositories to clone.
    GitHub,
    /// A branch or pull request walked one hunk at a time.
    GuidedReview,
    /// The stash: its entries, and a new one being named.
    Stash,
    /// The whole of the last error, which the status bar had to cut short.
    StatusDetails,
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
    StartAgent {
        path: String,
        label: String,
        profile: crate::preferences::Agent,
    },
    Promote(crate::git::environments::PromotionPreview),
    GenerateMessage,
    ReviewAssist(String),
    ReviewPrText,
    ReviewStyleFlags,
    /// Start a Claude Code session reviewing the whole branch.
    ReviewAgent,
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
        /// `None` keeps the limit already saved; `Some(0)` is unlimited.
        commit_subject_max_chars: Option<usize>,
        commit_body_max_lines: Option<usize>,
    },
    ClearSettings,
    EditCommitPrompt,
    EditReviewStyle,
    /// Commit what is staged as a replacement for the last commit.
    AmendCommit,
    /// Stage, unstage or discard one hunk of the diff pane.
    ApplyHunk {
        op: crate::git::hunk::HunkOp,
        hunk: Box<crate::git::hunk::ShownHunk>,
    },
    /// Add a commit undoing this one.
    RevertCommit {
        sha: String,
    },
    /// Replay this commit, from another branch, onto the current one.
    CherryPick {
        sha: String,
    },
    /// Stash every change, untracked files too.
    StashPush {
        message: String,
    },
    StashApply {
        sha: String,
    },
    StashPop {
        sha: String,
    },
    StashDrop {
        sha: String,
    },
    /// Push over a diverged upstream, only while it is where it was when the
    /// prompt showed what it would overwrite.
    ForcePushWithLease(Box<crate::git::ForcePushPlan>),
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
    /// Open a file at a line in the terminal editor, with lg suspended until
    /// it exits.
    EditFile {
        path: String,
        line: usize,
    },
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
    /// Turn a plain folder into a repository.
    InitRepository {
        path: String,
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
    GitHub(GitHubAction),
    Quit,
    /// Stop a session and forget it. `follow` puts the next session on
    /// screen, as closing one from the session pane does; the tree leaves the
    /// view alone.
    CloseSession {
        id: crate::session::SessionId,
        follow: bool,
    },
}

/// Something to do on GitHub, through `gh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubAction {
    /// Check a pull request out in the checkout on screen.
    Checkout {
        number: u64,
    },
    /// Check a pull request out in a new worktree at `path`.
    CheckoutWorktree {
        number: u64,
        path: String,
    },
    Review {
        number: u64,
        verdict: crate::github::Verdict,
        body: String,
    },
    Comment {
        number: u64,
        body: String,
    },
    /// One review with inline comments, from a guided review's notes.
    SubmitReview {
        number: u64,
        commit: String,
        event: crate::github::ReviewEvent,
        body: String,
        comments: Vec<crate::github::LineComment>,
    },
    Merge {
        number: u64,
        method: crate::github::MergeMethod,
        delete_branch: bool,
        auto: bool,
    },
    /// Turn a pull request into a draft, or mark a draft ready for review.
    SetDraft {
        number: u64,
        draft: bool,
    },
    Close {
        number: u64,
    },
    Reopen {
        number: u64,
    },
    Create(crate::github::NewPullRequest),
    /// Clone a repository into `dir` and point lg at it.
    Clone {
        repo: String,
        dir: String,
    },
    OpenInBrowser {
        url: String,
    },
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
    /// What was said. This, and only this, is replayed to the model as
    /// history on later turns.
    pub content: String,
    /// Something lg has to say about the message — that it was cut off, that
    /// the request failed. Shown under it, never sent: the model must not be
    /// handed a note about its own answer as if it had written it.
    pub note: Option<String>,
}

impl ReviewChatMessage {
    pub fn new(role: ReviewChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            note: None,
        }
    }
}

impl AppState {
    /// Quit, or ask first: leaving stops every running session, and that is not
    /// something to discover afterwards.
    pub fn request_quit(&mut self) {
        if self
            .conflict
            .preview
            .as_ref()
            .is_some_and(|preview| preview.editor.as_ref().is_ok_and(|editor| editor.dirty()))
        {
            self.modal = Modal::Conflict;
            self.set_status(
                "unsaved merge edits: Ctrl-s saves; Ctrl-r reloads and discards the draft",
                true,
            );
            return;
        }
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
        self.worktree_form.repo_dir = self
            .worktrees
            .iter()
            .find(|worktree| worktree.is_main)
            .map(|worktree| worktree.path.clone())
            .or_else(|| self.repo_root.clone())
            .unwrap_or_default();
        self.worktree_form.branch.clear();
        self.worktree_form.base.set(base_ref);
        self.worktree_form.path_edited = false;
        self.worktree_form.field = WorktreeField::Branch;
        self.sync_worktree_path();
        self.modal = Modal::Worktree;
    }

    /// Re-derive the path from the branch name, unless the user took it over.
    pub fn sync_worktree_path(&mut self) {
        if self.worktree_form.path_edited {
            return;
        }
        let repo_dir = Path::new(&self.worktree_form.repo_dir);
        let branch = self.worktree_form.branch.as_str().trim();
        let path = if branch.is_empty() {
            String::new()
        } else {
            crate::git::default_worktree_path(repo_dir, branch)
                .to_string_lossy()
                .into_owned()
        };
        self.worktree_form.path.set(path);
    }

    pub fn open_commit_modal(&mut self) {
        self.modal = Modal::Commit;
        if self.commit_amend {
            // The last commit's message is what is being edited; a draft the
            // model wrote for a new commit is not, and stays where it is.
            self.commit_files_scroll = 0;
            self.commit_cursor = self.commit_message.chars().count();
            return;
        }
        // A message written for this checkout while the modal was closed has
        // been waiting in its draft; this is where it is picked up. Another
        // checkout's draft stays where it is.
        self.take_ready_draft();
        self.commit_files_scroll = 0;
        self.commit_cursor = self.commit_message.chars().count();
        if self.commit_message.is_empty() && !self.generating() && self.ai_assist {
            if self.model_server_unreachable {
                self.set_status(
                    "no model server answered earlier \u{2014} type the message, or Ctrl+R to ask again",
                    false,
                );
            } else {
                self.set_status("generating\u{2026}", false);
                self.pending_action = Some(PendingAction::GenerateMessage);
            }
        }
    }

    /// Open the commit modal on amending the last commit: its message in the
    /// editor, ready to be changed, and the staged changes going into it.
    pub fn open_amend_modal(&mut self) {
        if self.generating() {
            self.set_status(
                "a message is being written: wait for it, or Ctrl+R in the commit modal",
                false,
            );
            return;
        }
        let message = match crate::git::head_commit_message() {
            Ok(message) => message,
            Err(err) => {
                self.set_status(format!("nothing to amend: {err}"), true);
                return;
            }
        };
        if !self.commit_amend {
            self.commit_amend_draft = Some(std::mem::take(&mut self.commit_message));
        }
        self.commit_amend = true;
        self.commit_message = message;
        self.modal = Modal::Commit;
        self.commit_files_scroll = 0;
        self.commit_cursor = self.commit_message.chars().count();
        self.set_status(
            "amending the last commit \u{2014} Ctrl+S replaces it, Ctrl+T makes a new commit instead",
            false,
        );
    }

    /// Turn amending off, putting back the message that was being written
    /// before it was turned on.
    pub fn leave_amend(&mut self) {
        self.commit_amend = false;
        self.commit_message = self.commit_amend_draft.take().unwrap_or_default();
        self.commit_cursor = self.commit_message.chars().count();
        self.set_status("writing a new commit", false);
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
        self.delete_branch.target = branch.name.clone();
        self.delete_branch.local = true;
        // The remote is offered when one is tracked, but left unticked: a
        // remote branch may be someone else's work too, and deleting it is
        // the half of this that cannot be taken back locally.
        self.delete_branch.remote_available = branch.upstream.is_some() && !branch.upstream_gone;
        self.delete_branch.remote = false;
        self.delete_branch.force = false;
        self.delete_branch.field = if self.delete_branch.remote_available {
            DeleteBranchField::Local
        } else {
            DeleteBranchField::Force
        };
        self.modal = Modal::DeleteBranch;
    }
}
