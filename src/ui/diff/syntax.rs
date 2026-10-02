//! Syntax colouring for the languages a diff line can be in.

use std::borrow::Cow;

use super::*;

/// The file a diff header names: the `b/` side of a `diff --git` or `+++`
/// line, which is what gives the lines under it their syntax.
pub fn diff_header_path(line: &str) -> Option<Cow<'_, str>> {
    if let Some(path) = line.strip_prefix("+++ b/") {
        // Git ends a name holding a space with a tab.
        return Some(Cow::Borrowed(path.split('\t').next().unwrap_or(path)));
    }
    crate::git::patch::diff_git_paths(line).map(|(_, new)| Cow::Owned(new))
}

pub(super) fn diff_line_syntax(line: &str) -> Option<Language> {
    Language::from_path(&diff_header_path(line)?)
}

/// What the highlighter made of a character. The colours above are meant for
/// a pane of text; a caller drawing code onto a grid in a palette of its own
/// wants the same classification without them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// The `+`, `-`, `@@` or file header a diff is structured by.
    Marker,
    Keyword,
    Type,
    Function,
    Literal,
    Comment,
    Plain,
}

/// The role of every character of the diff line `line`, whose file is at
/// `path`: one `Token` per character, in order.
pub fn diff_line_tokens(line: &str, path: &str) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(line.len());
    for span in highlight_diff_line_for_path(line, path).spans {
        let token = match span.style.fg {
            Some(Color::Yellow) => Token::Keyword,
            Some(Color::LightYellow) => Token::Literal,
            Some(Color::LightCyan | Color::LightBlue) => Token::Type,
            Some(Color::LightMagenta) => Token::Function,
            Some(Color::DarkGray) => Token::Comment,
            Some(
                Color::Green
                | Color::LightGreen
                | Color::Red
                | Color::LightRed
                | Color::Cyan
                | Color::Magenta,
            ) => Token::Marker,
            _ => Token::Plain,
        };
        tokens.extend(std::iter::repeat_n(token, span.content.chars().count()));
    }
    tokens
}
