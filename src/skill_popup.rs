use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::prelude::*;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;

use crate::skills::Skill;
use crate::theme::Theme;

const MAX_ROWS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum SkillPopupAction {
    None,
    Accept(String),
    Cancel,
    Close,
    Forward,
}

#[derive(Debug, Default)]
pub struct SkillPopup {
    query: String,
    selected: usize,
    dismissed_token: Option<String>,
}

impl SkillPopup {
    pub fn set_query(&mut self, query: &str, skills: &[Skill]) {
        if self.query != query {
            self.selected = 0;
        }
        self.query = query.to_string();
        self.clamp_selection(skills);
    }

    pub fn dismissed_token(&self) -> Option<&str> {
        self.dismissed_token.as_deref()
    }

    pub fn clear_dismissed_token(&mut self) {
        self.dismissed_token = None;
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        skills: &[Skill],
        token: Option<&str>,
    ) -> SkillPopupAction {
        if key.kind == KeyEventKind::Release {
            return SkillPopupAction::None;
        }

        match key.code {
            KeyCode::Esc => {
                self.dismissed_token = token.map(str::to_string);
                SkillPopupAction::Cancel
            }
            // Kitty-protocol terminals report Shift+Tab as Tab+SHIFT rather
            // than BackTab, so check the modifier before treating Tab as accept.
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_up(skills);
                SkillPopupAction::None
            }
            KeyCode::Tab => self.accept_selected(skills),
            KeyCode::Enter if key.modifiers.is_empty() => self.accept_selected(skills),
            KeyCode::Up | KeyCode::BackTab => {
                self.move_up(skills);
                SkillPopupAction::None
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_up(skills);
                SkillPopupAction::None
            }
            KeyCode::Down => {
                self.move_down(skills);
                SkillPopupAction::None
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_down(skills);
                SkillPopupAction::None
            }
            _ => SkillPopupAction::Forward,
        }
    }

    fn accept_selected(&self, skills: &[Skill]) -> SkillPopupAction {
        self.selected_skill(skills)
            .map_or(SkillPopupAction::Close, |skill| {
                SkillPopupAction::Accept(skill.mention())
            })
    }

    pub fn matching_indices(&self, skills: &[Skill]) -> Vec<usize> {
        matching_indices(&self.query, skills)
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, skills: &[Skill], theme: &Theme) {
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
            .map(|(row, index)| render_skill_row(&skills[*index], start + row == selected, theme))
            .collect::<Vec<_>>();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(theme.panel_style())
                .border_style(theme.border_style(true))
                .title_style(theme.title_style(true)),
        );
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

pub fn matching_indices(query: &str, skills: &[Skill]) -> Vec<usize> {
    let mut matches = skills
        .iter()
        .enumerate()
        .filter_map(|(index, skill)| match_score(skill, query).map(|score| (score, index)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|(score, index)| (score.clone(), *index));
    matches.into_iter().map(|(_, index)| index).collect()
}

fn render_skill_row(skill: &Skill, selected: bool, theme: &Theme) -> ListItem<'static> {
    let marker = if selected { "> " } else { "  " };
    let row_style = if selected {
        theme.selected_style()
    } else {
        theme.panel_style()
    };
    let mention_style = if selected {
        row_style
    } else {
        Style::default()
            .fg(theme.mauve)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::BOLD)
    };
    let mut spans = vec![
        Span::styled(marker, row_style),
        Span::styled(skill.mention(), mention_style),
    ];
    if !skill.description.is_empty() {
        spans.push(Span::styled(" ", row_style));
        spans.push(Span::styled(
            skill.description.clone(),
            if selected {
                row_style
            } else {
                theme.muted_style()
            },
        ));
    }
    ListItem::new(Line::from(spans)).style(row_style)
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
    fn typing_forwards_to_composer_and_set_query_updates_matches() {
        let skills = vec![
            skill("fusion", "Run Fusion"),
            skill("gemini-design", "Layouts"),
        ];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Char('f')), &skills, Some("")),
            SkillPopupAction::Forward
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Backspace), &skills, Some("f")),
            SkillPopupAction::Forward
        );

        popup.set_query("fus", &skills);
        assert_eq!(popup.matching_indices(&skills), vec![0]);
    }

    #[test]
    fn enter_accepts_selected_skill() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), &skills, Some("")),
            SkillPopupAction::Accept("$fusion".to_string())
        );
    }

    #[test]
    fn shift_enter_forwards_to_composer() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                &skills,
                Some("")
            ),
            SkillPopupAction::Forward
        );
    }

    #[test]
    fn arrows_and_shift_tab_wrap_selection() {
        let skills = vec![
            skill("fusion", "Run Fusion"),
            skill("gemini-design", "Layouts"),
        ];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Up), &skills, Some("")),
            SkillPopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Enter), &skills, Some("")),
            SkillPopupAction::Accept("$gemini-design".to_string())
        );

        let mut popup = SkillPopup::default();
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::BackTab), &skills, Some("")),
            SkillPopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Tab), &skills, Some("")),
            SkillPopupAction::Accept("$gemini-design".to_string())
        );
    }

    #[test]
    fn kitty_style_shift_tab_moves_up_instead_of_accepting() {
        let skills = vec![
            skill("fusion", "Run Fusion"),
            skill("gemini-design", "Layouts"),
        ];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
                &skills,
                Some("")
            ),
            SkillPopupAction::None
        );
        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Tab), &skills, Some("")),
            SkillPopupAction::Accept("$gemini-design".to_string())
        );
    }

    #[test]
    fn esc_records_dismissed_token() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::from(KeyCode::Esc), &skills, Some("fus")),
            SkillPopupAction::Cancel
        );
        assert_eq!(popup.dismissed_token(), Some("fus"));
    }

    #[test]
    fn release_events_do_not_accept() {
        let skills = vec![skill("fusion", "Run Fusion")];
        let mut popup = SkillPopup::default();
        let release =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release);

        assert_eq!(
            popup.handle_key(release, &skills, Some("")),
            SkillPopupAction::None
        );
    }
}
