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

use crate::slash_commands;
use crate::slash_commands::SlashCommand;

const MAX_ROWS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum SlashPopupAction {
    None,
    Accept(String),
    Cancel,
    Close,
    Forward,
}

#[derive(Debug, Default)]
pub struct SlashPopup {
    query: String,
    matches: Vec<&'static SlashCommand>,
    selected: usize,
    dismissed_token: Option<String>,
}

impl SlashPopup {
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.selected = 0;
        }
        self.query = query.to_string();
        self.matches = slash_commands::popup_matches(query);
        self.clamp_selection();
    }

    pub fn dismissed_token(&self) -> Option<&str> {
        self.dismissed_token.as_deref()
    }

    pub fn clear_dismissed_token(&mut self) {
        self.dismissed_token = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent, token: Option<&str>) -> SlashPopupAction {
        if key.kind == KeyEventKind::Release {
            return SlashPopupAction::None;
        }

        match key.code {
            KeyCode::Esc => {
                self.dismissed_token = token.map(str::to_string);
                SlashPopupAction::Cancel
            }
            KeyCode::Up => {
                self.move_up();
                SlashPopupAction::None
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_up();
                SlashPopupAction::None
            }
            KeyCode::Down => {
                self.move_down();
                SlashPopupAction::None
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_down();
                SlashPopupAction::None
            }
            KeyCode::Tab | KeyCode::BackTab => self.accept_selected(),
            KeyCode::Enter if key.modifiers.is_empty() => self.accept_selected(),
            KeyCode::Char('/') if key.modifiers.is_empty() => self.accept_selected(),
            _ => SlashPopupAction::Forward,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let title = if self.query.is_empty() {
            "Commands /".to_string()
        } else {
            format!("Commands /{}", self.query)
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
                .map(|(row, command)| render_command_row(command, start + row == selected))
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

    fn selected_command(&self) -> Option<&'static SlashCommand> {
        self.matches
            .get(self.selected.min(self.matches.len().saturating_sub(1)))
            .copied()
    }

    fn accept_selected(&self) -> SlashPopupAction {
        self.selected_command()
            .map_or(SlashPopupAction::Close, |command| {
                SlashPopupAction::Accept(command.name.to_string())
            })
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

fn render_command_row(command: &SlashCommand, selected: bool) -> ListItem<'_> {
    let marker = if selected { "> " } else { "  " };
    let mut spans = vec![marker.into(), format!("/{}", command.name).cyan()];
    if !command.description.is_empty() {
        spans.push(" ".dim());
        spans.push(command.description.dim());
    }
    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_accepts_selected_command() {
        let mut popup = SlashPopup::default();
        popup.set_query("re");

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), Some("re")),
            SlashPopupAction::Accept("review".to_string())
        );
    }

    #[test]
    fn shift_enter_forwards_to_composer() {
        let mut popup = SlashPopup::default();
        popup.set_query("re");

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                Some("re")
            ),
            SlashPopupAction::Forward
        );
    }

    #[test]
    fn ctrl_navigation_and_tab_accept() {
        let mut popup = SlashPopup::default();
        popup.set_query("m");

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                Some("m")
            ),
            SlashPopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Tab), Some("m")),
            SlashPopupAction::Accept("memories".to_string())
        );
    }

    #[test]
    fn esc_records_dismissed_token() {
        let mut popup = SlashPopup::default();
        popup.set_query("re");

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Esc), Some("re")),
            SlashPopupAction::Cancel
        );
        assert_eq!(popup.dismissed_token(), Some("re"));
    }

    #[test]
    fn slash_key_accepts_selected_command() {
        let mut popup = SlashPopup::default();
        popup.set_query("m");

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Char('/')), Some("m")),
            SlashPopupAction::Accept("model".to_string())
        );
    }
}
