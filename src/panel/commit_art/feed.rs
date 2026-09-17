//! The staged diff as a stream of tokens for the scene to read.

use super::*;

/// Which side of the diff a character came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Added,
    Removed,
    Context,
}

/// The diff, ready to be shown going in: what the model is reading.
///
/// The lines are laid out for the grid and coloured by what the code in
/// them is, both for the backdrop that scrolls the change itself past the
/// reader and for the stream that feeds the network. The stream reads them
/// from `tape`, which is the same lines run together with a break mark
/// between them, so what flows down it is the code as it stands in the
/// diff: markers, indentation, syntax colours and all. Hunk headers are
/// not code and are left out of the tape. A very long diff is cut, as the
/// stream loops round anyway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Feed {
    pub(super) lines: Vec<GridLine>,
    pub(super) tape: Vec<(char, Color)>,
    /// How far apart on the tape the lanes of the stream sit.
    pub(super) stride: usize,
}

/// A line of the diff ready to be drawn: each cell a character and the
/// colour it takes.
pub(super) type GridLine = Vec<(char, Color)>;

/// Characters of the diff kept for the stream, at most.
pub(super) const FEED_CAP: usize = 40_000;
/// Lines of the diff kept for the backdrop, at most, and how much of each:
/// the backdrop loops round, and no terminal is that wide.
pub(super) const FEED_LINE_CAP: usize = 4_000;
pub(super) const FEED_LINE_WIDTH: usize = 400;
/// Columns a tab stands for when the line is laid out on the grid.
pub(super) const TAB_WIDTH: usize = 4;
/// Slots are counted back from here into the diff; far enough ahead of any
/// slot the stream reaches in a wait.
pub(super) const FEED_ORIGIN: usize = STREAM_ORIGIN * 2;
/// The mark the tape carries where one line of the diff ends and the next
/// begins.
pub(super) const LINE_BREAK: char = '\u{21b5}';
/// The colour of that mark: dim enough to read as punctuation between the
/// lines rather than as part of them.
pub(super) const LINE_BREAK_COLOR: Color = Color::Rgb(86, 90, 116);
/// The least the lanes of the stream are set apart on the tape: wider than
/// the stream itself, so a diff of very short lines still gives every lane
/// its own run of code rather than the same one twice.
pub(super) const MIN_LANE_STRIDE: usize = MAX_STREAM_WIDTH + 8;

impl Feed {
    pub fn from_diff(diff: &str) -> Self {
        let mut lines: Vec<GridLine> = Vec::new();
        // The file the lines below belong to, which is what says how to read
        // the code in them.
        let mut path = String::new();
        let mut code = Vec::new();
        for line in diff.lines() {
            if let Some(next) = crate::ui::diff_header_path(line) {
                path = next.to_owned();
            }
            if [
                "diff --git",
                "index ",
                "--- ",
                "+++ ",
                "new file",
                "deleted file",
                "similarity",
            ]
            .iter()
            .any(|h| line.starts_with(h))
            {
                continue;
            }
            let side = match line.chars().next() {
                Some('+') => Side::Added,
                Some('-') => Side::Removed,
                _ => Side::Context,
            };
            lines.push(on_grid(line, &path, side));
            // A hunk header says where in the file the change is, which is
            // not something the model reads as code.
            if !line.starts_with("@@") {
                code.push(lines.len() - 1);
            }
            if lines.len() >= FEED_LINE_CAP {
                break;
            }
        }
        let tape = tape_of(&lines, &code);
        // A line of the diff on average is what sets one lane of the stream
        // apart from the next.
        let stride = tape.len().checked_div(code.len()).unwrap_or(0);
        Self {
            lines,
            tape,
            stride: stride.max(MIN_LANE_STRIDE),
        }
    }

    /// The character at tape position `pos` and the colour it is drawn in,
    /// or none when there is no diff to show and the stream falls back to
    /// abstract tokens.
    ///
    /// The tape is a loop, so all of the diff goes past however long the
    /// wait runs.
    pub(super) fn at(&self, pos: usize) -> Option<(char, Color)> {
        if self.tape.is_empty() {
            return None;
        }
        self.tape.get(pos % self.tape.len()).copied()
    }

    /// Whether the tape carries anything at `pos`: the gaps a line's
    /// indentation and the spaces in it leave carry nothing. With no diff
    /// to show, `seed` decides, so the abstract stream has gaps too.
    pub(super) fn occupied(&self, pos: usize, seed: u64) -> bool {
        match self.at(pos) {
            Some((c, _)) => c != ' ',
            None => !seed.is_multiple_of(TOKEN_GAP),
        }
    }

    /// How much to shift a lane of the stream by to put it `n` lanes along
    /// from the one at the wire: about `n` lines of the diff.
    pub(super) fn lane_shift(&self, n: usize) -> usize {
        n * self.stride
    }
}

/// The lines of the diff at `code` run together into one tape, with a break
/// mark where each ends. Trailing blanks are dropped: they are not code, and
/// the stream is narrow.
fn tape_of(lines: &[GridLine], code: &[usize]) -> Vec<(char, Color)> {
    let mut tape: Vec<(char, Color)> = Vec::new();
    for &i in code {
        if tape.len() >= FEED_CAP {
            break;
        }
        let line = &lines[i];
        let end = line
            .iter()
            .rposition(|&(c, _)| c != ' ')
            .map_or(0, |i| i + 1);
        tape.extend_from_slice(&line[..end]);
        tape.push((LINE_BREAK, LINE_BREAK_COLOR));
    }
    tape.truncate(FEED_CAP);
    tape
}

/// A line of the diff as it can be laid out on the grid: tabs opened out to
/// their stop, whitespace flattened to spaces, every other character put in a
/// cell it fits, and each of them coloured for what the highlighter made of
/// it and which side of the diff it is on. Long lines are cut well past any
/// terminal's width.
pub(super) fn on_grid(line: &str, path: &str, side: Side) -> GridLine {
    let tokens = crate::ui::diff_line_tokens(line, path);
    let mut out: GridLine = Vec::new();
    for (i, c) in line.chars().enumerate() {
        if out.len() >= FEED_LINE_WIDTH {
            break;
        }
        let color = code_color(tokens.get(i).copied().unwrap_or(Token::Plain), side);
        match c {
            '\t' => {
                let pad = TAB_WIDTH - out.len() % TAB_WIDTH;
                out.extend(std::iter::repeat_n((' ', color), pad));
            }
            c if c.is_whitespace() || c.is_control() => out.push((' ', color)),
            c => out.push((cell(c), color)),
        }
    }
    out
}

/// The colour a character of the diff takes in the backdrop: the scene's own
/// muted reading of what the highlighter made of it, pulled towards the
/// colour of its side of the diff so added and removed still tell apart at a
/// glance. The markers a diff is structured by are that side's colour and
/// nothing else.
pub(super) fn code_color(token: Token, side: Side) -> Color {
    let side = match side {
        Side::Added => DIFF_ADDED,
        Side::Removed => DIFF_REMOVED,
        Side::Context => DIFF_CONTEXT,
    };
    let code = match token {
        Token::Marker => return side,
        Token::Keyword => CODE_KEYWORD,
        Token::Type => CODE_TYPE,
        Token::Function => CODE_FUNCTION,
        Token::Literal => CODE_LITERAL,
        Token::Comment => CODE_COMMENT,
        Token::Plain => CODE_PLAIN,
    };
    mix(code, side, SIDE_TINT)
}
