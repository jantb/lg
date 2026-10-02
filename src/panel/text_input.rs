//! A line or box of text being typed, with a cursor that can move through it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Text being typed and where in it the next character goes. The cursor
/// counts characters, not bytes, and every edit keeps it inside the text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    pub text: String,
    pub cursor: usize,
}

/// What a key did to a [`TextInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// Not an editing key; the caller decides what it means.
    Unhandled,
    /// An editing key that left the text as it was, such as an arrow.
    Moved,
    /// The text changed.
    Changed,
}

impl TextInput {
    /// `text` with the cursor at its end.
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// How many characters the text has.
    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    /// Replace the text, with the cursor at its end.
    pub fn set(&mut self, text: impl Into<String>) {
        *self = Self::with_text(text);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// The cursor as a byte offset into the text.
    pub fn byte_cursor(&self) -> usize {
        self.byte_index(self.cursor)
    }

    fn byte_index(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(idx, _)| idx)
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.char_len());
    }

    pub fn insert_char(&mut self, c: char) {
        self.clamp();
        let at = self.byte_cursor();
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn insert_str(&mut self, text: &str) {
        self.clamp();
        let at = self.byte_cursor();
        self.text.insert_str(at, text);
        self.cursor += text.chars().count();
    }

    /// Remove the character before the cursor. False when there is none.
    pub fn backspace(&mut self) -> bool {
        self.clamp();
        if self.cursor == 0 {
            return false;
        }
        let start = self.byte_index(self.cursor - 1);
        let end = self.byte_cursor();
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
        true
    }

    /// Remove the character under the cursor. False when there is none.
    pub fn delete(&mut self) -> bool {
        self.clamp();
        if self.cursor >= self.char_len() {
            return false;
        }
        let start = self.byte_cursor();
        let end = self.byte_index(self.cursor + 1);
        self.text.replace_range(start..end, "");
        true
    }

    /// Remove the word before the cursor, and the spaces after it.
    pub fn delete_word_before(&mut self) {
        self.clamp();
        let before = self.before_cursor();
        let trimmed = before.trim_end();
        let start = trimmed
            .rfind(char::is_whitespace)
            .map(|at| at + trimmed[at..].chars().next().map_or(1, char::len_utf8))
            .unwrap_or(0);
        let end = self.byte_cursor();
        let removed = self.text[start..end].chars().count();
        self.text.replace_range(start..end, "");
        self.cursor -= removed;
    }

    /// Row and column of the cursor in the text as written, lines split at
    /// `\n` and nothing wrapped.
    pub fn line_and_column(&self) -> (usize, usize) {
        let before = self.before_cursor();
        let row = before.matches('\n').count();
        let column = before.rsplit('\n').next().unwrap_or("").chars().count();
        (row, column)
    }

    pub fn left(&mut self) {
        self.clamp();
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = self.cursor.saturating_add(1).min(self.char_len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.char_len();
    }

    /// Apply the editing keys every input shares: typing, Backspace, Delete,
    /// the arrows, Home and End. Keys with Ctrl held are left to the caller,
    /// which usually has its own meaning for them.
    pub fn edit_key(&mut self, key: KeyEvent) -> Edit {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(c) if !ctrl => {
                self.insert_char(c);
                Edit::Changed
            }
            KeyCode::Backspace if self.backspace() => Edit::Changed,
            KeyCode::Delete if self.delete() => Edit::Changed,
            KeyCode::Backspace | KeyCode::Delete => Edit::Moved,
            KeyCode::Left => {
                self.left();
                Edit::Moved
            }
            KeyCode::Right => {
                self.right();
                Edit::Moved
            }
            KeyCode::Home => {
                self.home();
                Edit::Moved
            }
            KeyCode::End => {
                self.end();
                Edit::Moved
            }
            _ => Edit::Unhandled,
        }
    }

    /// [`Self::edit_key`], saying only whether the key was an editing key.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.edit_key(key) != Edit::Unhandled
    }

    /// The text before the cursor, which is what a renderer measures to put
    /// the terminal cursor in the right column.
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.byte_index(self.cursor.min(self.char_len()))]
    }
}

/// Read as the text it holds. Only reading: every change goes through the
/// methods above, which keep the cursor inside the text.
impl std::ops::Deref for TextInput {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl From<&str> for TextInput {
    fn from(text: &str) -> Self {
        Self::with_text(text)
    }
}

impl From<String> for TextInput {
    fn from(text: String) -> Self {
        Self::with_text(text)
    }
}

impl PartialEq<&str> for TextInput {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_goes_in_at_the_cursor_even_between_multibyte_characters() {
        let mut input = TextInput::with_text("håp");
        input.left();
        input.insert_char('é');
        assert_eq!(input.as_str(), "håép");
        assert_eq!(input.cursor, 3);
        assert_eq!(input.before_cursor(), "håé");
    }

    #[test]
    fn backspace_and_delete_take_the_character_on_their_side() {
        let mut input = TextInput::with_text("abcd");
        input.cursor = 2;
        assert!(input.backspace());
        assert_eq!((input.as_str(), input.cursor), ("acd", 1));
        assert!(input.delete());
        assert_eq!((input.as_str(), input.cursor), ("ad", 1));

        input.home();
        assert!(!input.backspace());
        input.end();
        assert!(!input.delete());
        assert_eq!(input.as_str(), "ad");
    }

    #[test]
    fn arrows_home_and_end_keep_the_cursor_inside_the_text() {
        let mut input = TextInput::with_text("ab");
        input.right();
        assert_eq!(input.cursor, 2);
        input.home();
        input.left();
        assert_eq!(input.cursor, 0);
        input.end();
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn a_cursor_left_past_the_end_is_brought_back_before_editing() {
        let mut input = TextInput {
            text: "ab".into(),
            cursor: 10,
        };
        input.insert_char('c');
        assert_eq!((input.as_str(), input.cursor), ("abc", 3));
    }

    #[test]
    fn a_word_before_the_cursor_goes_with_the_space_after_it() {
        let mut input = TextInput::with_text("fix the bug ");
        input.delete_word_before();
        assert_eq!((input.as_str(), input.cursor), ("fix the ", 8));
        input.home();
        input.delete_word_before();
        assert_eq!(input.as_str(), "fix the ");
    }

    #[test]
    fn the_cursor_is_placed_by_line_and_column() {
        let mut input = TextInput::with_text("one\ntwo");
        assert_eq!(input.line_and_column(), (1, 3));
        input.home();
        assert_eq!(input.line_and_column(), (0, 0));
    }

    #[test]
    fn keys_say_whether_they_changed_the_text() {
        let mut input = TextInput::default();
        assert_eq!(input.edit_key(key(KeyCode::Char('x'))), Edit::Changed);
        assert_eq!(input.edit_key(key(KeyCode::Left)), Edit::Moved);
        assert_eq!(input.edit_key(key(KeyCode::Backspace)), Edit::Moved);
        assert_eq!(input.edit_key(key(KeyCode::Delete)), Edit::Changed);
        assert_eq!(input.edit_key(key(KeyCode::Enter)), Edit::Unhandled);
        assert!(!input.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
        assert_eq!(input.as_str(), "");
    }
}
