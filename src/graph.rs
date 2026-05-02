//! Commit-graph rendering — port of lazygit's `pkg/gui/presentation/graph`.
//!
//! Two stages:
//! 1. [`pipe_sets`] turns a `Vec<Commit>` into a per-row `Vec<Pipe>` describing every
//!    lane crossing that row.
//! 2. [`render_pipe_set`] turns one row of pipes into 2-char cells (symbol + right
//!    connector) using box-drawing glyphs.
//!
//! The algorithm matches lazygit so the output is glyph-for-glyph identical for the
//! same DAG.

use std::collections::HashSet;

use ratatui::style::Color;

/// Sentinel hash used for the seed pipe before the first commit.
pub const START_HASH: &str = "__START__";
/// Sentinel hash used as the parent of the root commit.
pub const ROOT_HASH: &str = "__ROOT__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PipeKind {
    /// Pipe ends at this row (drawn coming in from above, terminating here).
    Terminates,
    /// Pipe starts at this row (drawn going down).
    Starts,
    /// Pipe passes through this row.
    Continues,
}

#[derive(Debug, Clone)]
pub struct Pipe {
    pub from_hash: String,
    pub to_hash: String,
    pub from_pos: i16,
    pub to_pos: i16,
    pub kind: PipeKind,
    pub color: Color,
}

impl Pipe {
    fn left(&self) -> i16 {
        self.from_pos.min(self.to_pos)
    }
    fn right(&self) -> i16 {
        self.from_pos.max(self.to_pos)
    }
}

/// Minimal commit info the algorithm needs.
pub trait CommitNode {
    fn sha(&self) -> &str;
    fn parents(&self) -> &[String];
    fn is_first_parent(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellKind {
    Connection,
    Commit,
    Merge,
}

#[derive(Debug, Clone)]
struct Cell {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    kind: CellKind,
    color: Color,
    right_color: Option<Color>,
}

impl Cell {
    fn new() -> Self {
        Self {
            up: false,
            down: false,
            left: false,
            right: false,
            kind: CellKind::Connection,
            color: Color::Reset,
            right_color: None,
        }
    }

    fn set_up(&mut self, color: Color) {
        self.up = true;
        self.color = color;
    }
    fn set_down(&mut self, color: Color) {
        self.down = true;
        self.color = color;
    }
    fn set_left(&mut self, color: Color) {
        self.left = true;
        if !self.up && !self.down {
            self.color = color;
        }
    }
    fn set_right(&mut self, color: Color, override_existing: bool) {
        self.right = true;
        if self.right_color.is_none() || override_existing {
            self.right_color = Some(color);
        }
    }
    fn reset(&mut self) {
        self.up = false;
        self.down = false;
        self.left = false;
        self.right = false;
        self.right_color = None;
    }
}

/// One rendered cell pair: `(symbol, symbol_color, connector, connector_color)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedCell {
    pub symbol: char,
    pub symbol_color: Color,
    pub connector: char,
    pub connector_color: Color,
}

/// Selection-highlight color used for the bolded path. Style is applied at render time.
pub const SELECTED_COLOR: Color = Color::White;

/// Default 8-color rotating palette. Pipes are colored by `hash(from_hash) % 8`.
pub const PALETTE: &[Color] = &[
    Color::LightGreen,
    Color::LightMagenta,
    Color::LightCyan,
    Color::Yellow,
    Color::Cyan,
    Color::Magenta,
    Color::LightBlue,
    Color::LightYellow,
];

pub fn color_for(hash: &str) -> Color {
    let h = hash
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    PALETTE[h as usize % PALETTE.len()]
}

/// Build all pipe sets for `commits`. Index `i` of the returned vec is the pipe set
/// for `commits[i]`.
pub fn pipe_sets<C: CommitNode>(commits: &[C]) -> Vec<Vec<Pipe>> {
    if commits.is_empty() {
        return Vec::new();
    }

    let seed = vec![Pipe {
        from_hash: START_HASH.to_string(),
        to_hash: commits[0].sha().to_string(),
        from_pos: 0,
        to_pos: 0,
        kind: PipeKind::Starts,
        color: color_for(commits[0].sha()),
    }];

    let mut out: Vec<Vec<Pipe>> = Vec::with_capacity(commits.len());
    let mut prev = seed;
    for commit in commits {
        let next = next_pipes(&prev, commit);
        prev = next.clone();
        out.push(next);
    }
    out
}

fn next_pipes<C: CommitNode>(prev: &[Pipe], commit: &C) -> Vec<Pipe> {
    let max_pos = prev.iter().map(|p| p.to_pos).max().unwrap_or(0);

    // Filter out pipes that already terminated.
    let current: Vec<&Pipe> = prev.iter().filter(|p| p.kind != PipeKind::Terminates).collect();

    // Where does this commit land? At the position of the first prev pipe whose `to`
    // equals our hash; otherwise to the right of all existing lanes (covers `--all`).
    let mut pos = max_pos + 1;
    for p in &current {
        if p.to_hash == commit.sha() {
            pos = p.to_pos;
            break;
        }
    }

    let parents = commit.parents();
    let first_parent_hash = parents
        .first()
        .cloned()
        .unwrap_or_else(|| ROOT_HASH.to_string());
    let commit_color = color_for(commit.sha());

    let mut new_pipes: Vec<Pipe> = Vec::with_capacity(current.len() + parents.len());
    new_pipes.push(Pipe {
        from_hash: commit.sha().to_string(),
        to_hash: first_parent_hash,
        from_pos: pos,
        to_pos: pos,
        kind: PipeKind::Starts,
        color: commit_color,
    });

    let mut taken: HashSet<i16> = HashSet::new();
    let mut traversed: HashSet<i16> = HashSet::new();

    let mut traversed_by_continuing: HashSet<i16> = HashSet::new();
    for p in &current {
        if p.to_hash != commit.sha() {
            traversed_by_continuing.insert(p.to_pos);
        }
    }

    let traverse = |from: i16, to: i16, taken: &mut HashSet<i16>, traversed: &mut HashSet<i16>| {
        let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
        for i in lo..=hi {
            traversed.insert(i);
        }
        taken.insert(to);
    };

    let next_avail_continuing = |traversed: &HashSet<i16>| -> i16 {
        let mut i: i16 = 0;
        loop {
            if !traversed.contains(&i) {
                return i;
            }
            i += 1;
        }
    };

    let next_avail_new_pipe =
        |taken: &HashSet<i16>, traversed_by_continuing: &HashSet<i16>| -> i16 {
            let mut i: i16 = 0;
            loop {
                if !taken.contains(&i) && !traversed_by_continuing.contains(&i) {
                    return i;
                }
                i += 1;
            }
        };

    // Terminating pipes (those whose target is this commit) and continuing-from-left
    // pipes (those whose lane is to the left of pos).
    for p in &current {
        if p.to_hash == commit.sha() {
            new_pipes.push(Pipe {
                from_hash: p.from_hash.clone(),
                to_hash: p.to_hash.clone(),
                from_pos: p.to_pos,
                to_pos: pos,
                kind: PipeKind::Terminates,
                color: p.color,
            });
            traverse(p.to_pos, pos, &mut taken, &mut traversed);
        } else if p.to_pos < pos {
            let avail = next_avail_continuing(&traversed);
            new_pipes.push(Pipe {
                from_hash: p.from_hash.clone(),
                to_hash: p.to_hash.clone(),
                from_pos: p.to_pos,
                to_pos: avail,
                kind: PipeKind::Continues,
                color: p.color,
            });
            traverse(p.to_pos, avail, &mut taken, &mut traversed);
        }
    }

    // For merges: extra parents become new lanes.
    if parents.len() > 1 {
        for parent in &parents[1..] {
            let avail = next_avail_new_pipe(&taken, &traversed_by_continuing);
            new_pipes.push(Pipe {
                from_hash: commit.sha().to_string(),
                to_hash: parent.clone(),
                from_pos: pos,
                to_pos: avail,
                kind: PipeKind::Starts,
                color: commit_color,
            });
            taken.insert(avail);
        }
    }

    // Continuing pipes that were to the right of pos — they may slide left into a
    // freshly emptied lane.
    for p in &current {
        if p.to_hash != commit.sha() && p.to_pos > pos {
            let mut last = p.to_pos;
            let mut i = p.to_pos;
            while i > pos {
                if taken.contains(&i) || traversed.contains(&i) {
                    break;
                }
                last = i;
                i -= 1;
            }
            new_pipes.push(Pipe {
                from_hash: p.from_hash.clone(),
                to_hash: p.to_hash.clone(),
                from_pos: p.to_pos,
                to_pos: last,
                kind: PipeKind::Continues,
                color: p.color,
            });
            traverse(p.to_pos, last, &mut taken, &mut traversed);
        }
    }

    // Sort by to_pos, then by kind ordering (Terminates < Starts < Continues).
    new_pipes.sort_by(|a, b| match a.to_pos.cmp(&b.to_pos) {
        std::cmp::Ordering::Equal => a.kind.cmp(&b.kind),
        other => other,
    });

    new_pipes
}

/// Render one row of pipes into `RenderedCell`s. Pass the previous commit's SHA so
/// we can suppress the highlight on consecutive selected commits with no visible
/// connector between them.
pub fn render_pipe_set(
    pipes: &[Pipe],
    selected: Option<&str>,
    prev_sha: Option<&str>,
) -> Vec<RenderedCell> {
    let mut max_pos: i16 = 0;
    let mut commit_pos: i16 = 0;
    let mut start_count: usize = 0;
    for p in pipes {
        if p.kind == PipeKind::Starts {
            start_count += 1;
            commit_pos = p.from_pos;
        } else if p.kind == PipeKind::Terminates {
            commit_pos = p.to_pos;
        }
        if p.right() > max_pos {
            max_pos = p.right();
        }
    }
    let is_merge = start_count > 1;

    let mut cells: Vec<Cell> = (0..=max_pos).map(|_| Cell::new()).collect();

    // Decide whether to apply the selection highlight on this row.
    let highlight = match (selected, prev_sha) {
        (Some(sel), Some(prev)) if prev == sel => pipes.iter().any(|p| {
            p.from_hash == sel && (p.kind != PipeKind::Terminates || p.from_pos != p.to_pos)
        }),
        (Some(_), _) => true,
        (None, _) => false,
    };

    // Partition: pipes originating from selected commit go last so they overwrite.
    let (selected_pipes, other_pipes): (Vec<&Pipe>, Vec<&Pipe>) = pipes
        .iter()
        .partition(|p| highlight && Some(p.from_hash.as_str()) == selected);

    // First pass: non-selected STARTS (so right-style is set, can be overridden later).
    for p in &other_pipes {
        if p.kind == PipeKind::Starts {
            paint_pipe(&mut cells, p, p.color, true);
        }
    }

    // Second pass: non-selected TERMINATES + CONTINUES, skipping the no-op
    // self-terminate at commit position.
    for p in &other_pipes {
        if p.kind == PipeKind::Starts {
            continue;
        }
        if p.kind == PipeKind::Terminates && p.from_pos == commit_pos && p.to_pos == commit_pos {
            continue;
        }
        paint_pipe(&mut cells, p, p.color, false);
    }

    // Selected pipes: clear the path then repaint with highlight color.
    for p in &selected_pipes {
        for i in p.left()..=p.right() {
            cells[i as usize].reset();
        }
    }
    for p in &selected_pipes {
        paint_pipe(&mut cells, p, SELECTED_COLOR, true);
        if p.to_pos == commit_pos {
            cells[p.to_pos as usize].color = SELECTED_COLOR;
        }
    }

    // Mark commit/merge symbol.
    cells[commit_pos as usize].kind = if is_merge { CellKind::Merge } else { CellKind::Commit };

    cells.iter().map(render_cell).collect()
}

fn paint_pipe(cells: &mut [Cell], pipe: &Pipe, color: Color, override_right: bool) {
    let left = pipe.left();
    let right = pipe.right();

    if left != right {
        for i in (left + 1)..right {
            cells[i as usize].set_left(color);
            cells[i as usize].set_right(color, override_right);
        }
        cells[left as usize].set_right(color, override_right);
        cells[right as usize].set_left(color);
    }

    if pipe.kind == PipeKind::Starts || pipe.kind == PipeKind::Continues {
        cells[pipe.to_pos as usize].set_down(color);
    }
    if pipe.kind == PipeKind::Terminates || pipe.kind == PipeKind::Continues {
        cells[pipe.from_pos as usize].set_up(color);
    }
}

fn render_cell(cell: &Cell) -> RenderedCell {
    let (first, second) = box_drawing_chars(cell.up, cell.down, cell.left, cell.right);
    let symbol = match cell.kind {
        CellKind::Connection => first,
        CellKind::Commit => '\u{25ef}',
        CellKind::Merge => '\u{23e3}',
    };
    let connector_color = cell.right_color.unwrap_or(cell.color);
    RenderedCell {
        symbol,
        symbol_color: cell.color,
        connector: second,
        connector_color,
    }
}

fn box_drawing_chars(up: bool, down: bool, left: bool, right: bool) -> (char, char) {
    match (up, down, left, right) {
        (true, true, true, true) => ('│', '─'),
        (true, true, true, false) => ('│', ' '),
        (true, true, false, true) => ('│', '─'),
        (true, true, false, false) => ('│', ' '),
        (true, false, true, true) => ('┴', '─'),
        (true, false, true, false) => ('╯', ' '),
        (true, false, false, true) => ('╰', '─'),
        (true, false, false, false) => ('╵', ' '),
        (false, true, true, true) => ('┬', '─'),
        (false, true, true, false) => ('╮', ' '),
        (false, true, false, true) => ('╭', '─'),
        (false, true, false, false) => ('╷', ' '),
        (false, false, true, true) => ('─', '─'),
        (false, false, true, false) => ('─', ' '),
        (false, false, false, true) => ('╶', '─'),
        (false, false, false, false) => (' ', ' '),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCommit {
        sha: String,
        parents: Vec<String>,
    }
    impl CommitNode for TestCommit {
        fn sha(&self) -> &str {
            &self.sha
        }
        fn parents(&self) -> &[String] {
            &self.parents
        }
        fn is_first_parent(&self) -> bool {
            true
        }
    }

    fn c(sha: &str, parents: &[&str]) -> TestCommit {
        TestCommit {
            sha: sha.into(),
            parents: parents.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn rendered_string(cells: &[RenderedCell]) -> String {
        let mut s = String::new();
        for (i, cell) in cells.iter().enumerate() {
            s.push(cell.symbol);
            if i + 1 < cells.len() {
                s.push(cell.connector);
            }
        }
        // trim trailing spaces (lazygit's tests trim too)
        s.trim_end().to_string()
    }

    fn render_dag(commits: &[TestCommit]) -> Vec<String> {
        let sets = pipe_sets(commits);
        sets.iter()
            .map(|set| rendered_string(&render_pipe_set(set, None, None)))
            .collect()
    }

    #[test]
    fn render_with_some_merges() {
        let commits = [
            c("1", &["2"]),
            c("2", &["3"]),
            c("3", &["4"]),
            c("4", &["5", "7"]),
            c("7", &["5"]),
            c("5", &["8"]),
            c("8", &["9"]),
            c("9", &["A", "B"]),
            c("B", &["D"]),
            c("D", &["D"]),
            c("A", &["E"]),
            c("E", &["F"]),
            c("F", &["D"]),
            c("D", &["G"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec![
                "◯", "◯", "◯", "⏣─╮", "│ ◯", "◯─╯", "◯", "⏣─╮", "│ ◯", "│ ◯", "◯ │", "◯ │", "◯ │",
                "◯─╯",
            ]
        );
    }

    #[test]
    fn render_path_room_to_move_left() {
        let commits = [
            c("1", &["2"]),
            c("2", &["3", "4"]),
            c("4", &["3", "5"]),
            c("3", &["5"]),
            c("5", &["6"]),
            c("6", &["7"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec!["◯", "⏣─╮", "│ ⏣─╮", "◯─╯ │", "◯───╯", "◯",]
        );
    }

    #[test]
    fn render_new_merge_path_fills_gap() {
        let commits = [
            c("1", &["2", "3", "4", "5"]),
            c("4", &["2"]),
            c("2", &["A"]),
            c("A", &["6", "B"]),
            c("B", &["C"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec!["⏣─┬─┬─╮", "│ │ ◯ │", "◯─│─╯ │", "⏣─│─╮ │", "│ │ ◯ │",]
        );
    }

    #[test]
    fn render_deeply_nested_merges() {
        let commits = [
            c("1", &["2"]),
            c("2", &["3", "4"]),
            c("3", &["5", "4"]),
            c("5", &["7", "8"]),
            c("7", &["4", "A"]),
            c("4", &["B"]),
            c("B", &["C"]),
            c("C", &["D"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec![
                "◯",
                "⏣─╮",
                "⏣─│─╮",
                "⏣─│─│─╮",
                "⏣─│─│─│─╮",
                "◯─┴─╯ │ │",
                "◯ ╭───╯ │",
                "◯ │ ╭───╯",
            ]
        );
    }

    #[test]
    fn render_brand_new_lane_when_unrelated_commit_appears() {
        let commits = [
            c("1", &["2"]),
            c("2", &["3", "4"]),
            c("4", &["3", "5"]),
            c("Z", &["Z"]),
            c("3", &["5"]),
            c("5", &["6"]),
            c("6", &["7"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec![
                "◯",
                "⏣─╮",
                "│ ⏣─╮",
                "│ │ │ ◯",
                "◯─╯ │ │",
                "◯───╯ │",
                "◯ ╭───╯",
            ]
        );
    }

    // Ported from lazygit graph_test.go: "another two parents that have a common ancestor".
    #[test]
    fn render_double_merge_then_long_path() {
        let commits = [
            c("1", &["2"]),
            c("2", &["3", "4"]),
            c("3", &["5", "4"]),
            c("5", &["7", "8"]),
            c("7", &["4", "A"]),
            c("4", &["B"]),
            c("B", &["C"]),
            c("C", &["D"]),
            c("D", &["F"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec![
                "◯",
                "⏣─╮",
                "⏣─│─╮",
                "⏣─│─│─╮",
                "⏣─│─│─│─╮",
                "◯─┴─╯ │ │",
                "◯ ╭───╯ │",
                "◯ │ ╭───╯",
                "◯ │ │",
            ]
        );
    }

    // Ported from lazygit graph_test.go: 6th case at line 137 (extended path with C, D).
    #[test]
    fn render_path_to_left_continues_with_extra_commits() {
        let commits = [
            c("1", &["2"]),
            c("2", &["3", "4"]),
            c("3", &["5", "4"]),
            c("5", &["7", "8"]),
            c("7", &["4", "A"]),
            c("4", &["B"]),
            c("B", &["C"]),
            c("C", &["D"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec![
                "◯",
                "⏣─╮",
                "⏣─│─╮",
                "⏣─│─│─╮",
                "⏣─│─│─│─╮",
                "◯─┴─╯ │ │",
                "◯ ╭───╯ │",
                "◯ │ ╭───╯",
            ]
        );
    }

    // Ported from lazygit graph_test.go: "with a path that has room to move to the left
    // and continues" (line 144). Deep crossing with a continuing pipe.
    #[test]
    fn render_double_merge_with_immediate_first_parent_descendant() {
        let commits = [
            c("1", &["2", "3"]),
            c("3", &["2"]),
            c("2", &["4", "5"]),
            c("4", &["6", "7"]),
            c("6", &["8"]),
        ];
        let lines = render_dag(&commits);
        assert_eq!(
            lines,
            vec!["⏣─╮", "│ ◯", "⏣─│", "⏣─│─╮", "◯ │ │"]
        );
    }

    #[test]
    fn selection_no_visible_body_does_not_paint_consecutive_row() {
        // When prev commit is selected and the current row has no pipe whose `from` is
        // the selected SHA on a visible body, the highlight doesn't carry over.
        let commits = [
            c("S", &["P"]),
            c("P", &["G"]),
        ];
        let sets = pipe_sets(&commits);
        let row1 = render_pipe_set(&sets[1], Some("S"), Some("S"));
        // P's row: a single ◯, but no visible body originating from S → no highlight.
        assert_ne!(row1[0].symbol_color, SELECTED_COLOR);
    }

    #[test]
    fn selection_carries_across_continuing_pipe() {
        // Merge with parents [P, R]. Select the merge → second-parent pipe to col 1
        // is bolded white. Then on the next row (a side commit S whose parent is R),
        // the lane stays highlighted as it continues down.
        let commits = [
            c("M", &["P", "R"]),
            c("S", &["R"]),
            c("R", &["G"]),
        ];
        let sets = pipe_sets(&commits);
        let row0 = render_pipe_set(&sets[0], Some("M"), None);
        // Both starts originate from M → highlight covers ⏣─╮.
        assert_eq!(row0[0].symbol_color, SELECTED_COLOR);
        assert_eq!(row0[1].symbol_color, SELECTED_COLOR);
    }

    #[test]
    fn selection_bolds_pipes_originating_at_selected_commit() {
        let commits = [c("1", &["2"]), c("2", &["3"])];
        let sets = pipe_sets(&commits);
        let row = render_pipe_set(&sets[1], Some("2"), Some("1"));
        assert_eq!(row[0].symbol, '\u{25ef}');
        assert_eq!(row[0].symbol_color, SELECTED_COLOR);
    }

    #[test]
    fn merge_marker_at_correct_position() {
        let commits = [c("M", &["P", "S"]), c("S", &["P"]), c("P", &["G"])];
        let sets = pipe_sets(&commits);
        let row0 = render_pipe_set(&sets[0], None, None);
        assert_eq!(row0[0].symbol, '\u{23e3}');
        let row1 = render_pipe_set(&sets[1], None, None);
        assert_eq!(row1[0].symbol, '│');
        assert_eq!(row1[1].symbol, '\u{25ef}');
        let row2 = render_pipe_set(&sets[2], None, None);
        assert_eq!(row2[0].symbol, '\u{25ef}');
    }
}
