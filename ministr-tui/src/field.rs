//! A single-line text field — the console's only editor.
//!
//! Used by S2's path rows and S3's path field. The field is pure state
//! (text plus a cursor column), so every editing state renders
//! deterministically under the snapshot harness. Editing is deliberately
//! small: insert at the cursor, backspace, and cursor left/right —
//! nothing a calm instrument doesn't need.

use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

/// One editable line of text with a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The field's text.
    text: String,
    /// Cursor position, counted in characters (`0..=chars`).
    cursor: usize,
}

impl Field {
    /// A field holding `text`, cursor at the end.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            cursor: text.chars().count(),
        }
    }

    /// An empty field.
    #[must_use]
    pub fn empty() -> Self {
        Self::new("")
    }

    /// The field's text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Is the field empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Insert a character at the cursor.
    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor. Returns whether anything
    /// was deleted.
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let at = self.byte_at(self.cursor - 1);
        self.text.remove(at);
        self.cursor -= 1;
        true
    }

    /// Move the cursor one character left. Returns whether it moved.
    pub fn left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Move the cursor one character right. Returns whether it moved.
    pub fn right(&mut self) -> bool {
        if self.cursor >= self.text.chars().count() {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// The field as a renderable line. An active field shows its cursor
    /// as a reversed cell (the character under it, or a space at the
    /// end); an inactive field renders plainly.
    #[must_use]
    pub fn line(&self, active: bool) -> Line<'static> {
        if !active {
            return Line::from(self.text.clone());
        }
        let at = self.byte_at(self.cursor);
        let (before, rest) = self.text.split_at(at);
        let mut chars = rest.chars();
        let under = chars.next().map_or_else(|| " ".to_owned(), String::from);
        let after: String = chars.collect();
        Line::from(vec![
            Span::from(before.to_owned()),
            Span::from(under).reversed(),
            Span::from(after),
        ])
    }

    /// Byte offset of character position `pos`.
    fn byte_at(&self, pos: usize) -> usize {
        self.text
            .char_indices()
            .nth(pos)
            .map_or(self.text.len(), |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::Field;

    #[test]
    fn insert_and_backspace_at_the_cursor() {
        let mut field = Field::new("abc");
        field.left();
        field.insert('x');
        assert_eq!(field.text(), "abxc");
        assert!(field.backspace());
        assert_eq!(field.text(), "abc");
    }

    #[test]
    fn cursor_stops_at_both_edges() {
        let mut field = Field::new("a");
        assert!(field.left());
        assert!(!field.left());
        assert!(field.right());
        assert!(!field.right());
    }

    #[test]
    fn backspace_at_the_start_deletes_nothing() {
        let mut field = Field::new("a");
        field.left();
        assert!(!field.backspace());
        assert_eq!(field.text(), "a");
    }

    #[test]
    fn multibyte_text_edits_cleanly() {
        let mut field = Field::new("héllo");
        field.left();
        field.left();
        field.backspace();
        assert_eq!(field.text(), "hélo");
    }
}
