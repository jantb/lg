//! The staged diff as a stream of tokens for the scene to read.

use super::*;

/// Which side of the diff a character came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Added,
    Removed,
    Context,
}

/// The diff, ready to be fed into the network a character at a time: what
/// the model is reading, shown going in. Header lines are left out and runs
/// of whitespace become one gap, so the stream is the code and not its
/// indentation; a very long diff is cut, as the stream loops round anyway.
/// The lines are kept as well, laid out for the grid and coloured by what
/// the code in them is, for the backdrop that scrolls the change itself past
/// the reader.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Feed {
    pub(super) chars: Vec<(char, Side)>,
    pub(super) lines: Vec<GridLine>,
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

impl Feed {
    pub fn from_diff(diff: &str) -> Self {
        let mut chars: Vec<(char, Side)> = Vec::new();
        let mut lines: Vec<GridLine> = Vec::new();
        // The file the lines below belong to, which is what says how to read
        // the code in them.
        let mut path = String::new();
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
            if lines.len() < FEED_LINE_CAP {
                lines.push(on_grid(line, &path, side));
            }
            if chars.len() < FEED_CAP {
                for c in line.chars() {
                    if c.is_whitespace() || c.is_control() {
                        if chars.last().is_some_and(|&(last, _)| last != ' ') {
                            chars.push((' ', side));
                        }
                    } else {
                        chars.push((cell(c), side));
                    }
                }
                if chars.last().is_some_and(|&(last, _)| last != ' ') {
                    chars.push((' ', side));
                }
            }
            if chars.len() >= FEED_CAP && lines.len() >= FEED_LINE_CAP {
                break;
            }
        }
        chars.truncate(FEED_CAP);
        Self { chars, lines }
    }

    /// The character in stream slot `slot`, or none when there is no diff
    /// to show and the stream falls back to abstract tokens.
    ///
    /// Slots rise towards the network, which is on the right, so a run of
    /// text laid out slot by slot would come out mirrored. Taking the diff
    /// backwards puts it the right way round for a reader: the words drift
    /// rightwards into the network and read left to right on the way. The
    /// stream is a loop either way, so all of the diff goes past.
    pub(super) fn at(&self, slot: usize) -> Option<(char, Side)> {
        if self.chars.is_empty() {
            return None;
        }
        let i = FEED_ORIGIN.saturating_sub(slot);
        self.chars.get(i % self.chars.len()).copied()
    }

    /// Whether `slot` carries anything: a gap between words carries nothing.
    pub(super) fn occupied(&self, slot: usize) -> bool {
        match self.at(slot) {
            Some((c, _)) => c != ' ',
            None => !hash(slot, 11).is_multiple_of(TOKEN_GAP),
        }
    }
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
