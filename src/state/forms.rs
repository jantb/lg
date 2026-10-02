//! The small forms modals edit: who commits are attributed to, a new
//! worktree, and a branch to delete.

use crate::panel::text_input::TextInput;

use super::{AuthorField, DeleteBranchField, WorktreeField};

pub struct AuthorForm {
    pub path: TextInput,
    pub name: TextInput,
    pub email: TextInput,
    pub field: AuthorField,
    pub has_local_override: bool,
    pub has_subtree_rule: bool,
}

impl Default for AuthorForm {
    fn default() -> Self {
        Self {
            path: TextInput::default(),
            name: TextInput::default(),
            email: TextInput::default(),
            field: AuthorField::Path,
            has_local_override: false,
            has_subtree_rule: false,
        }
    }
}

impl AuthorForm {
    /// The input the focused field edits.
    pub fn focused_mut(&mut self) -> &mut TextInput {
        match self.field {
            AuthorField::Path => &mut self.path,
            AuthorField::Name => &mut self.name,
            AuthorField::Email => &mut self.email,
        }
    }
}

pub struct WorktreeForm {
    pub branch: TextInput,
    pub base: TextInput,
    pub path: TextInput,
    pub field: WorktreeField,
    /// Stops the path following the branch name once the user has typed a path
    /// of their own.
    pub path_edited: bool,
    /// Main worktree the new one will be created next to, captured when the
    /// form opens so the path preview does not have to re-derive it.
    pub repo_dir: String,
}

pub struct DeleteBranchForm {
    pub target: String,
    pub local: bool,
    pub remote: bool,
    pub remote_available: bool,
    pub force: bool,
    pub field: DeleteBranchField,
}

impl Default for WorktreeForm {
    fn default() -> Self {
        Self {
            branch: TextInput::default(),
            base: TextInput::default(),
            path: TextInput::default(),
            field: WorktreeField::Branch,
            path_edited: false,
            repo_dir: String::new(),
        }
    }
}

impl Default for DeleteBranchForm {
    fn default() -> Self {
        Self {
            target: String::new(),
            local: true,
            remote: false,
            remote_available: false,
            force: false,
            field: DeleteBranchField::Local,
        }
    }
}

/// The stash modal: what the stash held when it was last read, the entry
/// picked, and the name of a new entry while one is being typed.
#[derive(Default)]
pub struct StashForm {
    pub entries: Vec<crate::git::StashEntry>,
    pub idx: usize,
    /// Why the list could not be read, shown in place of it.
    pub error: Option<String>,
    /// The message for a new stash, while it is being written.
    pub naming: Option<TextInput>,
}

impl StashForm {
    pub fn selected(&self) -> Option<&crate::git::StashEntry> {
        self.entries
            .get(self.idx.min(self.entries.len().saturating_sub(1)))
    }

    /// Read the stash again, keeping the selection on the same entry when it
    /// is still there.
    pub fn reload(&mut self) {
        let selected = self.selected().map(|entry| entry.sha.clone());
        match crate::git::stash_list() {
            Ok(entries) => {
                self.idx = selected
                    .and_then(|sha| entries.iter().position(|entry| entry.sha == sha))
                    .unwrap_or(self.idx)
                    .min(entries.len().saturating_sub(1));
                self.entries = entries;
                self.error = None;
            }
            Err(err) => {
                self.entries.clear();
                self.idx = 0;
                self.error = Some(err.to_string());
            }
        }
    }
}
