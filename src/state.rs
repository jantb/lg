use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::{
    config::{BRANCH_MAIN, BRANCH_TEST, DEV_BRANCH_NAMES, is_deploy_branch_name},
    git::{
        AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo,
        ReleaseBranches, ReleaseEnv, RemoteBranch, Worktree,
    },
};

mod jobs;
mod tree;

pub use jobs::*;
pub use tree::{TreeKind, TreeRow, build_tree_rows};

pub fn clamp_index(idx: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(idx.min(len - 1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFollowup {
    pub push_branch: Option<String>,
    pub return_branch: Option<String>,
    pub safety_ref_cleanup: Option<SafetyRefCleanup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyRefCleanup {
    pub label: String,
    pub branch: String,
}

/// Which shape lg is in. Git mode is the full git view; workspace mode trades
/// the git panes for one tall list of checkouts and their sessions, with the
/// focused session filling the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Git,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Status,
    Files,
    Branches,
    Commits,
    Main,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    None,
    All,
    File(String),   // path
    Folder(String), // folder prefix (no trailing slash)
    Commit(String), // sha
    Branch(String), // branch name
    Review,
}

/// What the main pane is showing. The diff sources say *what* is diffed; this
/// says whether a diff is what is on screen at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainView {
    Diff,
    Session(crate::session::SessionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewMode {
    Unified,
    SideBySide,
}

#[derive(Debug, Clone)]
pub struct StatusMsg {
    pub text: String,
    pub is_error: bool,
    pub at: DateTime<Utc>,
}

/// A destructive action parked behind an explicit y/n confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmPrompt {
    pub title: String,
    pub question: String,
    pub detail: String,
    pub action: PendingAction,
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
    PruneWorktrees,
    StartSession {
        path: String,
        label: String,
        sandboxed: bool,
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

pub struct AppState {
    pub mode: AppMode,
    pub focus: Pane,
    pub modal: Modal,
    pub prev_focus: Pane,
    pub help_offset: u16,

    pub files: Vec<FileEntry>,
    pub branches: Vec<Branch>,
    pub remote_branches: Vec<RemoteBranch>,
    pub nested_repositories: Vec<NestedRepo>,
    /// Every checkout of the active repository, main worktree first. Empty
    /// until the first refresh finishes.
    pub worktrees: Vec<Worktree>,
    pub nested_repo_branches: Vec<Branch>,
    pub nested_repo_remote_branches: Vec<RemoteBranch>,
    pub commits: Vec<Commit>,
    pub commits_ref: Option<String>,
    pub current_branch_releases: BranchReleaseStatus,
    pub current_branch_releases_ref: Option<String>,
    pub release_branches: ReleaseBranches,
    pub unpushed_shas: HashSet<String>,

    pub files_idx: usize,
    pub branches_idx: usize,
    pub remote_branches_idx: usize,
    pub nested_repositories_idx: usize,
    pub nested_repo_tree_idx: usize,
    pub nested_repo_branches_idx: usize,
    pub nested_repo_remote_branches_idx: usize,
    pub commits_idx: usize,
    pub files_scroll_offset: usize,
    pub branches_scroll_offset: usize,
    pub remote_branches_scroll_offset: usize,
    pub nested_repositories_scroll_offset: usize,
    pub nested_repo_branches_scroll_offset: usize,
    pub nested_repo_remote_branches_scroll_offset: usize,
    pub commits_scroll_offset: usize,

    pub collapsed_dirs: HashSet<String>,

    /// Terminal sessions lg is keeping alive, one per checkout.
    pub sessions: crate::session::Sessions,
    pub main_view: MainView,
    /// Keys go to the focused session instead of to lg.
    pub session_capture: bool,

    pub diff_text: String,
    pub diff_offset: u16,
    pub diff_source: DiffSource,
    pub diff_view_mode: DiffViewMode,
    pub diff_line_count: u16,
    pub diff_viewport_height: u16,
    pub diff_viewport_width: u16,
    pub review: Option<AssistedReview>,
    pub review_idx: usize,
    pub review_collapsed: HashSet<String>,
    pub review_context_open: HashSet<String>,
    pub review_context_restore_collapsed: HashSet<String>,
    pub review_assists: HashMap<String, String>,
    pub review_style_findings: HashMap<String, ReviewStyleFinding>,
    pub review_flag_active_path: Option<String>,
    pub review_chat_messages: Vec<ReviewChatMessage>,
    pub review_chat_input: String,
    pub review_chat_cursor: usize,
    pub review_chat_scroll: u16,
    pub review_chat_height: Option<u16>,
    pub review_chat_drag_active: bool,

    pub commit_message: String,
    pub commit_cursor: usize,
    pub commit_scroll_offset: usize,
    pub author_path_input: String,
    pub author_name_input: String,
    pub author_email_input: String,
    pub author_field: AuthorField,
    pub author_has_local_override: bool,
    pub author_has_subtree_rule: bool,
    pub llm_model: String,
    pub llm_model_input: String,
    pub llm_model_idx: usize,
    pub llm_provider: crate::llm::LlmProvider,
    pub llm_provider_idx: usize,
    pub llm_config_path: String,
    pub settings_field: SettingsField,
    pub settings_mode: SettingsMode,
    /// Value of the row being edited as it was before editing started, so Esc
    /// can put it back.
    pub settings_edit_backup: String,
    pub settings_pr_language_input: String,
    pub settings_comment_style_input: String,
    /// Message shapes derived from this checkout's history, offered as the
    /// choice list for the message-shape row.
    pub settings_comment_style_choices: Vec<String>,
    /// Rows whose current value came from the history scan rather than from a
    /// saved setting or the user, so the modal can say where they came from.
    pub settings_derived_language: bool,
    pub settings_derived_shape: bool,
    pub settings_subject_max_input: String,
    pub settings_body_lines_input: String,
    pub settings_prompt_is_custom: bool,
    pub settings_dir: String,
    pub repo_root: Option<String>,
    pub workspace_root: Option<String>,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub ahead_behind: Option<(u32, u32)>,
    pub nested_repo_detail_path: Option<String>,

    pub status: Option<StatusMsg>,
    pub pending_action: Option<PendingAction>,
    pub confirm: Option<ConfirmPrompt>,
    pub push_after_commit: bool,
    pub should_quit: bool,
    pub animation_tick: usize,
    /// When `animation_tick` last advanced. Animation runs on this clock, not
    /// on the frame rate.
    animation_stepped_at: Instant,

    pub generation: Option<Generation>,
    pub push_job: Option<PushJob>,
    pub checkout_job: Option<CheckoutJob>,
    pub operation_job: Option<OperationJob>,
    pub fetch_job: Option<FetchJob>,
    pub refresh_job: Option<RefreshJob>,
    pub refresh_pending: bool,
    pub refresh_pending_diff: bool,
    pub release_status_job: Option<ReleaseStatusJob>,
    pub settings_suggest_job: Option<SettingsSuggestJob>,
    pub commit_log_job: Option<CommitLogJob>,
    pub diff_job: Option<DiffJob>,
    pub review_job: Option<ReviewJob>,
    pub review_assist_job: Option<ReviewAssistJob>,
    pub review_pr_job: Option<ReviewAssistJob>,
    pub review_flag_job: Option<ReviewFlagJob>,
    pub review_chat_job: Option<ReviewChatJob>,
    pub workflow_job: Option<WorkflowJob>,
    pub deferred_threads: Vec<JoinHandle<()>>,

    pub left_column_width: Option<u16>,
    pub column_drag_active: bool,
    pub left_panel_heights: Option<crate::ui::LeftPanelHeights>,
    pub row_drag_active: Option<(usize, usize)>,

    pub flow_idx: usize,
    pub flow_scroll_offset: usize,
    pub flow_confirm: Option<FlowAction>,
    pub flow_input: Option<FlowAction>,
    pub flow_text: String,

    pub conflicts: Vec<String>,
    pub conflict_idx: usize,
    pub conflict_scroll_offset: usize,
    pub conflict_log: String,
    pub conflict_followup: Option<ConflictFollowup>,

    pub delete_branch_target: String,
    pub delete_branch_local: bool,
    pub delete_branch_remote: bool,
    pub delete_branch_remote_available: bool,
    pub delete_branch_force: bool,
    pub delete_branch_field: DeleteBranchField,
    pub worktree_branch_input: String,
    pub worktree_base_input: String,
    pub worktree_path_input: String,
    pub worktree_field: WorktreeField,
    /// Stops the path following the branch name once the user has typed a path
    /// of their own.
    pub worktree_path_edited: bool,
    /// Main worktree the new one will be created next to, captured when the
    /// form opens so the path preview does not have to re-derive it.
    pub worktree_repo_dir: String,
    pub branch_view: BranchView,
    pub nested_repo_branch_view: BranchView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchView {
    Local,
    Remote,
}

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
    pub fn new() -> Self {
        Self {
            mode: AppMode::Git,
            focus: Pane::Status,
            modal: Modal::None,
            prev_focus: Pane::Status,
            help_offset: 0,

            files: Vec::new(),
            branches: Vec::new(),
            remote_branches: Vec::new(),
            nested_repositories: Vec::new(),
            worktrees: Vec::new(),
            nested_repo_branches: Vec::new(),
            nested_repo_remote_branches: Vec::new(),
            commits: Vec::new(),
            commits_ref: None,
            current_branch_releases: BranchReleaseStatus::default(),
            current_branch_releases_ref: None,
            release_branches: ReleaseBranches::default(),
            unpushed_shas: HashSet::new(),

            files_idx: 0,
            branches_idx: 0,
            remote_branches_idx: 0,
            nested_repositories_idx: 0,
            nested_repo_tree_idx: 0,
            nested_repo_branches_idx: 0,
            nested_repo_remote_branches_idx: 0,
            commits_idx: 0,
            files_scroll_offset: 0,
            branches_scroll_offset: 0,
            remote_branches_scroll_offset: 0,
            nested_repositories_scroll_offset: 0,
            nested_repo_branches_scroll_offset: 0,
            nested_repo_remote_branches_scroll_offset: 0,
            commits_scroll_offset: 0,

            collapsed_dirs: HashSet::new(),

            sessions: crate::session::Sessions::new(),
            main_view: MainView::Diff,
            session_capture: false,

            diff_text: String::new(),
            diff_offset: 0,
            diff_source: DiffSource::None,
            diff_view_mode: DiffViewMode::SideBySide,
            diff_line_count: 0,
            diff_viewport_height: 0,
            diff_viewport_width: 0,
            review: None,
            review_idx: 0,
            review_collapsed: HashSet::new(),
            review_context_open: HashSet::new(),
            review_context_restore_collapsed: HashSet::new(),
            review_assists: HashMap::new(),
            review_style_findings: HashMap::new(),
            review_flag_active_path: None,
            review_chat_messages: Vec::new(),
            review_chat_input: String::new(),
            review_chat_cursor: 0,
            review_chat_scroll: 0,
            review_chat_height: None,
            review_chat_drag_active: false,

            commit_message: String::new(),
            commit_cursor: 0,
            commit_scroll_offset: 0,
            author_path_input: String::new(),
            author_name_input: String::new(),
            author_email_input: String::new(),
            author_field: AuthorField::Path,
            author_has_local_override: false,
            author_has_subtree_rule: false,
            llm_model: crate::llm::current_model(),
            llm_model_input: String::new(),
            llm_model_idx: 0,
            llm_provider: crate::llm::current_provider(),
            llm_provider_idx: 0,
            llm_config_path: crate::llm::config_file_display(),
            settings_field: SettingsField::Model,
            settings_mode: SettingsMode::Browse,
            settings_edit_backup: String::new(),
            settings_pr_language_input: String::new(),
            settings_comment_style_input: String::new(),
            settings_comment_style_choices: Vec::new(),
            settings_derived_language: false,
            settings_derived_shape: false,
            settings_subject_max_input: String::new(),
            settings_body_lines_input: String::new(),
            settings_prompt_is_custom: false,
            settings_dir: String::new(),
            repo_root: None,
            workspace_root: None,
            branch: None,
            remote_url: None,
            ahead_behind: None,
            nested_repo_detail_path: None,

            status: None,
            pending_action: None,
            confirm: None,
            push_after_commit: false,
            should_quit: false,
            animation_tick: 0,
            animation_stepped_at: Instant::now(),

            generation: None,
            push_job: None,
            checkout_job: None,
            operation_job: None,
            fetch_job: None,
            refresh_job: None,
            refresh_pending: false,
            refresh_pending_diff: false,
            release_status_job: None,
            settings_suggest_job: None,
            commit_log_job: None,
            diff_job: None,
            review_job: None,
            review_assist_job: None,
            review_pr_job: None,
            review_flag_job: None,
            review_chat_job: None,
            workflow_job: None,
            deferred_threads: Vec::new(),

            left_column_width: None,
            column_drag_active: false,
            left_panel_heights: None,
            row_drag_active: None,

            flow_idx: 0,
            flow_scroll_offset: 0,
            flow_confirm: None,
            flow_input: None,
            flow_text: String::new(),

            conflicts: Vec::new(),
            conflict_idx: 0,
            conflict_scroll_offset: 0,
            conflict_log: String::new(),
            conflict_followup: None,

            delete_branch_target: String::new(),
            delete_branch_local: true,
            delete_branch_remote: false,
            delete_branch_remote_available: false,
            delete_branch_force: false,
            delete_branch_field: DeleteBranchField::Local,
            worktree_branch_input: String::new(),
            worktree_base_input: String::new(),
            worktree_path_input: String::new(),
            worktree_field: WorktreeField::Branch,
            worktree_path_edited: false,
            worktree_repo_dir: String::new(),
            branch_view: BranchView::Local,
            nested_repo_branch_view: BranchView::Local,
        }
    }

    /// Advance the animation clock if a step's worth of time has passed. Called
    /// once per frame, so the check is what keeps a spinner at one speed whether
    /// lg is idle or redrawing a session at `SESSION_TICK_MS`.
    pub fn advance_animation(&mut self) {
        let step = Duration::from_millis(crate::config::ANIMATION_STEP_MS);
        let now = Instant::now();
        if now.duration_since(self.animation_stepped_at) >= step {
            self.animation_stepped_at = now;
            self.animation_tick = self.animation_tick.wrapping_add(1);
        }
    }

    /// Replace the main pane's text and keep the line count in step. Scrolling
    /// is bounded by that count, so the two must not drift apart.
    pub fn set_diff_text(&mut self, text: String) {
        self.diff_text = text;
        self.diff_line_count = self.diff_text.lines().count().min(u16::MAX as usize) as u16;
    }

    /// Hand over every running job's worker handle so the caller can wait for
    /// them. Every job field is listed here: one left out is a worker the
    /// process can exit from under.
    pub fn take_job_handles(&mut self) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();
        macro_rules! take {
            ($($job:ident),+ $(,)?) => { $(
                if let Some(job) = self.$job.as_mut() {
                    handles.extend(job.handle_mut().take());
                }
            )+ };
        }
        take!(
            generation,
            push_job,
            checkout_job,
            operation_job,
            fetch_job,
            refresh_job,
            release_status_job,
            settings_suggest_job,
            commit_log_job,
            diff_job,
            review_job,
            review_assist_job,
            review_pr_job,
            review_flag_job,
            review_chat_job,
            workflow_job,
        );
        handles
    }

    /// Whether any background job is in flight. The event loop polls faster
    /// while one is, so its result lands without waiting out a full tick.
    pub fn any_job_running(&self) -> bool {
        self.generation.is_some()
            || self.push_job.is_some()
            || self.checkout_job.is_some()
            || self.operation_job.is_some()
            || self.fetch_job.is_some()
            || self.refresh_job.is_some()
            || self.release_status_job.is_some()
            || self.settings_suggest_job.is_some()
            || self.commit_log_job.is_some()
            || self.diff_job.is_some()
            || self.review_job.is_some()
            || self.review_assist_job.is_some()
            || self.review_pr_job.is_some()
            || self.review_flag_job.is_some()
            || self.review_chat_job.is_some()
            || self.workflow_job.is_some()
    }

    pub fn activity_label(&self) -> Option<&'static str> {
        if self.generation.is_some() {
            Some("generating")
        } else if self.push_job.is_some() {
            Some("pushing")
        } else if self.checkout_job.is_some() {
            Some("checking out")
        } else if let Some(job) = &self.operation_job {
            Some(job.label)
        } else if self.fetch_job.is_some() {
            Some("fetching")
        } else if self.refresh_job.is_some() {
            Some("refreshing")
        } else if self.release_status_job.is_some() {
            Some("checking deployments")
        } else if self.commit_log_job.is_some() {
            Some("loading commits")
        } else if self.diff_job.is_some() {
            Some("loading diff")
        } else if self.review_job.is_some() {
            Some("reviewing")
        } else if self.review_assist_job.is_some() {
            Some("explaining")
        } else if self.review_flag_job.is_some() {
            Some("flagging style")
        } else if self.review_pr_job.is_some() {
            Some("writing PR text")
        } else if self.review_chat_job.is_some() {
            Some("chatting")
        } else if self.workflow_job.is_some() {
            Some("running branch action")
        } else {
            match &self.pending_action {
                Some(PendingAction::GenerateMessage) => Some("starting generator"),
                Some(PendingAction::ReviewAssist(_)) => Some("starting explanation"),
                Some(PendingAction::ReviewPrText) => Some("starting PR text"),
                Some(PendingAction::ReviewStyleFlags) => Some("starting style flag pass"),
                Some(PendingAction::ReviewChat(_)) => Some("starting chat"),
                Some(PendingAction::CopyToClipboard { .. }) => Some("copying"),
                Some(PendingAction::Commit) => Some("committing"),
                Some(PendingAction::StageAllAndCommit) => Some("staging"),
                Some(PendingAction::Push) => Some("starting push"),
                Some(PendingAction::Pull) => Some("starting pull"),
                Some(PendingAction::MergeUpstream) => Some("starting merge"),
                Some(PendingAction::MergeMainAllBranches) => Some("starting branch sync"),
                Some(PendingAction::Flow(_)) => Some("starting branch action"),
                Some(
                    PendingAction::SaveAuthor { .. }
                    | PendingAction::ClearAuthor
                    | PendingAction::SaveSubtreeAuthor { .. }
                    | PendingAction::ClearSubtreeAuthor { .. },
                ) => Some("saving author"),
                Some(PendingAction::SaveSettings { .. } | PendingAction::ClearSettings) => {
                    Some("saving settings")
                }
                Some(PendingAction::EditCommitPrompt) => Some("opening commit prompt"),
                Some(PendingAction::StageAll | PendingAction::StagePath(_)) => Some("staging"),
                Some(PendingAction::UnstageAll | PendingAction::UnstagePath(_)) => {
                    Some("unstaging")
                }
                Some(PendingAction::RollbackPath { .. }) => Some("rolling back"),
                Some(PendingAction::DeletePath { .. }) => Some("deleting"),
                Some(PendingAction::IgnorePath { .. }) => Some("updating gitignore"),
                Some(PendingAction::OpenProject | PendingAction::OpenProjectAt(_)) => {
                    Some("opening project")
                }
                Some(PendingAction::OpenFile(_)) => Some("opening file"),
                Some(PendingAction::DeleteBranch { .. }) => Some("deleting branch"),
                Some(PendingAction::SetBranchUpstream { .. }) => Some("setting upstream"),
                Some(PendingAction::SwitchRepository { .. }) => Some("switching repo"),
                Some(PendingAction::CreateWorktree { .. }) => Some("adding worktree"),
                Some(PendingAction::RemoveWorktree { .. }) => Some("removing worktree"),
                Some(PendingAction::PruneWorktrees) => Some("pruning worktrees"),
                Some(PendingAction::StartSession { .. }) => Some("starting session"),
                Some(PendingAction::Quit) => Some("quitting"),
                None => None,
            }
        }
    }

    pub fn file_counts(&self) -> (usize, usize, usize) {
        self.files
            .iter()
            .fold((0, 0, 0), |(staged, unstaged, untracked), f| {
                (
                    staged + usize::from(f.x != ' ' && f.x != '?'),
                    unstaged + usize::from(f.y != ' ' && f.y != '?'),
                    untracked + usize::from(f.x == '?' || f.y == '?'),
                )
            })
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        build_tree_rows(&self.files, &self.collapsed_dirs)
    }

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

    /// The branch that deploys `env` in this checkout, if it has one. Falls back
    /// to the local branch list so the panels are right before the first
    /// refresh snapshot lands.
    pub fn release_branch(&self, env: ReleaseEnv) -> Option<&str> {
        if let Some(branch) = self.release_branches.branch(env) {
            return Some(branch);
        }
        match env {
            ReleaseEnv::Dev => DEV_BRANCH_NAMES
                .into_iter()
                .find(|name| self.branch_exists(name)),
            ReleaseEnv::Test => self.branch_exists(BRANCH_TEST).then_some(BRANCH_TEST),
        }
    }

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

    /// The session the main pane is showing, if it is showing one and that
    /// session still exists.
    pub fn session_view(&self) -> Option<crate::session::SessionId> {
        match self.main_view {
            MainView::Session(id) if self.sessions.get(id).is_some() => Some(id),
            _ => None,
        }
    }

    /// Whether keys typed right now belong to a session rather than to lg.
    pub fn session_input_active(&self) -> bool {
        self.modal == Modal::None
            && self.focus == Pane::Main
            && self.session_capture
            && self.session_view().is_some()
    }

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

    /// Swap between the git view and the session view. Workspace mode starts on
    /// the tree so the checkouts are there to pick from, and going back to git
    /// mode leaves the sessions running.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            AppMode::Git => AppMode::Workspace,
            AppMode::Workspace => AppMode::Git,
        };
        if self.mode == AppMode::Workspace {
            // A session already on screen keeps the focus; otherwise the tree
            // is the only thing worth pointing at.
            if self.session_view().is_none() {
                self.focus = Pane::Status;
            }
        } else if self.focus != Pane::Main {
            self.focus = Pane::Status;
        }
    }

    /// Point the focus at a pane, unless that pane is not on screen in this
    /// mode — the numbered focus keys then do nothing rather than focusing
    /// something invisible.
    pub fn focus_pane(&mut self, pane: Pane) -> bool {
        if !self.git_panes_visible() && !matches!(pane, Pane::Status | Pane::Main) {
            return false;
        }
        self.focus = pane;
        true
    }

    /// Whether the git panes are on screen at all.
    pub fn git_panes_visible(&self) -> bool {
        self.mode == AppMode::Git
    }

    /// Show a session in the main pane, and hand it the keyboard.
    pub fn show_session(&mut self, id: crate::session::SessionId) {
        self.sessions.focus(id);
        self.main_view = MainView::Session(id);
        self.focus = Pane::Main;
    }

    /// Go back to the diff, releasing the keyboard.
    pub fn show_diff(&mut self) {
        self.main_view = MainView::Diff;
        self.session_capture = false;
    }

    /// Give the main pane back to the diff for a newly selected file, branch or
    /// commit. A session drawn there goes to the background: it keeps running,
    /// and Ctrl-N returns to it. Returns whether one was backgrounded.
    pub fn background_session_for_diff(&mut self) -> bool {
        if self.session_view().is_none() {
            return false;
        }
        self.show_diff();
        self.set_status(
            "session in the background \u{2014} Ctrl-N returns to it",
            false,
        );
        true
    }

    /// Whether this repository is checked out in more than one place, which is
    /// what gives the repository tree worktree rows to show.
    pub fn has_linked_worktrees(&self) -> bool {
        self.worktrees.len() > 1
    }

    pub fn environments_visible(&self) -> bool {
        self.flow_available()
            || !self.nested_repositories.is_empty()
            || self.has_linked_worktrees()
            || match (self.workspace_root.as_deref(), self.repo_root.as_deref()) {
                (Some(workspace), Some(repo)) => Path::new(workspace) != Path::new(repo),
                _ => false,
            }
    }

    pub fn branch_actions_available(&self) -> bool {
        self.branch.is_some() || !self.branches.is_empty()
    }

    pub fn merge_main_available(&self) -> bool {
        let Some(branch) = self.branch.as_deref() else {
            return false;
        };
        match branch {
            BRANCH_MAIN => false,
            _ if is_deploy_branch_name(branch) => {
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

    pub fn start_generation(&mut self, rx: Receiver<GenMsg>, handle: JoinHandle<()>) {
        self.generation = Some(Generation {
            rx,
            handle: Some(handle),
            output: String::new(),
            spinner: 0,
        });
    }

    pub fn defer_thread_join(&mut self, handle: Option<JoinHandle<()>>) {
        if let Some(handle) = handle {
            self.deferred_threads.push(handle);
        }
    }

    pub fn reap_deferred_threads(&mut self) {
        let mut i = 0;
        while i < self.deferred_threads.len() {
            if self.deferred_threads[i].is_finished() {
                let handle = self.deferred_threads.swap_remove(i);
                let _ = handle.join();
            } else {
                i += 1;
            }
        }
    }

    pub fn take_deferred_threads(&mut self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.deferred_threads)
    }

    /// Cancel any in-flight LLM work and report what was stopped.
    ///
    /// Dropping a job drops its receiver; the streaming loop in `llm` bails out
    /// as soon as a send fails, so this really does stop the work rather than
    /// just hiding it. The assisted-review builder is not a stream, so it is
    /// detached and its result discarded.
    pub fn cancel_llm_jobs(&mut self) -> Option<&'static str> {
        let mut cancelled = None;

        if let Some(mut job) = self.review_chat_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("review chat cancelled");
        }
        if let Some(mut job) = self.review_flag_job.take() {
            self.defer_thread_join(job.handle.take());
            self.review_flag_active_path = None;
            cancelled = Some("style flag pass cancelled");
        }
        if let Some(mut job) = self.review_pr_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("PR text cancelled");
        }
        if let Some(mut job) = self.review_assist_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("explanation cancelled");
        }
        if let Some(mut job) = self.review_job.take() {
            self.defer_thread_join(job.handle.take());
            // Otherwise the pane keeps claiming it is still building the review.
            self.set_diff_text("review cancelled".to_string());
            cancelled = Some("review cancelled");
        }
        if self.generation.is_some() {
            self.cancel_generation();
            cancelled = Some("generation cancelled");
        }

        cancelled
    }

    /// True when [`cancel_llm_jobs`] would stop something.
    pub fn llm_job_running(&self) -> bool {
        self.review_job.is_some()
            || self.review_assist_job.is_some()
            || self.review_pr_job.is_some()
            || self.review_flag_job.is_some()
            || self.review_chat_job.is_some()
            || self.generation.is_some()
    }

    pub fn cancel_generation(&mut self) {
        if let Some(mut generation) = self.generation.take() {
            self.defer_thread_join(generation.handle.take());
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(StatusMsg {
            text: text.into(),
            is_error,
            at: Utc::now(),
        });
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
        });
        self.modal = Modal::ConfirmDestructive;
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

    /// Clamp per-pane indices to their vec lengths; 0 when empty.
    pub fn clamp(&mut self) {
        let clamp_idx = |idx: &mut usize, len: usize| *idx = clamp_index(*idx, len).unwrap_or(0);
        // files_idx indexes into the virtual tree-rows list (always >=1: AllChanges + descendants).
        let tree_len = self.tree_rows().len().max(1);
        self.files_idx = clamp_index(self.files_idx, tree_len).unwrap_or(0);
        clamp_idx(&mut self.branches_idx, self.branches.len());
        let remote_len = self.visible_remote_branches().count();
        clamp_idx(&mut self.remote_branches_idx, remote_len);
        clamp_idx(
            &mut self.nested_repositories_idx,
            self.nested_repositories.len(),
        );
        // The repository tree is built by the panel, and now carries worktree
        // rows as well as repositories and their branches; ask it for the count
        // rather than keeping a second copy of the arithmetic here.
        let tree_len = crate::panel::environments::nested_repo_tree_len(self);
        clamp_idx(&mut self.nested_repo_tree_idx, tree_len);
        clamp_idx(
            &mut self.nested_repo_branches_idx,
            self.nested_repo_branches.len(),
        );
        let nested_remote_len = self.visible_nested_repo_remote_branches().count();
        clamp_idx(&mut self.nested_repo_remote_branches_idx, nested_remote_len);
        clamp_idx(&mut self.commits_idx, self.commits.len());
        if self
            .commits
            .get(self.commits_idx)
            .is_some_and(crate::git::Commit::is_graph_row)
        {
            self.commits_idx = self
                .commits
                .iter()
                .enumerate()
                .find_map(|(idx, commit)| (!commit.is_graph_row()).then_some(idx))
                .unwrap_or(0);
        }
        let flow_len = usize::from(self.branch_actions_available()) * FlowAction::ALL.len();
        clamp_idx(&mut self.flow_idx, flow_len);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_animation_clock_ignores_extra_frames() {
        let mut state = AppState::new();
        let start = state.animation_tick;
        // A session on screen redraws at SESSION_TICK_MS; a burst of those
        // frames is still well inside one animation step.
        for _ in 0..200 {
            state.advance_animation();
        }
        assert_eq!(
            state.animation_tick, start,
            "animation speed must not follow the frame rate"
        );
    }

    #[test]
    fn replacing_the_diff_text_keeps_the_line_count_in_step() {
        let mut state = AppState::new();
        state.set_diff_text("one\ntwo\nthree".to_string());
        assert_eq!(state.diff_line_count, 3);

        state.set_diff_text("just one".to_string());
        assert_eq!(
            state.diff_line_count, 1,
            "a shorter text must not keep the old bound"
        );
    }

    #[test]
    fn cancelling_a_review_resizes_the_pane_to_its_notice() {
        let mut state = AppState::new();
        state.set_diff_text("a long review\n".repeat(50));
        let (_tx, rx) = std::sync::mpsc::channel();
        state.review_job = Some(ReviewJob {
            rx,
            handle: None,
            spinner: 0,
        });

        assert_eq!(state.cancel_llm_jobs(), Some("review cancelled"));
        assert_eq!(
            state.diff_line_count, 1,
            "the notice is one line, so scrolling must stop there"
        );
    }

    #[test]
    fn the_animation_clock_advances_once_a_step_has_passed() {
        let mut state = AppState::new();
        let start = state.animation_tick;
        state.animation_stepped_at =
            Instant::now() - Duration::from_millis(crate::config::ANIMATION_STEP_MS);
        state.advance_animation();
        assert_eq!(state.animation_tick, start.wrapping_add(1));
    }
}
