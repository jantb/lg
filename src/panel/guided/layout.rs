//! How a hunk's lines are laid out in rows: one line a row, or side by side
//! with each removed line beside the line that replaced it. Lines are named by
//! their index in the hunk either way, so the cursor, notes and findings stay
//! with their line however it is drawn.

use crate::git::guided::{GuidedHunk, LineKind};
use crate::state::DiffViewMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// One line across the whole width.
    Whole(usize),
    /// The old file's line on the left and the new file's on the right; a
    /// kept line is on both.
    Pair(Option<usize>, Option<usize>),
}

impl Row {
    /// The lines on this row, left to right, each once.
    pub fn lines(self) -> impl Iterator<Item = usize> {
        let (first, second) = match self {
            Self::Whole(at) => (Some(at), None),
            Self::Pair(left, right) => (left, right.filter(|r| Some(*r) != left)),
        };
        first.into_iter().chain(second)
    }
}

pub fn rows(step: &GuidedHunk, mode: DiffViewMode) -> Vec<Row> {
    match mode {
        DiffViewMode::Unified => (0..step.lines.len()).map(Row::Whole).collect(),
        DiffViewMode::SideBySide => side_by_side(step),
    }
}

/// The order the cursor visits a hunk's lines in: down the rows, and left to
/// right along each.
pub fn visit_order(rows: &[Row]) -> Vec<usize> {
    rows.iter().flat_map(|row| row.lines()).collect()
}

fn side_by_side(step: &GuidedHunk) -> Vec<Row> {
    let mut rows = Vec::new();
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    for (at, line) in step.lines.iter().enumerate() {
        match line.kind {
            LineKind::Added => added.push(at),
            // A removal after additions starts the next change.
            LineKind::Removed if added.is_empty() => removed.push(at),
            LineKind::Removed => {
                pair_up(&mut rows, &mut removed, &mut added);
                removed.push(at);
            }
            LineKind::Context => {
                pair_up(&mut rows, &mut removed, &mut added);
                rows.push(Row::Pair(Some(at), Some(at)));
            }
            LineKind::Meta => {
                pair_up(&mut rows, &mut removed, &mut added);
                rows.push(Row::Whole(at));
            }
        }
    }
    pair_up(&mut rows, &mut removed, &mut added);
    rows
}

/// A run of removed lines beside the run of added lines that replaced it.
fn pair_up(rows: &mut Vec<Row>, removed: &mut Vec<usize>, added: &mut Vec<usize>) {
    let count = removed.len().max(added.len());
    rows.extend((0..count).map(|k| Row::Pair(removed.get(k).copied(), added.get(k).copied())));
    removed.clear();
    added.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two removed lines beside the two that replaced them, kept lines on
    /// both sides, and the cursor going across each row before down.
    #[test]
    fn side_by_side_puts_each_removed_line_beside_its_replacement() {
        let step = crate::git::guided::parse_hunks(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
             @@ -1,3 +1,3 @@\n keep\n-a\n-b\n+c\n+d\n",
        )
        .remove(0);
        let at = |text: &str| step.lines.iter().position(|l| l.text == text);

        let rows = rows(&step, DiffViewMode::SideBySide);

        assert_eq!(
            rows,
            vec![
                Row::Pair(at(" keep"), at(" keep")),
                Row::Pair(at("-a"), at("+c")),
                Row::Pair(at("-b"), at("+d")),
            ]
        );
        assert_eq!(
            visit_order(&rows),
            [" keep", "-a", "+c", "-b", "+d"].map(|t| at(t).unwrap())
        );
    }
}
