use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::Receiver;

use chrono::{DateTime, Utc};

use crate::git::{Branch, Commit, FileEntry};

#[derive(Debug)]
pub enum GenMsg {
    Thinking(String),
    Output(String),
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct Generation {
    pub rx: Receiver<GenMsg>,
    pub output: String,
    pub spinner: usize,
}

pub const SPINNER_FRAMES: &[&str] = &[
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

#[derive(Debug)]
pub enum PushMsg {
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct PushJob {
    pub rx: Receiver<PushMsg>,
    pub spinner: usize,
    pub branch: String,
    pub remote: String,
}

#[derive(Debug)]
pub enum CheckoutMsg {
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct CheckoutJob {
    pub rx: Receiver<CheckoutMsg>,
    pub spinner: usize,
    pub branch: String,
}

#[derive(Debug)]
pub enum WorkflowMsg {
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct WorkflowJob {
    pub rx: Receiver<WorkflowMsg>,
    pub spinner: usize,
    pub label: String,
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
    Push,
    Help,
    Flow,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    None,
    All,
    File(String),   // path
    Folder(String), // folder prefix (no trailing slash)
    Commit(String), // sha
    Branch(String), // branch name
}

#[derive(Debug, Clone)]
pub struct StatusMsg {
    pub text: String,
    pub is_error: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    GenerateMessage,
    Commit,
    Push,
}

// ── File tree ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeKind {
    AllChanges,
    Folder {
        expanded: bool,
        total: usize,
        staged: usize,
    },
    File {
        entry_idx: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub kind: TreeKind,
    pub depth: u16,
    pub path: String,
    pub label: String,
}

#[derive(Default, Debug)]
struct DirNode {
    subdirs: BTreeMap<String, DirNode>,
    files: Vec<usize>,
}

fn count_descendants(node: &DirNode, files: &[FileEntry]) -> (usize, usize) {
    let mut total = 0usize;
    let mut staged = 0usize;
    for &idx in &node.files {
        total += 1;
        let fe = &files[idx];
        if fe.x != ' ' && fe.x != '?' {
            staged += 1;
        }
    }
    for child in node.subdirs.values() {
        let (t, s) = count_descendants(child, files);
        total += t;
        staged += s;
    }
    (total, staged)
}

fn emit_rows(
    node: &DirNode,
    prefix: &str,
    depth: u16,
    files: &[FileEntry],
    collapsed: &HashSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    enum Child<'a> {
        Dir(&'a String, &'a DirNode),
        File(usize),
    }

    let mut children: Vec<Child> = node.subdirs.iter().map(|(n, c)| Child::Dir(n, c)).collect();
    for &idx in &node.files {
        children.push(Child::File(idx));
    }
    children.sort_by_cached_key(|c| match c {
        Child::Dir(name, _) => name.to_ascii_lowercase(),
        Child::File(idx) => {
            let p = &files[*idx].path;
            p.rsplit_once('/')
                .map(|(_, n)| n)
                .unwrap_or(p)
                .to_ascii_lowercase()
        }
    });

    for c in children {
        match c {
            Child::Dir(name, child) => {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                let (total, staged) = count_descendants(child, files);
                let expanded = !collapsed.contains(&path);
                rows.push(TreeRow {
                    kind: TreeKind::Folder {
                        expanded,
                        total,
                        staged,
                    },
                    depth,
                    path: path.clone(),
                    label: name.clone(),
                });
                if expanded {
                    emit_rows(child, &path, depth + 1, files, collapsed, rows);
                }
            }
            Child::File(idx) => {
                let fe = &files[idx];
                let label = fe
                    .path
                    .rsplit_once('/')
                    .map(|(_, n)| n)
                    .unwrap_or(&fe.path)
                    .to_string();
                rows.push(TreeRow {
                    kind: TreeKind::File { entry_idx: idx },
                    depth,
                    path: fe.path.clone(),
                    label,
                });
            }
        }
    }
}

pub fn build_tree_rows(files: &[FileEntry], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = DirNode::default();
    for (i, fe) in files.iter().enumerate() {
        let mut node = &mut root;
        let parts: Vec<&str> = fe.path.split('/').collect();
        let last = parts.len().saturating_sub(1);
        for seg in &parts[..last] {
            node = node.subdirs.entry((*seg).to_string()).or_default();
        }
        node.files.push(i);
    }
    let mut rows = vec![TreeRow {
        kind: TreeKind::AllChanges,
        depth: 0,
        path: String::new(),
        label: "(all changes)".to_string(),
    }];
    emit_rows(&root, "", 0, files, collapsed, &mut rows);
    rows
}

pub struct AppState {
    pub focus: Pane,
    pub modal: Modal,
    pub prev_focus: Pane,

    pub files: Vec<FileEntry>,
    pub branches: Vec<Branch>,
    pub commits: Vec<Commit>,
    pub unpushed_shas: HashSet<String>,

    pub files_idx: usize,
    pub branches_idx: usize,
    pub commits_idx: usize,

    pub collapsed_dirs: HashSet<String>,

    pub diff_text: String,
    pub diff_offset: u16,
    pub diff_source: DiffSource,
    pub diff_line_count: u16,
    pub diff_viewport_height: u16,

    pub commit_message: String,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub ahead_behind: Option<(u32, u32)>,

    pub status: Option<StatusMsg>,
    pub pending_action: Option<PendingAction>,
    pub should_quit: bool,
    pub animation_tick: usize,

    pub generation: Option<Generation>,
    pub push_job: Option<PushJob>,
    pub checkout_job: Option<CheckoutJob>,
    pub workflow_job: Option<WorkflowJob>,

    pub flow_idx: usize,
    pub flow_confirm: Option<FlowAction>,
    pub flow_input: Option<FlowAction>,
    pub flow_text: String,

    pub conflicts: Vec<String>,
    pub conflict_idx: usize,
    pub conflict_log: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowAction {
    MergeMain,
    ReleaseDev,
    ReleaseTest,
    ResetDev,
    ResetTest,
    NewFeature,
    CleanOrphans,
}

impl FlowAction {
    pub const ALL: [Self; 7] = [
        Self::MergeMain,
        Self::ReleaseDev,
        Self::ReleaseTest,
        Self::ResetDev,
        Self::ResetTest,
        Self::NewFeature,
        Self::CleanOrphans,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::MergeMain => "Merge origin/main into current branch",
            Self::ReleaseDev => "Release current branch into develop",
            Self::ReleaseTest => "Release current branch into release/next",
            Self::ResetDev => "Reset develop from origin/main",
            Self::ResetTest => "Reset release/next from origin/main",
            Self::NewFeature => "Start new feature from origin/main",
            Self::CleanOrphans => "Clean local branches without upstream",
        }
    }

    pub fn needs_confirmation(self) -> bool {
        !matches!(self, Self::NewFeature)
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            focus: Pane::Files,
            modal: Modal::None,
            prev_focus: Pane::Files,

            files: Vec::new(),
            branches: Vec::new(),
            commits: Vec::new(),
            unpushed_shas: HashSet::new(),

            files_idx: 0,
            branches_idx: 0,
            commits_idx: 0,

            collapsed_dirs: HashSet::new(),

            diff_text: String::new(),
            diff_offset: 0,
            diff_source: DiffSource::None,
            diff_line_count: 0,
            diff_viewport_height: 0,

            commit_message: String::new(),
            branch: None,
            remote_url: None,
            ahead_behind: None,

            status: None,
            pending_action: None,
            should_quit: false,
            animation_tick: 0,

            generation: None,
            push_job: None,
            checkout_job: None,
            workflow_job: None,

            flow_idx: 0,
            flow_confirm: None,
            flow_input: None,
            flow_text: String::new(),

            conflicts: Vec::new(),
            conflict_idx: 0,
            conflict_log: String::new(),
        }
    }

    pub fn advance_animation(&mut self) {
        self.animation_tick = self.animation_tick.wrapping_add(1);
    }

    pub fn activity_label(&self) -> Option<&'static str> {
        if self.generation.is_some() {
            Some("generating")
        } else if self.push_job.is_some() {
            Some("pushing")
        } else if self.checkout_job.is_some() {
            Some("checking out")
        } else if self.workflow_job.is_some() {
            Some("running workflow")
        } else {
            match self.pending_action {
                Some(PendingAction::GenerateMessage) => Some("starting generator"),
                Some(PendingAction::Commit) => Some("committing"),
                Some(PendingAction::Push) => Some("starting push"),
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

    pub fn start_generation(&mut self, rx: Receiver<GenMsg>) {
        self.generation = Some(Generation {
            rx,
            output: String::new(),
            spinner: 0,
        });
    }

    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(StatusMsg {
            text: text.into(),
            is_error,
            at: Utc::now(),
        });
    }

    pub fn open_commit_modal(&mut self) {
        self.modal = Modal::Commit;
        if self.commit_message.is_empty() && self.generation.is_none() {
            self.set_status("generating\u{2026}", false);
            self.pending_action = Some(PendingAction::GenerateMessage);
        }
    }

    /// Clamp per-pane indices to their vec lengths; 0 when empty.
    pub fn clamp(&mut self) {
        let clamp_idx = |idx: &mut usize, len: usize| {
            if len == 0 {
                *idx = 0;
            } else if *idx >= len {
                *idx = len - 1;
            }
        };
        // files_idx indexes into the virtual tree-rows list (always >=1: AllChanges + descendants).
        let tree_len = self.tree_rows().len().max(1);
        if self.files_idx >= tree_len {
            self.files_idx = tree_len - 1;
        }
        clamp_idx(&mut self.branches_idx, self.branches.len());
        clamp_idx(&mut self.commits_idx, self.commits.len());
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

    fn fe(path: &str, x: char, y: char) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            x,
            y,
        }
    }

    #[test]
    fn tree_flat_files_emit_all_plus_files() {
        let files = vec![fe("a.rs", ' ', 'M'), fe("b.rs", 'A', ' ')];
        let rows = build_tree_rows(&files, &HashSet::new());
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].kind, TreeKind::AllChanges));
        assert!(matches!(rows[1].kind, TreeKind::File { entry_idx: 0 }));
        assert!(matches!(rows[2].kind, TreeKind::File { entry_idx: 1 }));
    }

    #[test]
    fn tree_groups_files_under_folders_when_expanded() {
        let files = vec![
            fe("src/lib.rs", 'M', ' '),
            fe("src/util/mod.rs", 'A', ' '),
            fe("README.md", ' ', 'M'),
        ];
        let rows = build_tree_rows(&files, &HashSet::new());
        // Interleaved alphabetical at each depth:
        //   root: README.md ('r') < src/ ('s')
        //   src/: lib.rs ('l') < util/ ('u')
        assert!(matches!(rows[0].kind, TreeKind::AllChanges));
        assert_eq!(rows[1].path, "README.md");
        assert_eq!(rows[2].path, "src");
        match rows[2].kind {
            TreeKind::Folder {
                expanded,
                total,
                staged,
            } => {
                assert!(expanded);
                assert_eq!(total, 2);
                assert_eq!(staged, 2);
            }
            _ => panic!("expected folder, got {:?}", rows[2].kind),
        }
        assert_eq!(rows[3].path, "src/lib.rs");
        assert_eq!(rows[4].path, "src/util");
        assert_eq!(rows[5].path, "src/util/mod.rs");
    }

    #[test]
    fn tree_collapsed_folder_hides_children() {
        let files = vec![fe("src/lib.rs", 'M', ' '), fe("src/mod.rs", 'A', ' ')];
        let mut collapsed = HashSet::new();
        collapsed.insert("src".to_string());
        let rows = build_tree_rows(&files, &collapsed);
        // AllChanges + folder "src" only
        assert_eq!(rows.len(), 2);
        match rows[1].kind {
            TreeKind::Folder {
                expanded,
                total,
                staged,
            } => {
                assert!(!expanded);
                assert_eq!(total, 2);
                assert_eq!(staged, 2);
            }
            _ => panic!("expected folder"),
        }
    }
}
