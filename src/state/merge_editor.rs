//! In-memory decisions and text editing; disk IO lives in git::MergeSnapshot.

use anyhow::{Result, bail};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
};

use crate::git::{ConflictHunk, ConflictedFile, MergeSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeAction {
    ReplaceOurs,
    InsertOurs,
    ReplaceTheirs,
    InsertTheirs,
    Both,
    Keep,
    Edit,
    Save,
    Base,
    Previous,
    Next,
}

impl MergeAction {
    pub fn preview(self, hunk: &MergeHunk) -> Option<String> {
        match self {
            Self::ReplaceOurs => Some(hunk.source.ours.clone()),
            Self::ReplaceTheirs => Some(hunk.source.theirs.clone()),
            Self::Both => Some(format!("{}{}", hunk.source.ours, hunk.source.theirs)),
            Self::InsertOurs | Self::InsertTheirs => {
                let mut result = hunk.result.clone();
                result.insert_str(
                    hunk.cursor,
                    if self == Self::InsertOurs {
                        &hunk.source.ours
                    } else {
                        &hunk.source.theirs
                    },
                );
                Some(result)
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct MergeHunk {
    pub source: ConflictHunk,
    pub result: String,
    pub accepted: bool,
    pub cursor: usize,
    history: Vec<(String, bool, usize)>,
}

impl MergeHunk {
    fn checkpoint(&mut self) {
        if self.history.len() >= 100 {
            self.history.remove(0);
        }
        self.history
            .push((self.result.clone(), self.accepted, self.cursor));
    }

    pub fn choose(&mut self, choice: char) {
        self.checkpoint();
        self.result = match choice {
            '1' => self.source.ours.clone(),
            '2' => self.source.theirs.clone(),
            '3' => format!("{}{}", self.source.ours, self.source.theirs),
            _ => self.result.clone(),
        };
        self.accepted = true;
        self.cursor = 0;
    }

    pub fn insert(&mut self, text: &str) {
        self.checkpoint();
        self.result.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.accepted = true;
    }

    pub fn undo(&mut self) {
        if let Some((text, accepted, cursor)) = self.history.pop() {
            self.result = text;
            self.accepted = accepted;
            self.cursor = cursor;
        }
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        let prefix = &self.result[..self.cursor];
        (
            prefix.bytes().filter(|b| *b == b'\n').count(),
            prefix
                .rsplit('\n')
                .next()
                .unwrap_or_default()
                .chars()
                .count(),
        )
    }

    pub fn edit(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            if key.code == KeyCode::Char('z') {
                self.undo();
            }
            return;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return;
        }
        match key.code {
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Enter => self.insert(
                if self.result.contains("\r\n") || self.source.ours.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                },
            ),
            KeyCode::Tab => self.insert("    "),
            KeyCode::Left => {
                self.cursor = self.result[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
                if self.result[..self.cursor].ends_with('\r')
                    && self.result[self.cursor..].starts_with('\n')
                {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.result[self.cursor..].starts_with("\r\n") {
                    self.cursor += 2;
                } else if let Some(c) = self.result[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
            }
            KeyCode::Home => {
                self.cursor = self.result[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
            }
            KeyCode::End => {
                let end = self.result[self.cursor..]
                    .find('\n')
                    .map_or(self.result.len(), |i| self.cursor + i);
                self.cursor = if self.result[..end].ends_with('\r') {
                    end - 1
                } else {
                    end
                };
            }
            KeyCode::Up | KeyCode::Down => {
                let (row, column) = self.cursor_position();
                let target = if key.code == KeyCode::Up {
                    row.saturating_sub(1)
                } else {
                    row + 1
                };
                let lines: Vec<_> = self.result.split_inclusive('\n').collect();
                let start: usize = lines.iter().take(target).map(|line| line.len()).sum();
                if target <= self.result.bytes().filter(|b| *b == b'\n').count() {
                    let line = self.result[start..]
                        .split('\n')
                        .next()
                        .unwrap_or_default()
                        .trim_end_matches('\r');
                    self.cursor = start
                        + line
                            .char_indices()
                            .nth(column)
                            .map_or(line.len(), |(i, _)| i);
                }
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.checkpoint();
                let start = if self.result[..self.cursor].ends_with("\r\n") {
                    self.cursor - 2
                } else {
                    self.result[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i)
                };
                self.result.replace_range(start..self.cursor, "");
                self.cursor = start;
                self.accepted = true;
            }
            KeyCode::Delete if self.cursor < self.result.len() => {
                self.checkpoint();
                let length = if self.result[self.cursor..].starts_with("\r\n") {
                    2
                } else {
                    self.result[self.cursor..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8)
                };
                self.result
                    .replace_range(self.cursor..self.cursor + length, "");
                self.accepted = true;
            }
            _ => {}
        }
    }
}

/// One stretch of the file as the editor lays it out: merged text every side
/// agrees on, or a conflict identified by its position in `hunks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergePart {
    Kept(String),
    Conflict(usize),
}

#[derive(Debug)]
pub struct MergeEditor {
    pub snapshot: MergeSnapshot,
    parsed: Option<ConflictedFile>,
    pub hunks: Vec<MergeHunk>,
    pub selected: usize,
    pub editing: bool,
    pub show_base: bool,
    /// Row offset into the laid-out file, honoured once neither `reveal` nor
    /// `follow_cursor` asks for something else.
    pub scroll: usize,
    /// Line offset in the ancestor view, kept apart so toggling the ancestor
    /// does not lose the place in the file.
    pub base_scroll: usize,
    pub horizontal: usize,
    pub saved: bool,
    /// The button under the mouse: which conflict it belongs to, and what it does.
    pub hovered: Option<(usize, MergeAction)>,
    /// Keep the result cursor of the selected conflict on screen.
    pub follow_cursor: bool,
    /// Bring the selected conflict into view on the next render.
    pub reveal: bool,
    /// Where the view was last drawn, so keyboard scrolling can measure it.
    pub viewport: Option<Rect>,
    baseline: Vec<(String, bool)>,
}

impl MergeEditor {
    pub fn new(snapshot: MergeSnapshot) -> Result<Self> {
        let parsed = ConflictedFile::parse(&snapshot.original);
        if parsed.is_none() && crate::git::holds_conflict_marker(&snapshot.original) {
            bail!("unsupported or incomplete conflict markers; press o to resolve externally");
        }
        let sources: Vec<_> = if let Some(file) = &parsed {
            file.hunks().cloned().collect()
        } else {
            vec![ConflictHunk {
                ours_label: "index stage 2".into(),
                ours: snapshot.ours.clone(),
                base: Some(snapshot.base.clone()),
                theirs: snapshot.theirs.clone(),
                theirs_label: "index stage 3".into(),
            }]
        };
        let hunks: Vec<_> = sources
            .into_iter()
            .map(|mut source| {
                if source.base.is_none()
                    && let Some(diff3) = &snapshot.diff3
                {
                    let matching: Vec<_> = diff3
                        .hunks()
                        .filter(|candidate| {
                            candidate.ours == source.ours && candidate.theirs == source.theirs
                        })
                        .collect();
                    // Never attach ancestor text by position alone: a user may
                    // already have resolved some of the file's earlier conflicts.
                    if matching.len() == 1 {
                        source.base = matching[0].base.clone();
                    }
                }
                MergeHunk {
                    result: if parsed.is_some() {
                        source.ours.clone()
                    } else {
                        snapshot.original.clone()
                    },
                    source,
                    accepted: parsed.is_none(),
                    cursor: 0,
                    history: Vec::new(),
                }
            })
            .collect();
        let baseline = hunks
            .iter()
            .map(|h| (h.result.clone(), h.accepted))
            .collect();
        Ok(Self {
            snapshot,
            parsed,
            hunks,
            selected: 0,
            editing: false,
            show_base: false,
            scroll: 0,
            base_scroll: 0,
            horizontal: 0,
            saved: false,
            hovered: None,
            follow_cursor: false,
            reveal: true,
            viewport: None,
            baseline,
        })
    }

    pub fn dirty(&self) -> bool {
        self.hunks
            .iter()
            .zip(&self.baseline)
            .any(|(h, (text, accepted))| h.result != *text || h.accepted != *accepted)
    }

    pub fn current(&self) -> &MergeHunk {
        &self.hunks[self.selected]
    }
    pub fn current_mut(&mut self) -> &mut MergeHunk {
        &mut self.hunks[self.selected]
    }

    /// Make `index` the selected conflict without moving the view; the caller
    /// is acting on something already on screen.
    pub fn select(&mut self, index: usize) {
        if index < self.hunks.len() {
            self.selected = index;
        }
    }

    pub fn navigate(&mut self, forward: bool) {
        self.selected = if forward {
            (self.selected + 1).min(self.hunks.len() - 1)
        } else {
            self.selected.saturating_sub(1)
        };
        self.reveal = true;
        self.follow_cursor = false;
        self.horizontal = 0;
    }

    /// The whole file in order: merged text and conflicts interleaved. A file
    /// without markers is one conflict spanning everything.
    pub fn parts(&self) -> Vec<MergePart> {
        self.parsed.as_ref().map_or_else(
            || vec![MergePart::Conflict(0)],
            |parsed| {
                parsed
                    .parts()
                    .map(|part| match part {
                        crate::git::FilePart::Kept(text) => MergePart::Kept(text.to_string()),
                        crate::git::FilePart::Conflict(index) => MergePart::Conflict(index),
                    })
                    .collect()
            },
        )
    }

    /// Carry out an action on conflict `index`. Saving is the caller's: it
    /// touches the disk and the dialog's bookkeeping.
    pub fn apply(&mut self, index: usize, action: MergeAction) {
        if index >= self.hunks.len() {
            return;
        }
        self.selected = index;
        match action {
            MergeAction::ReplaceOurs => self.hunks[index].choose('1'),
            MergeAction::ReplaceTheirs => self.hunks[index].choose('2'),
            MergeAction::Both => self.hunks[index].choose('3'),
            MergeAction::Keep => self.hunks[index].choose('0'),
            MergeAction::InsertOurs | MergeAction::InsertTheirs => {
                let source = if action == MergeAction::InsertOurs {
                    self.hunks[index].source.ours.clone()
                } else {
                    self.hunks[index].source.theirs.clone()
                };
                self.hunks[index].insert(&source);
            }
            MergeAction::Edit => {
                self.editing = true;
                self.follow_cursor = true;
            }
            MergeAction::Base => {
                self.show_base = !self.show_base;
                self.base_scroll = 0;
            }
            MergeAction::Previous => self.navigate(false),
            MergeAction::Next => self.navigate(true),
            MergeAction::Save => {}
        }
        if !matches!(
            action,
            MergeAction::Base | MergeAction::Previous | MergeAction::Next
        ) {
            self.show_base = false;
        }
    }

    pub fn save(&mut self) -> Result<()> {
        if self.hunks.iter().any(|h| !h.accepted) {
            bail!("accept or edit every conflict before saving");
        }
        let resolutions: Vec<_> = self.hunks.iter().map(|h| h.result.clone()).collect();
        let result = if let Some(parsed) = &self.parsed {
            parsed
                .render_exact(&resolutions)
                .expect("one result per hunk")
        } else {
            resolutions[0].clone()
        };
        self.snapshot.save(&result)?;
        self.baseline = self
            .hunks
            .iter()
            .map(|h| (h.result.clone(), h.accepted))
            .collect();
        self.saved = true;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ConflictPreview {
    pub root: String,
    pub path: String,
    pub editor: Result<MergeEditor, String>,
}
