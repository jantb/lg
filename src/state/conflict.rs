//! The conflict being worked through: its files, and what has been settled.

use std::collections::{HashMap, HashSet};

use super::{ConflictFollowup, ConflictPreview};

#[derive(Default)]
pub struct ConflictState {
    /// The conflicted files, by path.
    pub files: Vec<String>,
    pub idx: usize,
    pub preview: Option<ConflictPreview>,
    pub scroll_offset: usize,
    pub log: String,
    pub followup: Option<ConflictFollowup>,
    /// Files settled by the inline editor or local model in this conflict. They are still
    /// conflicted as far as git is concerned — nothing is staged until `v` —
    /// so this is what tells the panel which ones are waiting to be read
    /// rather than waiting to be resolved.
    pub resolved: HashSet<String>,
    /// How the local model settled each conflict of the files it resolved,
    /// by path, for the editor to show beside them.
    pub model_notes: HashMap<String, Vec<String>>,
}
