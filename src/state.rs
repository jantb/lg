//! Everything the running app knows, and the reads and writes over it.

use std::collections::HashSet;
use std::thread::JoinHandle;
use std::time::Instant;

use crate::git::{
    Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo, ReleaseBranches, RemoteBranch,
    Worktree,
};

mod activity;
mod branches;
mod conflict;
mod filter;
mod flow;
mod forms;
mod jobs;
mod merge_editor;
mod modal;
mod review;
mod tree;
mod view;

pub use activity::*;
pub use branches::*;
pub use conflict::ConflictState;
pub use filter::{ListFilter, visible_position};
pub use flow::*;
pub use forms::{AuthorForm, DeleteBranchForm, StashForm, WorktreeForm};
pub use jobs::*;
pub use merge_editor::*;
pub use modal::*;
pub use review::{ReviewRenderCache, ReviewState};
pub use tree::{TreeKind, TreeRow, build_tree_rows};
pub use view::*;

/// Where a list's selection is, and the first row it shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListCursor {
    pub idx: usize,
    pub scroll: usize,
}

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
    /// The modal the help overlay goes back to when it closes, for help
    /// opened from inside one rather than from the panes.
    pub help_return: Modal,
    pub help_offset: u16,
    /// What the help overlay's rows are narrowed to, and whether keys are
    /// being typed into it.
    pub help_filter: crate::panel::text_input::TextInput,
    pub help_filtering: bool,

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

    pub files_list: ListCursor,
    pub branches_list: ListCursor,
    pub remote_branches_list: ListCursor,
    pub nested_repositories_list: ListCursor,
    pub nested_repo_tree_idx: usize,
    pub nested_repo_branches_list: ListCursor,
    pub nested_repo_remote_branches_list: ListCursor,
    pub commits_list: ListCursor,

    pub collapsed_dirs: HashSet<String>,
    /// What `/` has narrowed each list to.
    pub files_filter: ListFilter,
    pub branches_filter: ListFilter,
    pub commits_filter: ListFilter,

    /// Terminal sessions lg is keeping alive: one agent of each kind per
    /// checkout, and any number of shells.
    pub sessions: crate::session::Sessions,
    pub main_view: MainView,
    /// Keys go to the focused session instead of to lg.
    pub session_capture: bool,

    pub diff_text: String,
    /// Bumped whenever `diff_text` is replaced, so caches keyed on the text
    /// can tell a new text from the old one without hashing it.
    pub diff_text_version: u64,
    /// The row count of the last diff rendered, with what it was rendered
    /// from. Counting rows means highlighting and wrapping the whole diff,
    /// and it is asked for several times a frame.
    pub diff_row_count_cache: std::cell::Cell<Option<(DiffRowCountKey, usize)>>,
    /// The diff pane's highlighted rows, kept until the text, width or view
    /// changes, so a frame only copies the rows it shows instead of
    /// highlighting the whole diff again.
    pub diff_render_cache: crate::state::DiffRenderCache,
    /// The file pane's rows with the files and folds they were built from.
    /// Several things ask for the rows each frame and on every key, and
    /// building them sorts and nests every path.
    pub tree_rows_cache: std::cell::RefCell<Option<crate::state::TreeRowsCache>>,
    /// The commit graph's lanes with the commits they were drawn for.
    pub pipe_sets_cache: std::cell::RefCell<Option<crate::state::PipeSetsCache>>,
    /// The review pane's drawn nodes.
    pub review_render_cache: std::cell::RefCell<crate::state::ReviewRenderCache>,
    /// Bumped each time the checkout's files are read again, so whatever was
    /// read off disk for them — the review's source context — is read afresh.
    pub files_generation: u64,
    pub diff_offset: u16,
    pub diff_source: DiffSource,
    pub diff_view_mode: DiffViewMode,
    pub diff_line_count: u16,
    pub diff_viewport_height: u16,
    pub diff_viewport_width: u16,
    /// The hunk the diff pane's hunk keys act on.
    pub diff_hunk: HunkCursor,
    /// The commit of the branch log the log view's keys act on, counted from
    /// the top of the log.
    pub log_commit: usize,
    /// The hunks of `diff_text`, read once per text.
    pub diff_hunks_cache:
        std::cell::RefCell<Option<(u64, std::sync::Arc<Vec<crate::git::hunk::ShownHunk>>)>>,
    pub review: ReviewState,

    pub commit_message: String,
    pub commit_cursor: usize,
    pub commit_scroll_offset: usize,
    /// First visible row of the staged-files list beside the commit editor.
    pub commit_files_scroll: usize,
    /// The commit modal replaces the last commit rather than adding one.
    pub commit_amend: bool,
    /// The message that was being written before amend put the last commit's
    /// in its place, for turning amend off again.
    pub commit_amend_draft: Option<String>,
    pub commit_author: String,
    pub author: AuthorForm,
    pub llm_model: String,
    pub llm_model_input: String,
    pub llm_model_idx: usize,
    /// Models the server said it serves, so the picker offers what will
    /// actually answer rather than a list compiled into lg. Empty until the
    /// server has been asked.
    pub llm_model_choices: Vec<String>,
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
    pub settings_review_style_is_custom: bool,
    pub settings_dir: String,
    pub repo_root: Option<String>,
    pub workspace_root: Option<String>,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub ahead_behind: Option<(u32, u32)>,
    pub nested_repo_detail_path: Option<String>,

    pub status: Option<StatusMsg>,
    /// The last error this session, whole, for the details popup `!` opens
    /// after the status bar has cut it short or it has expired.
    pub last_error: Option<StatusMsg>,
    /// First line the details popup shows.
    pub details_offset: u16,
    pub decorative_animations: bool,
    /// Whether the model is asked for anything. Off, the commit message is
    /// typed by hand and conflicts and reviews are left alone.
    pub ai_assist: bool,
    /// The model server could not be reached this session, so opening the
    /// commit modal stops asking it. Ctrl+R still does, and saving the model
    /// settings clears this.
    pub model_server_unreachable: bool,
    pub history_file: Option<std::path::PathBuf>,
    pub status_history: Vec<StatusMsg>,
    pub environment_view: crate::panel::deployment::Environments,
    pub commands: crate::panel::commands::Commands,
    pub settings_hub: crate::panel::settings::Settings,
    pub github: crate::panel::github::GitHub,
    /// The guided review under way, kept while it is not on screen so that
    /// coming back resumes it.
    pub guided: Option<Box<crate::panel::guided::Guided>>,
    pub agent_profiles: Vec<crate::preferences::Agent>,
    pub pending_action: Option<PendingAction>,
    pub confirm: Option<ConfirmPrompt>,
    pub push_after_commit: bool,
    /// A push or pull asked for while a fetch held the git job slot, started
    /// once the fetch is done rather than dropped.
    pub queued_after_fetch: Option<QueuedAfterFetch>,
    pub should_quit: bool,
    /// The animation clock in whole steps of `ANIMATION_STEP_MS`, for things
    /// that move frame by frame: spinners, a travelling marker, a blink.
    pub animation_tick: usize,
    /// The animation clock in milliseconds, for things that move continuously:
    /// a pulsing frame, a fading status line. Animation runs on this clock, not
    /// on the frame rate.
    pub animation_ms: u64,
    /// When the animation clock started.
    animation_started: Instant,

    /// A commit message being generated, or generated and not yet looked at,
    /// per checkout: several can be in flight at once, each listed under its
    /// own checkout in the workspace tree. The modal shows the one belonging
    /// to the repository on screen.
    pub commit_drafts: Vec<CommitDraft>,
    /// Finished messages put away from the tree with `x`, by checkout, for
    /// `c` to bring back. A row for each would be the clutter `x` removed.
    pub set_aside_messages: Vec<(String, String)>,
    pub push_job: Option<PushJob>,
    pub checkout_job: Option<CheckoutJob>,
    pub operation_job: Option<OperationJob>,
    pub fetch_job: Option<FetchJob>,
    pub refresh_job: Option<RefreshJob>,
    /// A refresh asked for while one was running, and how much it has to read.
    pub refresh_pending: Option<RefreshScope>,
    pub refresh_pending_diff: bool,
    pub release_status_job: Option<ReleaseStatusJob>,
    pub nested_detail_job: Option<NestedDetailJob>,
    pub settings_suggest_job: Option<SettingsSuggestJob>,
    pub commit_log_job: Option<CommitLogJob>,
    pub diff_job: Option<DiffJob>,
    pub review_job: Option<ReviewJob>,
    pub review_assist_job: Option<ReviewAssistJob>,
    pub review_pr_job: Option<ReviewAssistJob>,
    pub review_agent_job: Option<ReviewAgentJob>,
    pub review_flag_job: Option<ReviewFlagJob>,
    pub review_chat_job: Option<ReviewChatJob>,
    pub conflict_resolve_job: Option<ConflictResolveJob>,
    pub workflow_job: Option<WorkflowJob>,
    pub deferred_threads: Vec<JoinHandle<()>>,

    pub left_column_width: Option<u16>,
    pub column_drag_active: bool,
    /// The guided review's step list as wide as it was dragged, kept for
    /// the next review in this session.
    pub guided_list_width: Option<u16>,
    pub guided_list_drag_active: bool,
    pub left_panel_heights: Option<crate::ui::LeftPanelHeights>,
    pub row_drag_active: Option<(usize, usize)>,
    /// Text being selected with the mouse, or just selected and not yet copied.
    pub selection: Option<crate::ui::TextSelection>,

    pub flow_list: ListCursor,
    pub flow_confirm: Option<FlowAction>,
    /// What the action waiting on `flow_confirm` would touch, worked out when
    /// the prompt opened — for cleaning orphans, the branches it would delete.
    pub flow_confirm_detail: Vec<String>,
    pub flow_input: Option<FlowAction>,
    pub flow_text: crate::panel::text_input::TextInput,

    /// Which agent `s` starts and which one a conflict is handed to. It
    /// follows the last one picked, so running the same agent again is the
    /// same two keystrokes rather than a hunt down the list.
    pub preferred_agent: crate::session::SessionKind,
    pub agent_pick_idx: usize,
    pub agent_pick_sandboxed: bool,
    /// The highlighted entry of the repository action menu.
    pub repo_menu_idx: usize,

    pub conflict: ConflictState,

    pub delete_branch: DeleteBranchForm,
    pub worktree_form: WorktreeForm,
    pub stash: StashForm,
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
            help_return: Modal::None,
            help_offset: 0,
            help_filter: Default::default(),
            help_filtering: false,

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

            files_list: ListCursor::default(),
            branches_list: ListCursor::default(),
            remote_branches_list: ListCursor::default(),
            nested_repositories_list: ListCursor::default(),
            nested_repo_tree_idx: 0,
            nested_repo_branches_list: ListCursor::default(),
            nested_repo_remote_branches_list: ListCursor::default(),
            commits_list: ListCursor::default(),

            collapsed_dirs: HashSet::new(),
            files_filter: ListFilter::default(),
            branches_filter: ListFilter::default(),
            commits_filter: ListFilter::default(),

            sessions: crate::session::Sessions::new(),
            main_view: MainView::Diff,
            session_capture: false,

            diff_text: String::new(),
            diff_text_version: 0,
            diff_row_count_cache: std::cell::Cell::new(None),
            diff_render_cache: Default::default(),
            tree_rows_cache: Default::default(),
            pipe_sets_cache: Default::default(),
            review_render_cache: Default::default(),
            files_generation: 0,
            diff_offset: 0,
            diff_source: DiffSource::None,
            diff_view_mode: DiffViewMode::SideBySide,
            diff_line_count: 0,
            diff_viewport_height: 0,
            diff_viewport_width: 0,
            diff_hunk: HunkCursor::default(),
            log_commit: 0,
            diff_hunks_cache: Default::default(),
            review: ReviewState::default(),

            commit_message: String::new(),
            commit_cursor: 0,
            commit_scroll_offset: 0,
            commit_files_scroll: 0,
            commit_amend: false,
            commit_amend_draft: None,
            commit_author: String::new(),
            author: AuthorForm::default(),
            llm_model: crate::llm::current_model(),
            llm_model_input: String::new(),
            llm_model_idx: 0,
            llm_model_choices: Vec::new(),
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
            settings_review_style_is_custom: false,
            settings_dir: String::new(),
            repo_root: None,
            workspace_root: None,
            branch: None,
            remote_url: None,
            ahead_behind: None,
            nested_repo_detail_path: None,

            status: None,
            last_error: None,
            details_offset: 0,
            decorative_animations: true,
            ai_assist: crate::preferences::ai_enabled(),
            model_server_unreachable: false,
            history_file: None,
            status_history: Vec::new(),
            environment_view: Default::default(),
            commands: Default::default(),
            settings_hub: Default::default(),
            github: Default::default(),
            guided: None,
            agent_profiles: Vec::new(),
            pending_action: None,
            confirm: None,
            push_after_commit: false,
            queued_after_fetch: None,
            should_quit: false,
            animation_tick: 0,
            animation_ms: 0,
            animation_started: Instant::now(),

            commit_drafts: Vec::new(),
            set_aside_messages: Vec::new(),
            push_job: None,
            checkout_job: None,
            operation_job: None,
            fetch_job: None,
            refresh_job: None,
            refresh_pending: None,
            refresh_pending_diff: false,
            release_status_job: None,
            nested_detail_job: None,
            settings_suggest_job: None,
            commit_log_job: None,
            diff_job: None,
            review_job: None,
            review_assist_job: None,
            review_pr_job: None,
            review_agent_job: None,
            review_flag_job: None,
            review_chat_job: None,
            conflict_resolve_job: None,
            workflow_job: None,
            deferred_threads: Vec::new(),

            left_column_width: None,
            column_drag_active: false,
            guided_list_width: None,
            guided_list_drag_active: false,
            left_panel_heights: None,
            row_drag_active: None,
            selection: None,

            flow_list: ListCursor::default(),
            flow_confirm: None,
            flow_confirm_detail: Vec::new(),
            flow_input: None,
            flow_text: Default::default(),

            preferred_agent: crate::session::SessionKind::Claude,
            agent_pick_idx: 0,
            agent_pick_sandboxed: true,
            repo_menu_idx: 0,

            conflict: ConflictState::default(),

            delete_branch: DeleteBranchForm::default(),
            worktree_form: WorktreeForm::default(),
            stash: StashForm::default(),
            branch_view: BranchView::Local,
            nested_repo_branch_view: BranchView::Local,
        }
    }

    /// Clamp per-pane indices to their vec lengths; 0 when empty.
    pub fn clamp(&mut self) {
        let clamp_idx = |idx: &mut usize, len: usize| *idx = clamp_index(*idx, len).unwrap_or(0);
        // files_idx indexes into the virtual tree-rows list (always >=1: AllChanges + descendants).
        let tree_len = self.tree_rows().len().max(1);
        self.files_list.idx = clamp_index(self.files_list.idx, tree_len).unwrap_or(0);
        clamp_idx(&mut self.branches_list.idx, self.branches.len());
        let remote_len = self.visible_remote_branches().count();
        clamp_idx(&mut self.remote_branches_list.idx, remote_len);
        clamp_idx(
            &mut self.nested_repositories_list.idx,
            self.nested_repositories.len(),
        );
        // The repository tree is built by the panel, and now carries worktree
        // rows as well as repositories and their branches; ask it for the count
        // rather than keeping a second copy of the arithmetic here.
        let tree_len = crate::panel::environments::nested_repo_tree_len(self);
        clamp_idx(&mut self.nested_repo_tree_idx, tree_len);
        clamp_idx(
            &mut self.nested_repo_branches_list.idx,
            self.nested_repo_branches.len(),
        );
        let nested_remote_len = self.visible_nested_repo_remote_branches().count();
        clamp_idx(
            &mut self.nested_repo_remote_branches_list.idx,
            nested_remote_len,
        );
        clamp_idx(&mut self.commits_list.idx, self.commits.len());
        if self
            .commits
            .get(self.commits_list.idx)
            .is_some_and(crate::git::Commit::is_graph_row)
        {
            self.commits_list.idx = self
                .commits
                .iter()
                .enumerate()
                .find_map(|(idx, commit)| (!commit.is_graph_row()).then_some(idx))
                .unwrap_or(0);
        }
        let flow_len = usize::from(self.branch_actions_available()) * FlowAction::ALL.len();
        clamp_idx(&mut self.flow_list.idx, flow_len);
        self.clamp_filtered();
    }
}
