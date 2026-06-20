use std::time::Duration;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Widget;
use tui_textarea::CursorMove;
use tui_textarea::TextArea;

const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;
const MAX_COMPOSER_HEIGHT: u16 = 16;

pub enum ComposerAction {
    Submitted(String),
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingPaste {
    placeholder: String,
    payload: String,
}

pub struct ComposerInput {
    textarea: TextArea<'static>,
    hint_items: Vec<(String, String)>,
    pending_pastes: Vec<PendingPaste>,
}

impl ComposerInput {
    pub fn new() -> Self {
        Self {
            textarea: new_textarea(),
            hint_items: Vec::new(),
            pending_pastes: Vec::new(),
        }
    }

    pub fn input(&mut self, key: KeyEvent) -> ComposerAction {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return ComposerAction::None;
        }

        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
            let text = self.submission_text();
            if !text.trim().is_empty() {
                self.pending_pastes.clear();
                return ComposerAction::Submitted(text);
            }
            return ComposerAction::None;
        }

        if self.handle_placeholder_delete(key) {
            return ComposerAction::None;
        }

        self.textarea.input(key);
        self.reconcile_pending_pastes();
        ComposerAction::None
    }

    pub fn handle_paste(&mut self, pasted: String) -> bool {
        let pasted = normalize_pasted_text(&pasted);
        let char_count = pasted.chars().count();
        if char_count > LARGE_PASTE_CHAR_THRESHOLD {
            let placeholder = self.next_large_paste_placeholder(char_count);
            self.textarea.insert_str(&placeholder);
            self.pending_pastes.push(PendingPaste {
                placeholder,
                payload: pasted,
            });
        } else {
            self.textarea.insert_str(pasted);
        }
        true
    }

    pub fn set_initial_text(&mut self, text: &str) {
        self.textarea.insert_str(text);
    }

    pub fn is_empty(&self) -> bool {
        self.text().trim().is_empty() && self.pending_pastes.is_empty()
    }

    pub fn clear(&mut self) {
        self.textarea = new_textarea();
        self.pending_pastes.clear();
    }

    pub fn set_hint_items(&mut self, items: Vec<(impl Into<String>, impl Into<String>)>) {
        self.hint_items = items
            .into_iter()
            .map(|(key, label)| (key.into(), label.into()))
            .collect();
    }

    pub fn desired_height(&self, width: u16) -> u16 {
        let content_width = width.saturating_sub(2).max(1) as usize;
        let text_rows = self.wrapped_rows(content_width) as u16;
        text_rows.saturating_add(3).min(MAX_COMPOSER_HEIGHT)
    }

    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.width < 3 || area.height < 3 {
            return None;
        }

        let inner = Block::default().borders(Borders::ALL).inner(area);
        if inner.width == 0 || inner.height == 0 {
            return None;
        }

        let content_width = inner.width.max(1) as usize;
        let (visual_row, visual_col) = self.visual_cursor(content_width);
        let first_visible_row = first_visible_row(visual_row, inner.height as usize);
        let visible_row = visual_row.saturating_sub(first_visible_row);
        Some((
            inner.x + (visual_col as u16).min(inner.width.saturating_sub(1)),
            inner.y + (visible_row as u16).min(inner.height.saturating_sub(1)),
        ))
    }

    pub fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default().borders(Borders::ALL).title("Prompt");
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width > 0 && inner.height > 0 {
            clear_area(inner, buf);

            let content_width = inner.width.max(1) as usize;
            if self.text().is_empty() {
                buf.set_stringn(
                    inner.x,
                    inner.y,
                    "Compose new task",
                    content_width,
                    Style::default().dim(),
                );
            } else {
                let rows = self.display_rows(content_width);
                let (visual_row, _) = self.visual_cursor(content_width);
                let first_visible_row = first_visible_row(visual_row, inner.height as usize);
                for (offset, row) in rows
                    .iter()
                    .skip(first_visible_row)
                    .take(inner.height as usize)
                    .enumerate()
                {
                    buf.set_stringn(
                        inner.x,
                        inner.y + offset as u16,
                        row,
                        content_width,
                        Style::default(),
                    );
                }
            }
        }

        if self.hint_items.is_empty() || area.height == 0 {
            return;
        }

        let mut spans = Vec::new();
        for (index, (key, label)) in self.hint_items.iter().enumerate() {
            if index > 0 {
                spans.push("  ".dim());
            }
            spans.push(key.as_str().bold());
            spans.push(" ".dim());
            spans.push(label.as_str().dim());
        }
        buf.set_line(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            &Line::from(spans),
            area.width.saturating_sub(4),
        );
    }

    pub fn is_in_paste_burst(&self) -> bool {
        false
    }

    pub fn flush_paste_burst_if_due(&mut self) -> bool {
        false
    }

    pub fn recommended_flush_delay() -> Duration {
        Duration::from_millis(25)
    }

    fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    fn submission_text(&self) -> String {
        let mut text = self.text();
        for paste in &self.pending_pastes {
            text = text.replacen(&paste.placeholder, &paste.payload, 1);
        }
        text
    }

    fn next_large_paste_placeholder(&self, char_count: usize) -> String {
        let base = format!("[Pasted Content {char_count} chars]");
        if self
            .pending_pastes
            .iter()
            .all(|paste| paste.placeholder != base)
        {
            return base;
        }

        let mut suffix = 2usize;
        loop {
            let placeholder = format!("[Pasted Content {char_count} chars #{suffix}]");
            if self
                .pending_pastes
                .iter()
                .all(|paste| paste.placeholder != placeholder)
            {
                return placeholder;
            }
            suffix += 1;
        }
    }

    fn display_rows(&self, content_width: usize) -> Vec<String> {
        let mut rows = Vec::new();
        for line in self.textarea.lines() {
            rows.extend(wrap_line(line, content_width));
        }
        if rows.is_empty() {
            rows.push(String::new());
        }
        rows
    }

    fn wrapped_rows(&self, content_width: usize) -> usize {
        self.display_rows(content_width).len().max(1)
    }

    fn visual_cursor(&self, content_width: usize) -> (usize, usize) {
        let content_width = content_width.max(1);
        let (row, col) = self.textarea.cursor();
        let rows_before = self
            .textarea
            .lines()
            .iter()
            .take(row)
            .map(|line| wrap_line(line, content_width).len())
            .sum::<usize>();

        let visual_row = rows_before + (col / content_width);
        let visual_col = col % content_width;
        (visual_row, visual_col)
    }

    fn reconcile_pending_pastes(&mut self) {
        if self.pending_pastes.is_empty() {
            return;
        }
        let text = self.text();
        self.pending_pastes
            .retain(|paste| text.contains(&paste.placeholder));
    }

    fn handle_placeholder_delete(&mut self, key: KeyEvent) -> bool {
        if !matches!(key.code, KeyCode::Backspace | KeyCode::Delete) {
            return false;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }

        let text = self.text();
        let cursor = self.cursor_char_index();
        let mut matched = None;
        for paste in &self.pending_pastes {
            let Some(start) = char_index_of(&text, &paste.placeholder) else {
                continue;
            };
            let end = start + paste.placeholder.chars().count();
            let should_delete = match key.code {
                KeyCode::Backspace => start < cursor && cursor <= end,
                KeyCode::Delete => start <= cursor && cursor < end,
                _ => false,
            };
            if should_delete {
                matched = Some((paste.placeholder.clone(), start, end));
                break;
            }
        }

        let Some((placeholder, start, end)) = matched else {
            return false;
        };

        let updated = remove_char_range(&text, start, end);
        self.set_text_and_cursor(&updated, start);
        self.pending_pastes
            .retain(|paste| paste.placeholder != placeholder);
        true
    }

    fn cursor_char_index(&self) -> usize {
        let (row, col) = self.textarea.cursor();
        self.textarea
            .lines()
            .iter()
            .take(row)
            .map(|line| line.chars().count() + 1)
            .sum::<usize>()
            + col
    }

    fn set_text_and_cursor(&mut self, text: &str, cursor_char_index: usize) {
        let lines = text.split('\n').map(str::to_string).collect::<Vec<_>>();
        self.textarea = if lines.is_empty() {
            new_textarea()
        } else {
            lines.into_iter().collect()
        };
        self.textarea.set_placeholder_text("Compose new task");

        let (row, col) = row_col_for_char_index(text, cursor_char_index);
        for _ in 0..row {
            self.textarea.move_cursor(CursorMove::Down);
        }
        for _ in 0..col {
            self.textarea.move_cursor(CursorMove::Forward);
        }
    }
}

impl Default for ComposerInput {
    fn default() -> Self {
        Self::new()
    }
}

fn new_textarea() -> TextArea<'static> {
    let mut textarea = TextArea::default();
    textarea.set_placeholder_text("Compose new task");
    textarea
}

fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if line.is_empty() {
        return vec![String::new()];
    }

    let chars = line.chars().collect::<Vec<_>>();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn first_visible_row(cursor_row: usize, height: usize) -> usize {
    cursor_row.saturating_sub(height.saturating_sub(1))
}

fn clear_area(area: Rect, buf: &mut Buffer) {
    let blank = " ".repeat(area.width as usize);
    for y in area.y..area.bottom() {
        buf.set_string(area.x, y, &blank, Style::default());
    }
}

fn char_index_of(text: &str, needle: &str) -> Option<usize> {
    let byte_index = text.find(needle)?;
    Some(text[..byte_index].chars().count())
}

fn remove_char_range(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .enumerate()
        .filter_map(|(index, ch)| (index < start || index >= end).then_some(ch))
        .collect()
}

fn row_col_for_char_index(text: &str, char_index: usize) -> (usize, usize) {
    let mut remaining = char_index;
    for (row, line) in text.split('\n').enumerate() {
        let line_len = line.chars().count();
        if remaining <= line_len {
            return (row, remaining);
        }
        remaining = remaining.saturating_sub(line_len + 1);
    }
    let row = text.split('\n').count().saturating_sub(1);
    let col = text
        .split('\n')
        .next_back()
        .map(str::chars)
        .map(Iterator::count)
        .unwrap_or_default();
    (row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_paste_inserts_placeholder_and_expands_on_submit() {
        let mut composer = ComposerInput::new();
        let pasted = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);

        composer.handle_paste(pasted.clone());

        let placeholder = format!("[Pasted Content {} chars]", pasted.chars().count());
        assert_eq!(composer.text(), placeholder);
        assert_eq!(composer.pending_pastes.len(), 1);
        assert_eq!(composer.submission_text(), pasted);
    }

    #[test]
    fn two_same_size_large_pastes_expand_to_distinct_payloads() {
        let mut composer = ComposerInput::new();
        let first = "a".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);
        let second = "b".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);

        composer.handle_paste(first.clone());
        composer.handle_paste(second.clone());

        assert_eq!(composer.pending_pastes.len(), 2);
        assert_ne!(
            composer.pending_pastes[0].placeholder,
            composer.pending_pastes[1].placeholder
        );
        assert_eq!(composer.submission_text(), format!("{first}{second}"));
    }

    #[test]
    fn small_paste_inserts_raw_text() {
        let mut composer = ComposerInput::new();

        composer.handle_paste("hello".to_string());

        assert_eq!(composer.text(), "hello");
        assert!(composer.pending_pastes.is_empty());
        assert_eq!(composer.submission_text(), "hello");
    }

    #[test]
    fn crlf_paste_normalizes_to_single_newline() {
        let mut composer = ComposerInput::new();

        composer.handle_paste("a\r\nb\rc".to_string());

        assert_eq!(composer.text(), "a\nb\nc");
    }

    #[test]
    fn wrap_and_cursor_use_same_visual_model() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("abcdefghij");

        assert_eq!(wrap_line("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(composer.visual_cursor(4), (2, 2));
    }

    #[test]
    fn render_wraps_long_lines_instead_of_clipping_horizontally() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("abcdefghij");
        let area = Rect::new(0, 0, 6, 5);
        let mut buffer = Buffer::empty(area);

        composer.render_ref(area, &mut buffer);

        assert_eq!(row_text(&buffer, 1, 1, 4), "abcd");
        assert_eq!(row_text(&buffer, 1, 2, 4), "efgh");
        assert_eq!(row_text(&buffer, 1, 3, 4), "ij  ");
    }

    #[test]
    fn desired_height_accounts_for_wrapped_rows() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text(&"x".repeat(100));

        assert_eq!(composer.desired_height(22), 8);
    }

    #[test]
    fn clear_resets_text_and_pending_pastes() {
        let mut composer = ComposerInput::new();
        composer.handle_paste("x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1));

        composer.clear();

        assert!(composer.is_empty());
        assert_eq!(composer.text(), "");
        assert!(composer.pending_pastes.is_empty());
    }

    #[test]
    fn editing_large_paste_placeholder_drops_hidden_payload() {
        let mut composer = ComposerInput::new();
        let pasted = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);
        composer.handle_paste(pasted);
        let visible_before_delete = composer.text();

        composer.input(KeyEvent::from(KeyCode::Backspace));

        assert!(composer.pending_pastes.is_empty());
        assert_ne!(composer.submission_text(), visible_before_delete);
    }

    #[test]
    fn backspace_deletes_large_paste_placeholder_as_one_unit() {
        let mut composer = ComposerInput::new();
        composer.handle_paste("x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1));

        composer.input(KeyEvent::from(KeyCode::Backspace));

        assert_eq!(composer.text(), "");
        assert!(composer.pending_pastes.is_empty());
    }

    #[test]
    fn delete_deletes_large_paste_placeholder_as_one_unit() {
        let mut composer = ComposerInput::new();
        let pasted = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);
        composer.handle_paste(pasted);
        let placeholder_len = composer.text().chars().count();
        for _ in 0..placeholder_len {
            composer.input(KeyEvent::from(KeyCode::Left));
        }

        composer.input(KeyEvent::from(KeyCode::Delete));

        assert_eq!(composer.text(), "");
        assert!(composer.pending_pastes.is_empty());
    }

    #[test]
    fn initial_text_never_uses_large_paste_placeholder() {
        let mut composer = ComposerInput::new();
        let initial = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);

        composer.set_initial_text(&initial);

        assert_eq!(composer.text(), initial);
        assert!(composer.pending_pastes.is_empty());
    }

    #[test]
    fn release_event_does_not_mutate_text() {
        let mut composer = ComposerInput::new();
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert!(matches!(composer.input(release), ComposerAction::None));
        assert_eq!(composer.text(), "");
    }

    fn row_text(buffer: &Buffer, x: u16, y: u16, width: u16) -> String {
        (x..x + width)
            .map(|col| buffer[(col, y)].symbol())
            .collect::<Vec<_>>()
            .join("")
    }
}
