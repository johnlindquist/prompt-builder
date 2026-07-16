use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Widget;
use tui_textarea::CursorMove;
use tui_textarea::TextArea;

use crate::file_search;
use crate::file_search::AtToken;
use crate::slash_commands;
use crate::theme::Theme;

use std::time::Duration;
use std::time::Instant;

const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;
const MAX_COMPOSER_HEIGHT: u16 = 16;

// Mirrors Codex's paste_burst.rs heuristics: characters arriving faster than a
// human can type are treated as a paste, and Enter inside that window inserts
// a newline instead of submitting a half-pasted prompt. Only matters when the
// terminal lacks bracketed paste.
const PASTE_BURST_MIN_CHARS: usize = 3;
const PASTE_BURST_CHAR_INTERVAL: Duration = Duration::from_millis(8);
const PASTE_ENTER_SUPPRESS_WINDOW: Duration = Duration::from_millis(120);

#[derive(Debug, Default)]
struct PasteBurst {
    last_fast_char: Option<Instant>,
    consecutive_fast_chars: usize,
    window_until: Option<Instant>,
}

impl PasteBurst {
    fn on_plain_char(&mut self, now: Instant) {
        let is_fast = self
            .last_fast_char
            .is_some_and(|last| now.duration_since(last) <= PASTE_BURST_CHAR_INTERVAL);
        self.consecutive_fast_chars = if is_fast {
            self.consecutive_fast_chars + 1
        } else {
            1
        };
        self.last_fast_char = Some(now);
        if self.consecutive_fast_chars >= PASTE_BURST_MIN_CHARS {
            self.window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
        }
    }

    fn newline_instead_of_submit(&mut self, now: Instant) -> bool {
        let suppress = self.window_until.is_some_and(|until| now <= until);
        if suppress {
            // A pasted newline keeps the burst alive for following chars.
            self.on_plain_char(now);
        }
        suppress
    }

    fn reset(&mut self) {
        self.last_fast_char = None;
        self.consecutive_fast_chars = 0;
        self.window_until = None;
    }
}

pub enum ComposerAction {
    Submitted(String),
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashToken {
    pub query: String,
    pub start: usize,
    pub end: usize,
    pub has_space_after: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingPaste {
    placeholder: String,
    payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DisplayRow {
    global_char_start: usize,
    text: String,
}

pub struct ComposerInput {
    textarea: TextArea<'static>,
    hint_items: Vec<(String, String)>,
    pending_pastes: Vec<PendingPaste>,
    notice: Option<String>,
    title: Option<String>,
    paste_burst: PasteBurst,
}

impl ComposerInput {
    pub fn new() -> Self {
        Self {
            textarea: new_textarea(),
            hint_items: Vec::new(),
            pending_pastes: Vec::new(),
            notice: None,
            title: None,
            paste_burst: PasteBurst::default(),
        }
    }

    pub fn set_notice(&mut self, notice: impl Into<String>) {
        self.notice = Some(notice.into());
    }

    pub fn clear_notice(&mut self) {
        self.notice = None;
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    pub fn clear_title(&mut self) {
        self.title = None;
    }

    pub fn input(&mut self, key: KeyEvent) -> ComposerAction {
        self.input_at(key, Instant::now())
    }

    fn input_at(&mut self, key: KeyEvent, now: Instant) -> ComposerAction {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return ComposerAction::None;
        }

        if is_newline_key(key) {
            self.textarea.insert_newline();
            self.reconcile_pending_pastes();
            return ComposerAction::None;
        }

        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            if self.paste_burst.newline_instead_of_submit(now) {
                self.textarea.insert_newline();
                self.reconcile_pending_pastes();
                return ComposerAction::None;
            }
            let text = self.submission_text();
            if !text.trim().is_empty() {
                self.pending_pastes.clear();
                return ComposerAction::Submitted(text);
            }
            return ComposerAction::None;
        }

        if is_plain_char_key(key) {
            self.paste_burst.on_plain_char(now);
        } else {
            self.paste_burst.reset();
        }

        if self.handle_placeholder_delete(key) {
            return ComposerAction::None;
        }

        self.textarea.input(key);
        self.reconcile_pending_pastes();
        ComposerAction::None
    }

    pub fn handle_paste(&mut self, pasted: String) -> bool {
        self.paste_burst.reset();
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

    /// Replaces the entire composer contents, dropping any pending paste
    /// payloads, and moves the cursor to the end. Used for history recall.
    pub fn set_text_end(&mut self, text: &str) {
        self.pending_pastes.clear();
        self.set_text_and_cursor(text, text.chars().count());
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

    pub fn render_ref(
        &self,
        area: Rect,
        focused: bool,
        theme: &Theme,
        skill_mentions: &[String],
        buf: &mut Buffer,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(self.title.as_deref().unwrap_or("Prompt").to_string())
            .style(theme.panel_style())
            .border_style(theme.border_style(focused))
            .title_style(theme.title_style(focused));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width > 0 && inner.height > 0 {
            clear_area(inner, theme.panel_style(), buf);

            let content_width = inner.width.max(1) as usize;
            if self.text().is_empty() {
                buf.set_stringn(
                    inner.x,
                    inner.y,
                    "Compose new task",
                    content_width,
                    theme.muted_style(),
                );
            } else {
                let text = self.text();
                let mention_ranges = skill_mention_ranges(&text, skill_mentions);
                let rows = self.display_rows(content_width);
                let (visual_row, _) = self.visual_cursor(content_width);
                let first_visible_row = first_visible_row(visual_row, inner.height as usize);
                for (offset, row) in rows
                    .iter()
                    .skip(first_visible_row)
                    .take(inner.height as usize)
                    .enumerate()
                {
                    let line = styled_display_line(row, &mention_ranges, theme);
                    buf.set_line(
                        inner.x,
                        inner.y + offset as u16,
                        &line,
                        content_width as u16,
                    );
                }
            }
        }

        if area.height == 0 {
            return;
        }

        if let Some(notice) = &self.notice {
            buf.set_line(
                area.x.saturating_add(2),
                area.bottom().saturating_sub(1),
                &Line::from(Span::styled(notice.as_str(), theme.warning_style())),
                area.width.saturating_sub(4),
            );
            return;
        }

        if self.hint_items.is_empty() {
            return;
        }

        let mut spans = Vec::new();
        for (index, (key, label)) in self.hint_items.iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled("  ", theme.muted_style()));
            }
            spans.push(Span::styled(
                key.as_str(),
                theme.title_style(false).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" ", theme.muted_style()));
            spans.push(Span::styled(label.as_str(), theme.muted_style()));
        }
        buf.set_line(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            &Line::from(spans),
            area.width.saturating_sub(4),
        );
    }

    pub fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn current_at_token(&self, allow_empty: bool) -> Option<AtToken> {
        file_search::current_at_token(&self.text(), self.cursor_char_index(), allow_empty)
    }

    pub fn current_slash_token(&self) -> Option<SlashToken> {
        current_slash_token(&self.text(), self.cursor_char_index())
    }

    pub fn replace_char_range(&mut self, start: usize, end: usize, replacement: &str) {
        let text = self.text();
        let mut updated = text.chars().take(start).collect::<String>();
        updated.push_str(replacement);
        updated.extend(text.chars().skip(end));
        let cursor = start + replacement.chars().count();
        self.set_text_and_cursor(&updated, cursor);
        self.reconcile_pending_pastes();
    }

    pub fn submission_text(&self) -> String {
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

    fn display_rows(&self, content_width: usize) -> Vec<DisplayRow> {
        let mut rows = Vec::new();
        let mut logical_line_start = 0;
        for line in self.textarea.lines() {
            rows.extend(wrap_segments(line, content_width).into_iter().map(
                |(segment_start, text)| DisplayRow {
                    global_char_start: logical_line_start + segment_start,
                    text,
                },
            ));
            logical_line_start += line.chars().count() + 1;
        }
        if rows.is_empty() {
            rows.push(DisplayRow {
                global_char_start: 0,
                text: String::new(),
            });
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

        let line = self
            .textarea
            .lines()
            .get(row)
            .map(String::as_str)
            .unwrap_or_default();
        let (row_in_line, visual_col) = visual_position_in_line(line, content_width, col);
        (rows_before + row_in_line, visual_col)
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

    pub fn cursor_char_index(&self) -> usize {
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

/// Wraps a logical line into display rows, breaking after the last space that
/// fits when possible (word wrap) and hard-breaking long words. Every source
/// character lands in exactly one row so cursor offsets stay contiguous.
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    wrap_segments(line, width)
        .into_iter()
        .map(|(_, row)| row)
        .collect()
}

fn wrap_segments(line: &str, width: usize) -> Vec<(usize, String)> {
    let width = width.max(1);
    let chars = line.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return vec![(0, String::new())];
    }

    let mut segments = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let remaining = chars.len() - start;
        if remaining <= width {
            segments.push((start, chars[start..].iter().collect()));
            break;
        }
        let window = &chars[start..start + width];
        let break_len = window
            .iter()
            .rposition(|ch| ch.is_whitespace())
            .map(|index| index + 1)
            .unwrap_or(width);
        segments.push((start, chars[start..start + break_len].iter().collect()));
        start += break_len;
    }
    segments
}

/// Maps a character column in a logical line to (row-within-line, column)
/// under the same wrapping model as `wrap_line`.
fn visual_position_in_line(line: &str, width: usize, col: usize) -> (usize, usize) {
    let segments = wrap_segments(line, width);
    for (row, (start, text)) in segments.iter().enumerate() {
        let len = text.chars().count();
        let is_last = row + 1 == segments.len();
        if col < start + len || is_last {
            return (row, col.saturating_sub(*start).min(len));
        }
    }
    (0, 0)
}

fn first_visible_row(cursor_row: usize, height: usize) -> usize {
    cursor_row.saturating_sub(height.saturating_sub(1))
}

fn clear_area(area: Rect, style: Style, buf: &mut Buffer) {
    let blank = " ".repeat(area.width as usize);
    for y in area.y..area.bottom() {
        buf.set_string(area.x, y, &blank, style);
    }
}

fn skill_mention_ranges(text: &str, mentions: &[String]) -> Vec<std::ops::Range<usize>> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut ranges = Vec::new();
    for start in 0..chars.len() {
        if chars[start] != '$' {
            continue;
        }
        for mention in mentions {
            let mention = mention.chars().collect::<Vec<_>>();
            let end = start + mention.len();
            if end <= chars.len()
                && chars[start..end] == mention
                && chars
                    .get(end)
                    .is_none_or(|ch| !(ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/')))
            {
                ranges.push(start..end);
                break;
            }
        }
    }
    ranges
}

fn styled_display_line(
    row: &DisplayRow,
    mention_ranges: &[std::ops::Range<usize>],
    theme: &Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_is_chip = false;
    for (offset, ch) in row.text.chars().enumerate() {
        let global = row.global_char_start + offset;
        let is_chip = mention_ranges.iter().any(|range| range.contains(&global));
        if !run.is_empty() && is_chip != run_is_chip {
            let style = if run_is_chip {
                theme.skill_chip_style()
            } else {
                theme.text_style()
            };
            spans.push(Span::styled(std::mem::take(&mut run), style));
        }
        run_is_chip = is_chip;
        run.push(ch);
    }
    if !run.is_empty() {
        let style = if run_is_chip {
            theme.skill_chip_style()
        } else {
            theme.text_style()
        };
        spans.push(Span::styled(run, style));
    }
    Line::from(spans)
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

fn is_plain_char_key(key: KeyEvent) -> bool {
    let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
    matches!(code, KeyCode::Char(_))
        && !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

fn is_newline_key(key: KeyEvent) -> bool {
    let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
    matches!(
        (code, modifiers),
        (KeyCode::Enter, modifiers)
            if modifiers.contains(KeyModifiers::SHIFT)
                || modifiers.contains(KeyModifiers::ALT)
                || modifiers.contains(KeyModifiers::CONTROL)
    ) || matches!(
        (code, modifiers),
        (KeyCode::Char('j'), modifiers) | (KeyCode::Char('m'), modifiers)
            if modifiers.contains(KeyModifiers::CONTROL)
    )
}

fn normalize_key_parts(code: KeyCode, mut modifiers: KeyModifiers) -> (KeyCode, KeyModifiers) {
    let KeyCode::Char(ch) = code else {
        return (code, modifiers);
    };
    if let Some(ctrl_char) = c0_control_char_to_ctrl_char(ch) {
        modifiers.insert(KeyModifiers::CONTROL);
        return (KeyCode::Char(ctrl_char), modifiers);
    }
    if ch.is_ascii_uppercase() {
        modifiers.insert(KeyModifiers::SHIFT);
        return (KeyCode::Char(ch.to_ascii_lowercase()), modifiers);
    }
    (code, modifiers)
}

pub(crate) fn normalize_key_for_binding(key: KeyEvent) -> KeyEvent {
    let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
    KeyEvent::new_with_kind(code, modifiers, key.kind)
}

fn c0_control_char_to_ctrl_char(ch: char) -> Option<char> {
    let code = u32::from(ch);
    match code {
        0x00 => Some(' '),
        0x01..=0x1a => char::from_u32(code - 0x01 + u32::from('a')),
        0x1c..=0x1f => char::from_u32(code - 0x1c + u32::from('4')),
        _ => None,
    }
}

fn current_slash_token(text: &str, cursor: usize) -> Option<SlashToken> {
    let chars = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    if chars.first() != Some(&'/') {
        return None;
    }

    let first_line_end = chars
        .iter()
        .position(|ch| *ch == '\n')
        .unwrap_or(chars.len());
    if cursor > first_line_end {
        return None;
    }
    if first_line_end > 1 && chars.get(1).is_some_and(|ch| ch.is_whitespace()) {
        return None;
    }

    let mut token_end = 1;
    while token_end < first_line_end && !chars[token_end].is_whitespace() {
        token_end += 1;
    }
    if !(1..=token_end).contains(&cursor) {
        return None;
    }

    let query = chars[1..token_end].iter().collect::<String>();
    if query.contains('/') || !slash_commands::has_command_prefix(&query) {
        return None;
    }
    let has_space_after = chars.get(token_end).is_some_and(|ch| ch.is_whitespace());
    Some(SlashToken {
        query,
        start: 0,
        end: token_end,
        has_space_after,
    })
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
    fn burst_typed_enter_still_submits() {
        let mut composer = ComposerInput::new();
        let mut now = Instant::now();
        // Human-speed typing: 100ms between chars.
        for ch in ['h', 'i'] {
            composer.input_at(KeyEvent::from(KeyCode::Char(ch)), now);
            now += Duration::from_millis(100);
        }

        match composer.input_at(KeyEvent::from(KeyCode::Enter), now) {
            ComposerAction::Submitted(text) => assert_eq!(text, "hi"),
            ComposerAction::None => panic!("typed Enter should submit"),
        }
    }

    #[test]
    fn burst_paste_enter_inserts_newline_instead_of_submitting() {
        let mut composer = ComposerInput::new();
        let mut now = Instant::now();
        // Paste-speed input: 1ms between chars.
        for ch in ['a', 'b', 'c'] {
            composer.input_at(KeyEvent::from(KeyCode::Char(ch)), now);
            now += Duration::from_millis(1);
        }

        assert!(matches!(
            composer.input_at(KeyEvent::from(KeyCode::Enter), now),
            ComposerAction::None
        ));
        now += Duration::from_millis(1);
        composer.input_at(KeyEvent::from(KeyCode::Char('d')), now);
        assert_eq!(composer.text(), "abc\nd");

        // Enter after the paste settles submits the whole thing.
        now += Duration::from_millis(500);
        match composer.input_at(KeyEvent::from(KeyCode::Enter), now) {
            ComposerAction::Submitted(text) => assert_eq!(text, "abc\nd"),
            ComposerAction::None => panic!("Enter after burst window should submit"),
        }
    }

    #[test]
    fn burst_window_survives_pasted_newline_runs() {
        let mut composer = ComposerInput::new();
        let mut now = Instant::now();
        for ch in ['a', 'b', 'c'] {
            composer.input_at(KeyEvent::from(KeyCode::Char(ch)), now);
            now += Duration::from_millis(1);
        }
        // Two consecutive pasted newlines: both must insert, not submit.
        for _ in 0..2 {
            assert!(matches!(
                composer.input_at(KeyEvent::from(KeyCode::Enter), now),
                ComposerAction::None
            ));
            now += Duration::from_millis(1);
        }

        assert_eq!(composer.text(), "abc\n\n");
    }

    #[test]
    fn bracketed_paste_resets_burst_state() {
        let mut composer = ComposerInput::new();
        let mut now = Instant::now();
        for ch in ['a', 'b', 'c'] {
            composer.input_at(KeyEvent::from(KeyCode::Char(ch)), now);
            now += Duration::from_millis(1);
        }
        composer.handle_paste(" pasted".to_string());

        match composer.input_at(KeyEvent::from(KeyCode::Enter), now) {
            ComposerAction::Submitted(text) => assert_eq!(text, "abc pasted"),
            ComposerAction::None => panic!("bracketed paste should clear burst suppression"),
        }
    }

    #[test]
    fn wrap_breaks_at_word_boundaries() {
        assert_eq!(
            wrap_line("fix the composer", 8),
            vec!["fix the ", "composer"]
        );
        assert_eq!(wrap_line("a bb ccc dddd", 5), vec!["a bb ", "ccc ", "dddd"]);
        // Long words still hard-break.
        assert_eq!(wrap_line("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrapped_cursor_follows_word_boundaries() {
        // "fix the composer": cursor after "the " (index 8) is start of row 1.
        assert_eq!(visual_position_in_line("fix the composer", 8, 8), (1, 0));
        assert_eq!(visual_position_in_line("fix the composer", 8, 7), (0, 7));
        assert_eq!(visual_position_in_line("fix the composer", 8, 16), (1, 8));
        assert_eq!(visual_position_in_line("", 8, 0), (0, 0));
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

        composer.render_ref(area, true, &Theme::catppuccin(), &[], &mut buffer);

        assert_eq!(row_text(&buffer, 1, 1, 4), "abcd");
        assert_eq!(row_text(&buffer, 1, 2, 4), "efgh");
        assert_eq!(row_text(&buffer, 1, 3, 4), "ij  ");
    }

    #[test]
    fn known_skill_mentions_render_as_theme_chips() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("use $fusion now");
        let area = Rect::new(0, 0, 24, 4);
        let mut buffer = Buffer::empty(area);
        let theme = Theme::catppuccin();

        composer.render_ref(area, true, &theme, &["$fusion".to_string()], &mut buffer);

        assert_eq!(buffer[(5, 1)].symbol(), "$ ".trim());
        assert_eq!(buffer[(5, 1)].style().bg, Some(theme.surface0));
        assert_eq!(buffer[(4, 1)].style().bg, Some(theme.panel_bg));
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
    fn replace_char_range_preserves_large_paste_payloads() {
        let mut composer = ComposerInput::new();
        let pasted = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);
        let placeholder = format!("[Pasted Content {} chars]", pasted.chars().count());
        composer.handle_paste(pasted.clone());
        composer.set_initial_text(" fix @ma");
        let token = composer
            .current_at_token(true)
            .expect("expected active @ token");

        composer.replace_char_range(token.start, token.end, "src/main.rs ");

        assert_eq!(composer.text(), format!("{placeholder} fix src/main.rs "));
        assert_eq!(
            composer.submission_text(),
            format!("{pasted} fix src/main.rs ")
        );
    }

    #[test]
    fn slash_token_detects_bare_slash() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/");

        assert_eq!(
            composer.current_slash_token(),
            Some(SlashToken {
                query: String::new(),
                start: 0,
                end: 1,
                has_space_after: false,
            })
        );
    }

    #[test]
    fn slash_token_detects_command_prefix() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/re");

        assert_eq!(
            composer.current_slash_token(),
            Some(SlashToken {
                query: "re".to_string(),
                start: 0,
                end: 3,
                has_space_after: false,
            })
        );
    }

    #[test]
    fn slash_token_ignores_plain_text_cases() {
        for text in [
            "/ test",
            " /review",
            "/etc/hosts",
            "hello /review",
            "hello\n/review",
        ] {
            let mut composer = ComposerInput::new();
            composer.set_initial_text(text);

            assert_eq!(composer.current_slash_token(), None, "{text}");
        }
    }

    #[test]
    fn slash_token_uses_fuzzy_activation_but_ignores_unknowns() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/ac");

        assert_eq!(
            composer.current_slash_token(),
            Some(SlashToken {
                query: "ac".to_string(),
                start: 0,
                end: 3,
                has_space_after: false,
            })
        );

        let mut composer = ComposerInput::new();
        composer.set_initial_text("/zzz");
        assert_eq!(composer.current_slash_token(), None);
    }

    #[test]
    fn slash_token_ends_before_args() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/review arg");
        for _ in 0.." arg".chars().count() {
            composer.input(KeyEvent::from(KeyCode::Left));
        }

        assert_eq!(
            composer.current_slash_token(),
            Some(SlashToken {
                query: "review".to_string(),
                start: 0,
                end: 7,
                has_space_after: true,
            })
        );
        composer.input(KeyEvent::from(KeyCode::Right));
        assert_eq!(composer.current_slash_token(), None);
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

    #[test]
    fn modified_enter_variants_insert_newlines() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("a");

        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('b')));
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('c')));
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('d')));

        assert_eq!(composer.text(), "a\nb\nc\nd");
    }

    #[test]
    fn codex_newline_aliases_insert_newlines() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("a");

        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('b')));
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('c')));
        assert!(matches!(
            composer.input(KeyEvent::from(KeyCode::Char('\n'))),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('d')));
        assert!(matches!(
            composer.input(KeyEvent::from(KeyCode::Char('\r'))),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('e')));

        assert_eq!(composer.text(), "a\nb\nc\nd\ne");
    }

    #[test]
    fn modified_raw_cr_and_lf_insert_newlines() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("a");

        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Char('\r'), KeyModifiers::SHIFT)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('b')));
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::SHIFT)),
            ComposerAction::None
        ));
        composer.input(KeyEvent::from(KeyCode::Char('c')));

        assert_eq!(composer.text(), "a\nb\nc");
    }

    #[test]
    fn enter_release_does_not_submit_or_insert_newline() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("a");
        let release =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::SHIFT, KeyEventKind::Release);

        assert!(matches!(composer.input(release), ComposerAction::None));
        assert_eq!(composer.text(), "a");
    }

    fn row_text(buffer: &Buffer, x: u16, y: u16, width: u16) -> String {
        (x..x + width)
            .map(|col| buffer[(col, y)].symbol())
            .collect::<Vec<_>>()
            .join("")
    }
}
