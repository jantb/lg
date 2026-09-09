//! In-memory decisions and text editing; disk IO lives in git::MergeSnapshot.

use anyhow::{Result, bail};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
};

use crate::git::{ConflictHunk, ConflictedFile, MergeSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeAction {
    /// Take our side into the result: in place of the base, or after
    /// whatever side was taken first.
    AcceptOurs,
    AcceptTheirs,
    /// Both sides, ours first.
    Both,
    /// Settle the conflict with the result as it stands.
    Keep,
    Edit,
    Save,
    Base,
    Previous,
    Next,
    /// Accept one side for every conflict not yet settled.
    AllOurs,
    AllTheirs,
}

impl MergeAction {
    pub fn preview(self, hunk: &MergeHunk) -> Option<String> {
        match self {
            Self::AcceptOurs | Self::AllOurs => Some(hunk.accepting(&[Side::Ours])),
            Self::AcceptTheirs | Self::AllTheirs => Some(hunk.accepting(&[Side::Theirs])),
            Self::Both => Some(hunk.accepting(&[Side::Ours, Side::Theirs])),
            _ => None,
        }
    }
}

/// One side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

#[derive(Debug)]
pub struct MergeHunk {
    pub source: ConflictHunk,
    pub result: String,
    pub accepted: bool,
    pub cursor: usize,
    /// The sides taken into the result so far, in the order they were taken.
    pub applied: Vec<Side>,
    history: Vec<(String, bool, usize, Vec<Side>)>,
}

impl MergeHunk {
    fn checkpoint(&mut self) {
        if self.history.len() >= 100 {
            self.history.remove(0);
        }
        self.history.push((
            self.result.clone(),
            self.accepted,
            self.cursor,
            self.applied.clone(),
        ));
    }

    fn side_text(&self, side: Side) -> &str {
        match side {
            Side::Ours => &self.source.ours,
            Side::Theirs => &self.source.theirs,
        }
    }

    /// The result once `sides` are taken as well: the first side taken
    /// stands in for an untouched base, and each after that follows on, as
    /// does the first when the result has already been written to by hand.
    /// A side already taken is not taken twice.
    pub fn accepting(&self, sides: &[Side]) -> String {
        let mut result = self.result.clone();
        let mut applied = self.applied.clone();
        let untouched =
            applied.is_empty() && self.result == self.source.base.clone().unwrap_or_default();
        for &side in sides {
            if applied.contains(&side) {
                continue;
            }
            if untouched && applied.is_empty() {
                result = self.side_text(side).to_string();
            } else {
                result.push_str(self.side_text(side));
            }
            applied.push(side);
        }
        result
    }

    /// Which sides a result written elsewhere amounts to, when it is one of
    /// them or both in either order; nothing is claimed for anything else.
    fn sides_of(source: &ConflictHunk, result: &str) -> Vec<Side> {
        if result.is_empty() {
            return Vec::new();
        }
        let both = format!("{}{}", source.ours, source.theirs);
        let reversed = format!("{}{}", source.theirs, source.ours);
        if result == source.ours {
            vec![Side::Ours]
        } else if result == source.theirs {
            vec![Side::Theirs]
        } else if result == both {
            vec![Side::Ours, Side::Theirs]
        } else if result == reversed {
            vec![Side::Theirs, Side::Ours]
        } else {
            Vec::new()
        }
    }

    /// Take `sides` into the result and count the conflict settled.
    pub fn accept(&mut self, sides: &[Side]) {
        self.checkpoint();
        self.result = self.accepting(sides);
        for &side in sides {
            if !self.applied.contains(&side) {
                self.applied.push(side);
            }
        }
        self.accepted = true;
        self.cursor = 0;
    }

    /// The keyboard's shorthand: `1` ours, `2` theirs, `3` both, `0` keep.
    pub fn choose(&mut self, choice: char) {
        match choice {
            '1' => self.accept(&[Side::Ours]),
            '2' => self.accept(&[Side::Theirs]),
            '3' => self.accept(&[Side::Ours, Side::Theirs]),
            _ => {
                self.checkpoint();
                self.accepted = true;
            }
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.checkpoint();
        self.result.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.accepted = true;
    }

    pub fn undo(&mut self) {
        if let Some((text, accepted, cursor, applied)) = self.history.pop() {
            self.result = text;
            self.accepted = accepted;
            self.cursor = cursor;
            self.applied = applied;
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
        let mut parsed = ConflictedFile::parse(&snapshot.original);
        if parsed.is_none() && crate::git::holds_conflict_marker(&snapshot.original) {
            bail!("unsupported or incomplete conflict markers; press o to resolve externally");
        }
        // A file with no markers left in it but still unmerged in the index
        // has been written by something else: an earlier save, or another
        // tool. The conflicts are still there to be seen in the three stages,
        // so recover them from git's own three-way merge and take what the
        // file holds in each one's place as its result so far.
        let mut recovered = None;
        if parsed.is_none()
            && let Some(diff3) = &snapshot.diff3
            && let Some(results) = split_resolved(&snapshot.original, diff3)
        {
            parsed = Some(diff3.clone());
            recovered = Some(results);
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
            .enumerate()
            .map(|(index, mut source)| {
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
                // The result starts from what both sides started from: the
                // common ancestor, or nothing where there is none to show.
                // Taking a side is then a decision the reader can see.
                let (result, accepted) = match &recovered {
                    Some(results) => (results[index].clone(), true),
                    None if parsed.is_some() => (source.base.clone().unwrap_or_default(), false),
                    None => (snapshot.original.clone(), true),
                };
                let applied = if recovered.is_some() {
                    MergeHunk::sides_of(&source, &result)
                } else {
                    Vec::new()
                };
                MergeHunk {
                    result,
                    source,
                    accepted,
                    cursor: 0,
                    applied,
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
            MergeAction::AcceptOurs => self.hunks[index].accept(&[Side::Ours]),
            MergeAction::AcceptTheirs => self.hunks[index].accept(&[Side::Theirs]),
            MergeAction::Both => self.hunks[index].accept(&[Side::Ours, Side::Theirs]),
            MergeAction::Keep => self.hunks[index].choose('0'),
            MergeAction::AllOurs | MergeAction::AllTheirs => {
                let side = if action == MergeAction::AllOurs {
                    Side::Ours
                } else {
                    Side::Theirs
                };
                for hunk in self.hunks.iter_mut().filter(|h| !h.accepted) {
                    hunk.accept(&[side]);
                }
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

/// What `original`, a file with no markers in it, holds in place of each
/// conflict of `merged`, git's three-way merge of its stages: the text
/// between the stretches git merged on its own, which must all appear in
/// `original` in order and account for all of it. `None` when they do not,
/// or when two conflicts adjoin with nothing merged between them to split
/// the file on.
fn split_resolved(original: &str, merged: &ConflictedFile) -> Option<Vec<String>> {
    let mut results = Vec::new();
    let mut position = 0;
    let mut open = false;
    for part in merged.parts() {
        match part {
            crate::git::FilePart::Kept(text) => {
                let found = position + original[position..].find(text)?;
                if open {
                    results.push(original[position..found].to_string());
                    open = false;
                } else if found != position {
                    return None;
                }
                position = found + text.len();
            }
            crate::git::FilePart::Conflict(_) => {
                if open {
                    return None;
                }
                open = true;
            }
        }
    }
    if open {
        results.push(original[position..].to_string());
    } else if position != original.len() {
        return None;
    }
    (results.len() == merged.hunk_count()).then_some(results)
}

#[derive(Debug)]
pub struct ConflictPreview {
    pub root: String,
    pub path: String,
    pub editor: Result<MergeEditor, String>,
}
