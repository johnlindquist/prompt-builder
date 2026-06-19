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

use crate::skills::Skill;

const MAX_ROWS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum SkillPopupAction {
    None,
    Insert(String),
    Cancel,
}

#[derive(Debug, Default)]
pub struct SkillPopup {
    query: String,
    selected: usize,
}

impl SkillPopup {
    pub fn handle_key(&mut self, key: KeyEvent, skills: &[Skill]) -> SkillPopupAction {
        if key.kind == KeyEventKind::Release {
            return SkillPopupAction::None;
        }

        match key.code {
            KeyCode::Esc => SkillPopupAction::Cancel,
            KeyCode::Enter | KeyCode::Tab => self
                .selected_skill(skills)
                .map_or(SkillPopupAction::Cancel, |skill| {
                    SkillPopupAction::Insert(skill.mention())
                }),
            KeyCode::Up | KeyCode::BackTab => {
                self.move_up(skills);
                SkillPopupAction::None
            }
            KeyCode::Down => {
                self.move_down(skills);
                SkillPopupAction::None
            }
            KeyCode::Backspace => {
                if self.query.is_empty() {
                    SkillPopupAction::Cancel
                } else {
                    self.query.pop();
                    self.clamp_selection(skills);
                    SkillPopupAction::None
                }
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.query.push(c);
                self.clamp_selection(skills);
                SkillPopupAction::None
            }
            _ => SkillPopupAction::None,
        }
    }

    pub fn matching_indices(&self, skills: &[Skill]) -> Vec<usize> {
        let mut matches = skills
            .iter()
            .enumerate()
            .filter_map(|(index, skill)| {
                match_score(skill, &self.query).map(|score| (score, index))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(score, index)| (score.clone(), *index));
        matches.into_iter().map(|(_, index)| index).collect()
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, skills: &[Skill]) {
        let matches = self.matching_indices(skills);
        let selected = self.selected.min(matches.len().saturating_sub(1));
        let start = selected
            .saturating_sub(MAX_ROWS - 1)
            .min(matches.len().saturating_sub(MAX_ROWS));
        let title = if self.query.is_empty() {
            "Skills".to_string()
        } else {
            format!("Skills ${}", self.query)
        };
        let items = matches
            .iter()
            .skip(start)
            .take(MAX_ROWS)
            .enumerate()
            .map(|(row, index)| render_skill_row(&skills[*index], start + row == selected))
            .collect::<Vec<_>>();
        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
        Clear.render(area, buf);
        Widget::render(list, area, buf);
    }

    pub fn required_height(&self, skills: &[Skill]) -> u16 {
        let rows = self.matching_indices(skills).len().clamp(1, MAX_ROWS);
        rows as u16 + 2
    }

    fn selected_skill<'a>(&self, skills: &'a [Skill]) -> Option<&'a Skill> {
        let matches = self.matching_indices(skills);
        matches
            .get(self.selected.min(matches.len().saturating_sub(1)))
            .and_then(|index| skills.get(*index))
    }

    fn move_up(&mut self, skills: &[Skill]) {
        let len = self.matching_indices(skills).len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected == 0 {
            self.selected = len - 1;
        } else {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self, skills: &[Skill]) {
        let len = self.matching_indices(skills).len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = (self.selected + 1) % len;
        }
    }

    fn clamp_selection(&mut self, skills: &[Skill]) {
        let len = self.matching_indices(skills).len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }
}

fn render_skill_row(skill: &Skill, selected: bool) -> ListItem<'_> {
    let marker = if selected { "> " } else { "  " };
    let mut spans = vec![marker.into(), skill.mention().cyan()];
    if !skill.description.is_empty() {
        spans.push(" ".dim());
        spans.push(skill.description.clone().dim());
    }
    ListItem::new(Line::from(spans))
}

fn match_score(skill: &Skill, query: &str) -> Option<(u8, String)> {
    let query = query.trim().to_lowercase();
    let name = skill.name.to_lowercase();
    let description = skill.description.to_lowercase();
    if query.is_empty() {
        return Some((3, name));
    }
    if name == query {
        Some((0, name))
    } else if name.starts_with(&query) {
        Some((1, name))
    } else if name.contains(&query) {
        Some((2, name))
    } else if is_subsequence(&query, &name) {
        Some((3, name))
    } else if description.contains(&query) {
        Some((4, name))
    } else {
        None
    }
}

fn is_subsequence(query: &str, value: &str) -> bool {
    let mut chars = value.chars();
    query
        .chars()
        .all(|query_char| chars.any(|c| c == query_char))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skill(name: &str, description: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: description.to_string(),
            path: PathBuf::from(format!("/tmp/{name}/SKILL.md")),
        }
    }

    #[test]
    fn filters_by_prefix_contains_and_subsequence() {
        let skills = vec![
            skill("fusion", "Run Fusion"),
            skill("gemini-design", "Generate layouts"),
            skill("oracle-packx", "Bundle context"),
        ];
        let mut popup = SkillPopup {
            query: "gd".to_string(),
            ..Default::default()
        };

        assert_eq!(popup.matching_indices(&skills), vec![1]);

        popup.query = "pack".to_string();
        assert_eq!(popup.matching_indices(&skills), vec![2]);
    }

    #[test]
    fn typing_and_backspace_update_query() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Char('f')), &skills),
            SkillPopupAction::None
        );
        assert_eq!(popup.query, "f");
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Backspace), &skills),
            SkillPopupAction::None
        );
        assert_eq!(popup.query, "");
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Backspace), &skills),
            SkillPopupAction::Cancel
        );
    }

    #[test]
    fn enter_inserts_selected_skill() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), &skills),
            SkillPopupAction::Insert("$fusion".to_string())
        );
    }

    #[test]
    fn arrows_wrap_selection() {
        let skills = vec![
            skill("fusion", "Run Fusion"),
            skill("gemini-design", "Layouts"),
        ];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Up), &skills),
            SkillPopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), &skills),
            SkillPopupAction::Insert("$gemini-design".to_string())
        );
    }

    #[test]
    fn release_events_do_not_mutate_query() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('$'),
            KeyModifiers::SHIFT,
            KeyEventKind::Release,
        );

        assert_eq!(popup.handle_key(release, &skills), SkillPopupAction::None);
        assert_eq!(popup.query, "");
    }
}
