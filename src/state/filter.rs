//! The `/` filter over the Files, Branches and Commits lists.
//!
//! A filter hides rows; it never reorders them or changes what a row is. The
//! selection of a filtered list still counts in the whole list, so every
//! action that reads "the selected branch" or "the selected commit" reads the
//! row the user sees highlighted, filtered or not. Only moving the selection,
//! drawing the list and mapping a click onto it go through the rows the filter
//! leaves.

use super::{AppState, BranchView, Pane};
use crate::panel::text_input::TextInput;

/// What a list is narrowed to, and whether keys are being typed into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    pub text: TextInput,
    pub typing: bool,
}

impl ListFilter {
    /// Whether the filter hides anything: there is something typed.
    pub fn active(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// Whether `haystack` passes: the typed text appears in it, whatever the
    /// case.
    pub fn matches(&self, haystack: &str) -> bool {
        let needle = self.text.trim().to_lowercase();
        needle.is_empty() || haystack.to_lowercase().contains(&needle)
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.typing = false;
    }
}

impl AppState {
    /// The filter of a list pane.
    pub fn list_filter(&self, pane: Pane) -> Option<&ListFilter> {
        match pane {
            Pane::Files => Some(&self.files_filter),
            Pane::Branches => Some(&self.branches_filter),
            Pane::Commits => Some(&self.commits_filter),
            Pane::Status | Pane::Main => None,
        }
    }

    pub fn list_filter_mut(&mut self, pane: Pane) -> Option<&mut ListFilter> {
        match pane {
            Pane::Files => Some(&mut self.files_filter),
            Pane::Branches => Some(&mut self.branches_filter),
            Pane::Commits => Some(&mut self.commits_filter),
            Pane::Status | Pane::Main => None,
        }
    }

    /// Whether the focused pane's filter is taking the keyboard.
    pub fn filter_typing(&self) -> bool {
        self.modal == super::Modal::None
            && self
                .list_filter(self.focus)
                .is_some_and(|filter| filter.typing)
    }

    /// The rows of `pane` the filter leaves, as positions in the list its
    /// selection counts in; `None` when nothing is filtered. The Files pane
    /// filters while it builds its tree (see [`AppState::tree_rows`]), so its
    /// rows are already the filtered ones and it answers `None`.
    pub fn filtered_rows(&self, pane: Pane) -> Option<Vec<usize>> {
        match pane {
            Pane::Branches if self.branches_filter.active() => {
                let filter = &self.branches_filter;
                Some(match self.branch_view {
                    BranchView::Local => self
                        .branches
                        .iter()
                        .enumerate()
                        .filter(|(_, branch)| filter.matches(&branch.name))
                        .map(|(at, _)| at)
                        .collect(),
                    BranchView::Remote => self
                        .visible_remote_branches()
                        .enumerate()
                        .filter(|(_, branch)| filter.matches(&branch.name))
                        .map(|(at, _)| at)
                        .collect(),
                })
            }
            Pane::Commits if self.commits_filter.active() => {
                let filter = &self.commits_filter;
                Some(
                    self.commits
                        .iter()
                        .enumerate()
                        .filter(|(_, commit)| {
                            !commit.is_graph_row()
                                && (filter.matches(&commit.subject)
                                    || filter.matches(&commit.sha)
                                    || filter.matches(&commit.author))
                        })
                        .map(|(at, _)| at)
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// Whether the selection is on a row the filter hides — as it is when
    /// nothing matches. Nothing selected can be acted on then.
    pub fn selection_hidden(&self, pane: Pane) -> bool {
        let Some(rows) = self.filtered_rows(pane) else {
            return false;
        };
        let idx = match pane {
            Pane::Branches => match self.branch_view {
                BranchView::Local => self.branches_list.idx,
                BranchView::Remote => self.remote_branches_list.idx,
            },
            Pane::Commits => self.commits_list.idx,
            _ => return false,
        };
        !rows.contains(&idx)
    }

    /// The selection of a filtered pane, moved by `amount` rows among the ones
    /// the filter leaves. False when the pane is not filtered, so the caller
    /// moves it the ordinary way; true otherwise, whether or not it moved.
    pub fn step_filtered(&mut self, pane: Pane, down: bool, amount: usize) -> bool {
        let Some(rows) = self.filtered_rows(pane) else {
            return false;
        };
        let Some(idx) = self.list_idx_mut(pane) else {
            return false;
        };
        if rows.is_empty() {
            return true;
        }
        let at = rows.iter().position(|row| *row == *idx).unwrap_or(0);
        let next = if down {
            at.saturating_add(amount).min(rows.len() - 1)
        } else {
            at.saturating_sub(amount)
        };
        *idx = rows[next];
        true
    }

    /// Put the selection on the first row the filter leaves, as typing into
    /// a filter does: the row selected before may well be gone.
    pub fn select_first_filtered(&mut self, pane: Pane) {
        match pane {
            Pane::Files => {
                // Row 0 is "(all changes)"; the first match is under it.
                let len = self.tree_rows().len();
                self.files_list.idx = usize::from(len > 1);
            }
            pane => {
                let first = self
                    .filtered_rows(pane)
                    .and_then(|rows| rows.first().copied());
                if let Some(idx) = self.list_idx_mut(pane) {
                    *idx = first.unwrap_or(0);
                }
            }
        }
    }

    /// A filtered pane's selection, kept on a row the filter leaves after the
    /// list under it changed.
    pub(super) fn clamp_filtered(&mut self) {
        for pane in [Pane::Branches, Pane::Commits] {
            let Some(rows) = self.filtered_rows(pane) else {
                continue;
            };
            let Some(idx) = self.list_idx_mut(pane) else {
                continue;
            };
            if !rows.contains(idx) {
                *idx = rows
                    .iter()
                    .copied()
                    .find(|row| *row > *idx)
                    .or_else(|| rows.last().copied())
                    .unwrap_or(0);
            }
        }
    }

    fn list_idx_mut(&mut self, pane: Pane) -> Option<&mut usize> {
        match pane {
            Pane::Branches => Some(self.branch_list_idx_mut()),
            Pane::Commits => Some(&mut self.commits_list.idx),
            Pane::Files => Some(&mut self.files_list.idx),
            Pane::Status | Pane::Main => None,
        }
    }

    /// What a pane's title adds while it is filtered.
    pub fn filter_title(&self, pane: Pane) -> String {
        match self.list_filter(pane) {
            Some(filter) if filter.typing => format!(" /{}\u{2502}", filter.text.as_str()),
            Some(filter) if filter.active() => format!(" /{}", filter.text.as_str()),
            _ => String::new(),
        }
    }
}

/// Where in a filtered list's visible rows `idx` sits, and how many rows there
/// are; the whole list when `rows` is `None`.
pub fn visible_position(rows: Option<&[usize]>, idx: usize, len: usize) -> (Option<usize>, usize) {
    match rows {
        Some(rows) => (rows.iter().position(|row| *row == idx), rows.len()),
        None => ((idx < len).then_some(idx), len),
    }
}
