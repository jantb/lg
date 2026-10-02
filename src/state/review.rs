//! The assisted review on screen and everything read or written about it.

use std::collections::{HashMap, HashSet};

use crate::git::AssistedReview;
use crate::panel::text_input::TextInput;

use super::{ReviewChatMessage, ReviewStyleFinding};

/// What the review pane drew for each node below its title line, kept until
/// anything it was drawn from changes. Drawing a node means highlighting its
/// diff, rendering its markdown and reading the files its source context
/// shows, and the pane is drawn, measured and hit-tested several times a frame.
#[derive(Debug, Default)]
pub struct ReviewRenderCache {
    /// What every entry was drawn from: the review, what has been said about
    /// it, the width and the view. A different key throws everything away.
    pub key: Option<u64>,
    /// Per node index: what that node's own lines depend on (open or not,
    /// source shown or not, its assist text), and the lines.
    pub bodies: HashMap<usize, (u64, std::sync::Arc<Vec<ratatui::text::Line<'static>>>)>,
    /// Files read for source context, by path.
    pub files: HashMap<String, Option<String>>,
}

#[derive(Default)]
pub struct ReviewState {
    /// The review being read, once it has been built.
    pub assisted: Option<AssistedReview>,
    pub idx: usize,
    pub collapsed: HashSet<String>,
    pub context_open: HashSet<String>,
    pub context_restore_collapsed: HashSet<String>,
    pub assists: HashMap<String, String>,
    pub style_findings: HashMap<String, ReviewStyleFinding>,
    pub flag_active_path: Option<String>,
    pub chat_messages: Vec<ReviewChatMessage>,
    pub chat_input: TextInput,
    pub chat_scroll: u16,
    pub chat_height: Option<u16>,
    pub chat_drag_active: bool,
}

impl ReviewState {
    /// Forget the review and everything said about it, ahead of building a
    /// new one. How the chat pane is laid out is the reader's, and stays.
    pub fn reset(&mut self) {
        *self = Self {
            chat_height: self.chat_height,
            chat_drag_active: self.chat_drag_active,
            ..Self::default()
        };
    }
}
