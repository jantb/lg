//! Measuring styled text in the columns a terminal draws it in.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

/// The columns `ch` takes on screen. A control character is dropped when
/// drawn, so it takes none.
pub(crate) fn char_columns(ch: char) -> usize {
    if ch.is_control() {
        return 0;
    }
    let mut buf = [0u8; 4];
    Span::raw(&*ch.encode_utf8(&mut buf)).width()
}

pub(crate) fn text_columns(text: &str) -> usize {
    text.chars().map(char_columns).sum()
}

/// `line` broken into rows at most `width` columns wide: at the spaces
/// between words where it can be, and inside a word too long for a row of its
/// own. Leading spaces are kept, so indentation stays; spaces a break falls in
/// that do not fit the row they end are dropped. A `width` of 0 leaves the
/// line whole.
pub fn word_wrap(line: Line<'_>, width: usize) -> Vec<Line<'static>> {
    let style = line.style;
    let alignment = line.alignment;
    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |ch| (ch, span.style)))
        .collect();
    let mut rows = RowBuilder::default();
    let mut at = 0;
    while at < chars.len() {
        let space = chars[at].0.is_whitespace();
        let end = chars[at..]
            .iter()
            .position(|(ch, _)| ch.is_whitespace() != space)
            .map_or(chars.len(), |len| at + len);
        let token = &chars[at..end];
        let token_columns: usize = token.iter().map(|(ch, _)| char_columns(*ch)).sum();
        if width == 0 || rows.used + token_columns <= width {
            rows.extend(token);
        } else if space {
            for &(ch, style) in token {
                if rows.used + char_columns(ch) > width {
                    break;
                }
                rows.push(ch, style);
            }
            rows.full = true;
        } else {
            if token_columns <= width {
                rows.break_row();
            }
            for &(ch, style) in token {
                if rows.used > 0 && rows.used + char_columns(ch) > width {
                    rows.break_row();
                }
                rows.push(ch, style);
            }
        }
        at = end;
    }
    rows.finish()
        .into_iter()
        .map(|spans| {
            let mut row = Line::from(spans).style(style);
            row.alignment = alignment;
            row
        })
        .collect()
}

#[derive(Default)]
struct RowBuilder {
    rows: Vec<Vec<Span<'static>>>,
    row: Vec<Span<'static>>,
    used: usize,
    /// The row ended in spaces that did not fit: the next word starts a row.
    full: bool,
}

impl RowBuilder {
    fn push(&mut self, ch: char, style: Style) {
        if self.full {
            self.break_row();
        }
        match self.row.last_mut() {
            Some(span) if span.style == style => span.content.to_mut().push(ch),
            _ => self.row.push(Span::styled(ch.to_string(), style)),
        }
        self.used += char_columns(ch);
    }

    fn extend(&mut self, token: &[(char, Style)]) {
        for &(ch, style) in token {
            self.push(ch, style);
        }
    }

    fn break_row(&mut self) {
        self.rows.push(std::mem::take(&mut self.row));
        self.used = 0;
        self.full = false;
    }

    fn finish(mut self) -> Vec<Vec<Span<'static>>> {
        self.rows.push(self.row);
        self.rows
    }
}
