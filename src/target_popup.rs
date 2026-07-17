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

use crate::targets::Target;
use crate::targets::TargetKind;
use crate::theme::Theme;

const MAX_ROWS: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub enum TargetPopupAction {
    None,
    Use(usize),
    EditDocument,
    Reload,
    Cancel,
}

/// Modal target manager. Pure UI state: file IO and target mutation live in
/// `targets.rs` and are orchestrated by `app.rs`.
#[derive(Debug, Default)]
pub struct TargetPopup {
    selected: usize,
    notice: Option<String>,
}

impl TargetPopup {
    pub fn new(selected: usize, target_count: usize) -> Self {
        Self {
            selected: selected.min(target_count.saturating_sub(1)),
            notice: None,
        }
    }

    pub fn selected_index(&self, target_count: usize) -> Option<usize> {
        (target_count != 0).then(|| self.selected.min(target_count - 1))
    }

    pub fn set_selected(&mut self, selected: usize, target_count: usize) {
        self.selected = selected.min(target_count.saturating_sub(1));
    }

    pub fn set_notice(&mut self, notice: impl Into<String>) {
        // Keep the popup a fixed shape: one concise line, not a full anyhow chain.
        let notice = notice.into();
        let first_line = notice.lines().next().unwrap_or_default().to_string();
        self.notice = Some(first_line);
    }

    pub fn handle_key(&mut self, key: KeyEvent, target_count: usize) -> TargetPopupAction {
        if key.kind == KeyEventKind::Release {
            return TargetPopupAction::None;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => TargetPopupAction::Cancel,
            KeyCode::Char('c') if control => TargetPopupAction::Cancel,
            KeyCode::Up => {
                self.move_by(target_count, -1);
                TargetPopupAction::None
            }
            KeyCode::Char('p') if control => {
                self.move_by(target_count, -1);
                TargetPopupAction::None
            }
            KeyCode::Down => {
                self.move_by(target_count, 1);
                TargetPopupAction::None
            }
            KeyCode::Char('n') if control => {
                self.move_by(target_count, 1);
                TargetPopupAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => self
                .selected_index(target_count)
                .map_or(TargetPopupAction::None, TargetPopupAction::Use),
            KeyCode::Char('e') if key.modifiers.is_empty() => TargetPopupAction::EditDocument,
            KeyCode::Char('g') if control => TargetPopupAction::EditDocument,
            KeyCode::Char('r') if key.modifiers.is_empty() => TargetPopupAction::Reload,
            _ => TargetPopupAction::None,
        }
    }

    pub fn required_height(&self, targets: &[Target]) -> u16 {
        let rows = targets.len().clamp(1, MAX_ROWS);
        // Rows + footer (+ notice) + borders.
        rows as u16 + self.notice.is_some() as u16 + 3
    }

    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        targets: &[Target],
        active: usize,
        theme: &Theme,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let selected = self.selected.min(targets.len().saturating_sub(1));
        let start = selected
            .saturating_sub(MAX_ROWS - 1)
            .min(targets.len().saturating_sub(MAX_ROWS));
        let mut items = targets
            .iter()
            .enumerate()
            .skip(start)
            .take(MAX_ROWS)
            .map(|(index, target)| {
                render_target_row(target, index == selected, index == active, theme)
            })
            .collect::<Vec<_>>();
        if let Some(notice) = &self.notice {
            items.push(ListItem::new(Line::from(Span::styled(
                notice.clone(),
                theme.error_style(),
            ))));
        }
        items.push(ListItem::new(Line::from(Span::styled(
            "Enter use  e edit/add/remove  r reload  Esc close",
            theme.muted_style(),
        ))));
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Targets")
                .style(theme.panel_style())
                .border_style(theme.border_style(true))
                .title_style(theme.title_style(true)),
        );
        Clear.render(area, buf);
        Widget::render(list, area, buf);
    }

    fn move_by(&mut self, target_count: usize, delta: isize) {
        if target_count == 0 {
            self.selected = 0;
            return;
        }
        self.notice = None;
        self.selected = (self.selected as isize + delta).rem_euclid(target_count as isize) as usize;
    }
}

fn render_target_row(
    target: &Target,
    selected: bool,
    active: bool,
    theme: &Theme,
) -> ListItem<'static> {
    let marker = if selected { "> " } else { "  " };
    let active_marker = if active { "* " } else { "  " };
    let mut summary = target.kind.label().to_string();
    if let Some(bin) = &target.bin {
        summary.push_str(&format!("  bin={bin}"));
    }
    if let Some(model) = &target.model {
        summary.push_str(&format!("  model={model}"));
    }
    if let Some(profile) = &target.profile {
        summary.push_str(&format!("  profile={profile}"));
    }
    if let Some(flow) = &target.flow {
        summary.push_str(&format!("  flow={flow}"));
    }
    for key in target.env.keys() {
        summary.push_str(&format!("  {key}"));
    }
    if target.kind != TargetKind::Codex && (target.profile.is_some() || !target.config.is_empty()) {
        summary.push_str("  (profile/config ignored)");
    }
    let row_style = if selected {
        theme.selected_style()
    } else {
        theme.panel_style()
    };
    let name_style = if selected {
        row_style
    } else {
        theme.text_style().add_modifier(Modifier::BOLD)
    };
    let active_style = if selected {
        row_style
    } else {
        Style::default()
            .fg(theme.green)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::BOLD)
    };
    let line = Line::from(vec![
        Span::styled(marker, row_style),
        Span::styled(active_marker, active_style),
        Span::styled(target.name.clone(), name_style),
        Span::styled("  ", row_style),
        Span::styled(
            summary,
            if selected {
                row_style
            } else {
                theme.secondary_style()
            },
        ),
    ]);
    ListItem::new(line).style(row_style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn arrows_and_ctrl_navigation_wrap() {
        let mut popup = TargetPopup::new(0, 3);

        assert_eq!(
            popup.handle_key(key(KeyCode::Up), 3),
            TargetPopupAction::None
        );
        assert_eq!(popup.selected_index(3), Some(2));
        assert_eq!(
            popup.handle_key(key(KeyCode::Down), 3),
            TargetPopupAction::None
        );
        assert_eq!(popup.selected_index(3), Some(0));
        assert_eq!(popup.handle_key(ctrl('n'), 3), TargetPopupAction::None);
        assert_eq!(popup.selected_index(3), Some(1));
        assert_eq!(popup.handle_key(ctrl('p'), 3), TargetPopupAction::None);
        assert_eq!(popup.selected_index(3), Some(0));
    }

    #[test]
    fn enter_uses_selected_target() {
        let mut popup = TargetPopup::new(1, 3);

        assert_eq!(
            popup.handle_key(key(KeyCode::Enter), 3),
            TargetPopupAction::Use(1)
        );
    }

    #[test]
    fn zero_targets_never_produces_use_action() {
        let mut popup = TargetPopup::new(0, 0);

        assert_eq!(
            popup.handle_key(key(KeyCode::Enter), 0),
            TargetPopupAction::None
        );
    }

    #[test]
    fn edit_reload_and_cancel_keys_map_to_actions() {
        let mut popup = TargetPopup::new(0, 2);

        assert_eq!(
            popup.handle_key(key(KeyCode::Char('e')), 2),
            TargetPopupAction::EditDocument
        );
        assert_eq!(
            popup.handle_key(ctrl('g'), 2),
            TargetPopupAction::EditDocument
        );
        assert_eq!(
            popup.handle_key(key(KeyCode::Char('r')), 2),
            TargetPopupAction::Reload
        );
        assert_eq!(
            popup.handle_key(key(KeyCode::Esc), 2),
            TargetPopupAction::Cancel
        );
        assert_eq!(popup.handle_key(ctrl('c'), 2), TargetPopupAction::Cancel);
    }

    #[test]
    fn release_events_do_not_change_selection() {
        let mut popup = TargetPopup::new(0, 3);
        let release =
            KeyEvent::new_with_kind(KeyCode::Down, KeyModifiers::NONE, KeyEventKind::Release);

        assert_eq!(popup.handle_key(release, 3), TargetPopupAction::None);
        assert_eq!(popup.selected_index(3), Some(0));
    }

    #[test]
    fn unrecognized_keys_are_swallowed_not_forwarded() {
        let mut popup = TargetPopup::new(0, 2);

        assert_eq!(
            popup.handle_key(key(KeyCode::Char('x')), 2),
            TargetPopupAction::None
        );
    }

    #[test]
    fn notice_is_truncated_to_first_line() {
        let mut popup = TargetPopup::default();
        popup.set_notice("first line\nsecond line");

        assert_eq!(popup.notice.as_deref(), Some("first line"));
    }

    #[test]
    fn render_shows_selection_active_marker_and_footer() {
        let targets = vec![
            Target {
                name: "pi-work".to_string(),
                kind: TargetKind::Pi,
                model: Some("openai/gpt-5".to_string()),
                profile: Some("ignored".to_string()),
                ..Target::default()
            },
            Target {
                name: "codex".to_string(),
                kind: TargetKind::Codex,
                ..Target::default()
            },
            Target {
                name: "claude".to_string(),
                kind: TargetKind::Claude,
                model: Some("opus".to_string()),
                ..Target::default()
            },
        ];
        let popup = TargetPopup::new(2, targets.len());
        let area = Rect::new(0, 0, 100, popup.required_height(&targets));
        let mut buf = Buffer::empty(area);

        popup.render(area, &mut buf, &targets, 0, &Theme::catppuccin());

        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Targets"));
        assert!(text.contains("pi-work"));
        assert!(text.contains("model=openai/gpt-5"));
        assert!(text.contains("profile/config ignored"));
        assert!(text.contains("* pi-work"));
        assert!(text.contains("> "));
        assert!(text.contains("model=opus"));
        assert!(text.contains("Enter use"));
    }
}
