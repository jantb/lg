//! Everything the running app knows, and the reads and writes over it.

use std::collections::{HashMap, HashSet};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::git::{
    AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo, ReleaseBranches,
    RemoteBranch, Worktree,
};

mod activity;
mod branches;
mod flow;
mod jobs;
mod modal;
mod tree;
mod view;

pub use activity::*;
pub use branches::*;
pub use flow::*;
pub use jobs::*;
pub use modal::*;
pub use tree::{TreeKind, TreeRow, build_tree_rows};
pub use view::*;

pub fn clamp_index(idx: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(idx.min(len - 1))
    }
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
    pub conflict_resolve_job: Option<ConflictResolveJob>,
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

    /// Which agent `s` starts and which one a conflict is handed to. It
    /// follows the last one picked, so running the same agent again is the
    /// same two keystrokes rather than a hunt down the list.
    pub preferred_agent: crate::session::SessionKind,
    pub agent_pick_idx: usize,
    pub agent_pick_sandboxed: bool,

    pub conflicts: Vec<String>,
    pub conflict_idx: usize,
    pub conflict_scroll_offset: usize,
    pub conflict_log: String,
    pub conflict_followup: Option<ConflictFollowup>,
    /// Files the local model settled in this conflict. They are still
    /// conflicted as far as git is concerned — nothing is staged until `v` —
    /// so this is what tells the panel which ones are waiting to be read
    /// rather than waiting to be resolved.
    pub conflict_resolved: std::collections::HashSet<String>,

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

impl Default for AppState {
    fn default() -> Self {
        Self::new()
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
            conflict_resolve_job: None,
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

            preferred_agent: crate::session::SessionKind::Claude,
            agent_pick_idx: 0,
            agent_pick_sandboxed: true,

            conflicts: Vec::new(),
            conflict_idx: 0,
            conflict_scroll_offset: 0,
            conflict_log: String::new(),
            conflict_followup: None,
            conflict_resolved: std::collections::HashSet::new(),

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
