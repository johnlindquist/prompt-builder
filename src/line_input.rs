use crossterm::event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Widget;

use crate::theme::Theme;

/// Single-line readline-style text field rendered as a bordered 3-row block.
/// Used for the conversation Name field and mdflow flow-input form fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineInput {
    title: String,
    placeholder: String,
    text: String,
    cursor: usize,
}

impl LineInput {
    pub(crate) fn new(title: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            placeholder: placeholder.into(),
            text: String::new(),
            cursor: 0,
        }
    }

    pub(crate) fn set_text(&mut self, text: &str) {
        self.text = single_line_text(text);
        self.cursor = self.text.chars().count();
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub(crate) fn input(&mut self, key: KeyEvent) {
        if !matches!(
            key.kind,
            event::KeyEventKind::Press | event::KeyEventKind::Repeat
        ) {
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.char_len(),
            KeyCode::Char('b') if ctrl => self.move_left(),
            KeyCode::Char('f') if ctrl => self.move_right(),
            KeyCode::Char('b') if alt => self.move_word_left(),
            KeyCode::Char('f') if alt => self.move_word_right(),
            KeyCode::Char('w') if ctrl => self.delete_word_back(),
            KeyCode::Char('d') if alt => self.delete_word_forward(),
            KeyCode::Char('u') if ctrl => self.delete_to_start(),
            KeyCode::Char('k') if ctrl => self.delete_to_end(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert_char(c),
            KeyCode::Backspace if ctrl || alt => self.delete_word_back(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete if ctrl || alt => self.delete_word_forward(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left if ctrl || alt => self.move_word_left(),
            KeyCode::Right if ctrl || alt => self.move_word_right(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.char_len(),
            _ => {}
        }
    }

    pub(crate) fn handle_paste(&mut self, text: &str) {
        let normalized = single_line_text(text);
        for c in normalized.chars() {
            self.insert_char(c);
        }
    }

    pub(crate) fn render_ref(&self, area: Rect, focused: bool, theme: &Theme, buf: &mut Buffer) {
        let title = if focused {
            format!("{} *", self.title)
        } else {
            self.title.clone()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(theme.panel_style())
            .border_style(theme.border_style(focused))
            .title_style(theme.title_style(focused));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        clear_area(inner, theme.panel_style(), buf);
        let content_width = inner.width as usize;
        if self.text.is_empty() {
            buf.set_stringn(
                inner.x,
                inner.y,
                &self.placeholder,
                content_width,
                theme.muted_style(),
            );
            return;
        }

        let cursor = self.cursor.min(self.char_len());
        let first_visible = cursor.saturating_sub(content_width.saturating_sub(1));
        let visible = self
            .text
            .chars()
            .skip(first_visible)
            .take(content_width)
            .collect::<String>();
        buf.set_stringn(inner.x, inner.y, visible, content_width, theme.text_style());
    }

    pub(crate) fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.width < 3 || area.height < 3 {
            return None;
        }
        let inner = Block::default().borders(Borders::ALL).inner(area);
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let content_width = inner.width as usize;
        let cursor = self.cursor.min(self.char_len());
        let first_visible = cursor.saturating_sub(content_width.saturating_sub(1));
        let visible_col = cursor.saturating_sub(first_visible) as u16;
        Some((
            inner.x + visible_col.min(inner.width.saturating_sub(1)),
            inner.y,
        ))
    }

    fn insert_char(&mut self, c: char) {
        let byte_index = byte_index_for_char(&self.text, self.cursor);
        self.text.insert(byte_index, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = byte_index_for_char(&self.text, self.cursor - 1);
        let end = byte_index_for_char(&self.text, self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        if self.cursor >= self.char_len() {
            return;
        }
        let start = byte_index_for_char(&self.text, self.cursor);
        let end = byte_index_for_char(&self.text, self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_len());
    }

    fn word_left_index(&self) -> usize {
        let chars = self.text.chars().collect::<Vec<_>>();
        let mut index = self.cursor.min(chars.len());
        while index > 0 && chars[index - 1].is_whitespace() {
            index -= 1;
        }
        while index > 0 && !chars[index - 1].is_whitespace() {
            index -= 1;
        }
        index
    }

    fn word_right_index(&self) -> usize {
        let chars = self.text.chars().collect::<Vec<_>>();
        let mut index = self.cursor.min(chars.len());
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        while index < chars.len() && !chars[index].is_whitespace() {
            index += 1;
        }
        index
    }

    fn move_word_left(&mut self) {
        self.cursor = self.word_left_index();
    }

    fn move_word_right(&mut self) {
        self.cursor = self.word_right_index();
    }

    fn delete_char_range(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let start = byte_index_for_char(&self.text, from);
        let end = byte_index_for_char(&self.text, to);
        self.text.replace_range(start..end, "");
        if self.cursor > from {
            self.cursor = from.max(self.cursor.saturating_sub(to - from));
        }
    }

    fn delete_word_back(&mut self) {
        self.delete_char_range(self.word_left_index(), self.cursor);
    }

    fn delete_word_forward(&mut self) {
        self.delete_char_range(self.cursor, self.word_right_index());
    }

    fn delete_to_start(&mut self) {
        self.delete_char_range(0, self.cursor);
    }

    fn delete_to_end(&mut self) {
        self.delete_char_range(self.cursor, self.char_len());
    }

    fn char_len(&self) -> usize {
        self.text.chars().count()
    }
}

fn byte_index_for_char(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

pub(crate) fn single_line_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn clear_area(area: Rect, style: Style, buf: &mut Buffer) {
    let blank = " ".repeat(area.width as usize);
    for y in area.y..area.bottom() {
        buf.set_string(area.x, y, &blank, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_input() -> LineInput {
        LineInput::new("Name", "Optional conversation name")
    }

    #[test]
    fn supports_readline_word_editing() {
        let mut input = name_input();
        input.set_text("hello brave world");

        input.input(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(input.text(), "hello brave ");

        input.input(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        input.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT));
        assert_eq!(input.text(), "hello  ");

        input.input(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        input.input(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(input.text(), "");
    }

    #[test]
    fn ctrl_u_kills_to_line_start() {
        let mut input = name_input();
        input.set_text("abc def");
        input.input(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));

        input.input(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(input.text(), "def");

        input.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        input.input(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(input.text(), "def!");
    }

    #[test]
    fn paste_collapses_to_single_line() {
        let mut input = name_input();

        input.handle_paste("Fix\nthis\tthing");

        assert_eq!(input.text(), "Fix this thing");
    }
}
