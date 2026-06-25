use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;

use crate::file_search::FileMatch;

const MAX_ROWS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum FilePopupAction {
    None,
    Accept(String),
    Cancel,
    Close,
    Forward,
}

#[derive(Debug, Default)]
pub struct FilePopup {
    query: String,
    matches: Vec<FileMatch>,
    selected: usize,
    dismissed_token: Option<String>,
}

impl FilePopup {
    pub fn set_query(&mut self, query: &str, matches: Vec<FileMatch>) {
        if self.query != query {
            self.selected = 0;
        }
        self.query = query.to_string();
        self.matches = matches;
        self.clamp_selection();
    }

    pub fn dismissed_token(&self) -> Option<&str> {
        self.dismissed_token.as_deref()
    }

    pub fn clear_dismissed_token(&mut self) {
        self.dismissed_token = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent, token: Option<&str>) -> FilePopupAction {
        if key.kind == KeyEventKind::Release {
            return FilePopupAction::None;
        }

        match key.code {
            KeyCode::Esc => {
                self.dismissed_token = token.map(str::to_string);
                FilePopupAction::Cancel
            }
            KeyCode::Up => {
                self.move_up();
                FilePopupAction::None
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_up();
                FilePopupAction::None
            }
            KeyCode::Down => {
                self.move_down();
                FilePopupAction::None
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_down();
                FilePopupAction::None
            }
            KeyCode::Tab | KeyCode::BackTab => self
                .selected_path()
                .map_or(FilePopupAction::Close, FilePopupAction::Accept),
            KeyCode::Enter if key.modifiers.is_empty() => self
                .selected_path()
                .map_or(FilePopupAction::Close, FilePopupAction::Accept),
            _ => FilePopupAction::Forward,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let title = if self.query.is_empty() {
            "Files @".to_string()
        } else {
            format!("Files @{}", self.query)
        };
        let selected = self.selected.min(self.matches.len().saturating_sub(1));
        let start = selected
            .saturating_sub(MAX_ROWS - 1)
            .min(self.matches.len().saturating_sub(MAX_ROWS));
        let items = if self.matches.is_empty() {
            vec![ListItem::new(Line::from("no matches".dim()))]
        } else {
            self.matches
                .iter()
                .skip(start)
                .take(MAX_ROWS)
                .enumerate()
                .map(|(row, file_match)| render_file_row(file_match, start + row == selected))
                .collect::<Vec<_>>()
        };
        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
        Clear.render(area, buf);
        Widget::render(list, area, buf);
    }

    pub fn required_height(&self) -> u16 {
        let rows = self.matches.len().clamp(1, MAX_ROWS);
        rows as u16 + 2
    }

    fn selected_path(&self) -> Option<String> {
        self.matches
            .get(self.selected.min(self.matches.len().saturating_sub(1)))
            .map(|file_match| file_match.path.clone())
    }

    fn move_up(&mut self) {
        let len = self.matches.len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected == 0 {
            self.selected = len - 1;
        } else {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        let len = self.matches.len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = (self.selected + 1) % len;
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.matches.len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }
}

fn render_file_row(file_match: &FileMatch, selected: bool) -> ListItem<'_> {
    let marker = if selected { "> " } else { "  " };
    ListItem::new(Line::from(vec![
        marker.into(),
        file_match.path.as_str().cyan(),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileMatch {
        FileMatch {
            path: path.to_string(),
            score: 1,
        }
    }

    #[test]
    fn arrows_wrap_selection() {
        let mut popup = FilePopup::default();
        popup.set_query("rs", vec![file("a.rs"), file("b.rs")]);

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Up), Some("rs")),
            FilePopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Tab), Some("rs")),
            FilePopupAction::Accept("b.rs".to_string())
        );
    }

    #[test]
    fn ctrl_navigation_and_enter_accept() {
        let mut popup = FilePopup::default();
        popup.set_query("rs", vec![file("a.rs"), file("b.rs")]);

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                Some("rs")
            ),
            FilePopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), Some("rs")),
            FilePopupAction::Accept("b.rs".to_string())
        );
    }

    #[test]
    fn shift_enter_forwards_to_composer() {
        let mut popup = FilePopup::default();
        popup.set_query("rs", vec![file("a.rs")]);

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                Some("rs")
            ),
            FilePopupAction::Forward
        );
    }

    #[test]
    fn esc_records_dismissed_token() {
        let mut popup = FilePopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Esc), Some("main")),
            FilePopupAction::Cancel
        );
        assert_eq!(popup.dismissed_token(), Some("main"));
    }

    #[test]
    fn enter_without_match_closes_without_accepting() {
        let mut popup = FilePopup::default();
        popup.set_query("missing", Vec::new());

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), Some("missing")),
            FilePopupAction::Close
        );
    }
}
