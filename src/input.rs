//! Single-line text input with a cursor.

/// Text being edited plus a cursor, counted in chars (not bytes), always
/// within `0..=len`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    /// Start editing `value` with the cursor at the end.
    pub fn new(value: &str) -> Self {
        Self {
            value: value.to_owned(),
            cursor: value.chars().count(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map_or(self.value.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_index(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the char before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_index(self.cursor);
            self.value.remove(at);
        }
    }

    /// Delete the char under the cursor.
    pub fn delete(&mut self) {
        if self.cursor < self.value.chars().count() {
            let at = self.byte_index(self.cursor);
            self.value.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_end() {
        let input = TextInput::new("abc");
        assert_eq!((input.value(), input.cursor()), ("abc", 3));
    }

    #[test]
    fn insert_and_delete_at_cursor() {
        let mut input = TextInput::new("acd");
        input.left();
        input.left();
        input.insert('b');
        assert_eq!((input.value(), input.cursor()), ("abcd", 2));
        input.backspace();
        assert_eq!((input.value(), input.cursor()), ("acd", 1));
        input.delete();
        assert_eq!((input.value(), input.cursor()), ("ad", 1));
    }

    #[test]
    fn cursor_stays_in_bounds() {
        let mut input = TextInput::new("ab");
        input.right();
        assert_eq!(input.cursor(), 2);
        input.delete(); // at end: no-op
        assert_eq!(input.value(), "ab");
        input.home();
        input.left();
        assert_eq!(input.cursor(), 0);
        input.backspace(); // at start: no-op
        assert_eq!(input.value(), "ab");
        input.end();
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn multibyte_chars() {
        let mut input = TextInput::new("héé");
        input.left();
        input.insert('x');
        assert_eq!(input.value(), "héxé");
        input.home();
        input.delete();
        assert_eq!(input.value(), "éxé");
    }

    #[test]
    fn clear_resets_cursor() {
        let mut input = TextInput::new("abc");
        input.clear();
        assert_eq!((input.value(), input.cursor()), ("", 0));
        input.insert('z');
        assert_eq!(input.value(), "z");
    }
}
