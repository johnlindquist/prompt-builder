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

use crate::flow::FlowEntry;
use crate::theme::Theme;

const MAX_ROWS: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub enum FlowPopupAction {
    None,
    /// Use the flow at this catalog index; `None` selects "No flow".
    Accept(Option<usize>),
    Reload,
    Cancel,
}

/// Modal fuzzy picker over discovered mdflow flows. Row 0 is always the
/// pinned "No flow" choice; typing narrows the flow rows beneath it.
#[derive(Debug, Default)]
pub struct FlowPopup {
    query: String,
    selected: usize,
    notice: Option<String>,
}

impl FlowPopup {
    pub fn new(active: Option<usize>, flows: &[FlowEntry]) -> Self {
        let mut popup = Self::default();
        if let Some(active) = active {
            let rows = popup.matches(flows);
            if let Some(row) = rows.iter().position(|entry| *entry == Some(active)) {
                popup.selected = row;
            }
        }
        popup
    }

    pub fn set_notice(&mut self, notice: impl Into<String>) {
        let notice = notice.into();
        let first_line = notice.lines().next().unwrap_or_default().to_string();
        self.notice = Some(first_line);
    }

    /// Visible rows for the current query: `None` = the pinned "No flow"
    /// row, `Some(index)` = `flows[index]`.
    pub fn matches(&self, flows: &[FlowEntry]) -> Vec<Option<usize>> {
        let mut rows = vec![None];
        let mut scored = flows
            .iter()
            .enumerate()
            .filter_map(|(index, flow)| match_score(flow, &self.query).map(|score| (score, index)))
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| a.cmp(b));
        rows.extend(scored.into_iter().map(|(_, index)| Some(index)));
        rows
    }

    pub fn handle_key(&mut self, key: KeyEvent, flows: &[FlowEntry]) -> FlowPopupAction {
        if key.kind == KeyEventKind::Release {
            return FlowPopupAction::None;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let row_count = self.matches(flows).len();

        match key.code {
            KeyCode::Esc => FlowPopupAction::Cancel,
            KeyCode::Char('c') if control => FlowPopupAction::Cancel,
            KeyCode::Char('r') if control => FlowPopupAction::Reload,
            KeyCode::Char('p') if control => {
                self.move_by(row_count, -1);
                FlowPopupAction::None
            }
            KeyCode::Char('n') if control => {
                self.move_by(row_count, 1);
                FlowPopupAction::None
            }
            KeyCode::Up => {
                self.move_by(row_count, -1);
                FlowPopupAction::None
            }
            KeyCode::Down => {
                self.move_by(row_count, 1);
                FlowPopupAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                let rows = self.matches(flows);
                rows.get(self.selected.min(rows.len().saturating_sub(1)))
                    .map_or(FlowPopupAction::None, |row| FlowPopupAction::Accept(*row))
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
                FlowPopupAction::None
            }
            KeyCode::Char(c) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.query.push(c);
                self.selected = 0;
                FlowPopupAction::None
            }
            _ => FlowPopupAction::None,
        }
    }

    pub fn required_height(&self, flows: &[FlowEntry]) -> u16 {
        let rows = self.matches(flows).len().clamp(1, MAX_ROWS);
        // Rows + query line + footer (+ notice) + borders.
        rows as u16 + 1 + self.notice.is_some() as u16 + 3
    }

    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        flows: &[FlowEntry],
        active: Option<usize>,
        theme: &Theme,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let rows = self.matches(flows);
        let selected = self.selected.min(rows.len().saturating_sub(1));
        let start = selected
            .saturating_sub(MAX_ROWS - 1)
            .min(rows.len().saturating_sub(MAX_ROWS));

        let mut items = Vec::new();
        let query_label = if self.query.is_empty() {
            Line::from(Span::styled("type to filter flows", theme.muted_style()))
        } else {
            Line::from(vec![
                Span::styled("filter: ", theme.muted_style()),
                Span::styled(self.query.clone(), theme.text_style()),
            ])
        };
        items.push(ListItem::new(query_label));
        items.extend(
            rows.iter()
                .skip(start)
                .take(MAX_ROWS)
                .enumerate()
                .map(|(offset, row)| {
                    let row_index = start + offset;
                    self.render_row(*row, flows, row_index == selected, active, theme)
                }),
        );
        if let Some(notice) = &self.notice {
            items.push(ListItem::new(Line::from(Span::styled(
                notice.clone(),
                theme.muted_style(),
            ))));
        }
        items.push(ListItem::new(Line::from(Span::styled(
            "Enter use  Ctrl+R reload  Esc close",
            theme.muted_style(),
        ))));

        Clear.render(area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Flows")
            .style(theme.panel_style())
            .border_style(theme.border_style(true))
            .title_style(theme.title_style(true));
        Widget::render(List::new(items).block(block), area, buf);
    }

    fn render_row(
        &self,
        row: Option<usize>,
        flows: &[FlowEntry],
        selected: bool,
        active: Option<usize>,
        theme: &Theme,
    ) -> ListItem<'static> {
        let marker = if row == active && (row.is_some() || active.is_none()) {
            "● "
        } else {
            "  "
        };
        let mut spans = vec![Span::styled(
            marker.to_string(),
            if selected {
                theme.selected_style()
            } else {
                theme.muted_style()
            },
        )];
        match row.and_then(|index| flows.get(index)) {
            None => {
                spans.push(Span::styled(
                    "No flow (launch target directly)".to_string(),
                    if selected {
                        theme.selected_style()
                    } else {
                        theme.text_style()
                    },
                ));
            }
            Some(flow) => {
                spans.push(Span::styled(
                    flow.name.clone(),
                    if selected {
                        theme.selected_style()
                    } else {
                        theme.text_style()
                    },
                ));
                if let Some(engine) = flow.engine.as_deref().filter(|engine| !engine.is_empty()) {
                    spans.push(Span::styled(format!("  [{engine}]"), theme.muted_style()));
                }
                if let Some(description) = flow
                    .description
                    .as_deref()
                    .filter(|description| !description.trim().is_empty())
                {
                    spans.push(Span::styled(
                        format!("  {description}"),
                        theme.muted_style(),
                    ));
                }
            }
        }
        ListItem::new(Line::from(spans))
    }

    fn move_by(&mut self, row_count: usize, step: isize) {
        if row_count == 0 {
            return;
        }
        let count = row_count as isize;
        self.selected = (self.selected as isize + step).rem_euclid(count) as usize;
    }
}

fn match_score(flow: &FlowEntry, query: &str) -> Option<(u8, String)> {
    let query = query.trim().to_lowercase();
    let name = flow.name.to_lowercase();
    let description = flow
        .description
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if query.is_empty() {
        // Preserve catalog order (frecency-ranked) when unfiltered.
        return Some((0, String::new()));
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

    fn flow(name: &str, description: &str) -> FlowEntry {
        FlowEntry {
            name: name.to_string(),
            path: format!("/tmp/flows/{name}"),
            description: (!description.is_empty()).then(|| description.to_string()),
            ..FlowEntry::default()
        }
    }

    fn flows() -> Vec<FlowEntry> {
        vec![
            flow("review.md", "Review changes"),
            flow("commit.md", "Write commit notes"),
            flow("release-notes.md", "Draft release notes"),
        ]
    }

    #[test]
    fn no_flow_row_is_always_first_even_when_filtering() {
        let mut popup = FlowPopup::default();
        for c in "review".chars() {
            popup.handle_key(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                &flows(),
            );
        }

        let rows = popup.matches(&flows());

        assert_eq!(rows[0], None);
        assert_eq!(rows[1], Some(0));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn unfiltered_rows_preserve_catalog_order() {
        let popup = FlowPopup::default();

        assert_eq!(
            popup.matches(&flows()),
            vec![None, Some(0), Some(1), Some(2)]
        );
    }

    #[test]
    fn description_matches_rank_below_name_matches() {
        let mut popup = FlowPopup::default();
        for c in "notes".chars() {
            popup.handle_key(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                &flows(),
            );
        }

        let rows = popup.matches(&flows());

        // "release-notes.md" contains "notes" in the name; "commit.md" only
        // in its description.
        assert_eq!(rows, vec![None, Some(2), Some(1)]);
    }

    #[test]
    fn enter_accepts_selection_and_esc_cancels() {
        let mut popup = FlowPopup::default();
        let flows = flows();
        popup.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &flows);

        assert_eq!(
            popup.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &flows),
            FlowPopupAction::Accept(Some(0))
        );
        assert_eq!(
            popup.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &flows),
            FlowPopupAction::Cancel
        );
    }

    #[test]
    fn enter_on_first_row_selects_no_flow() {
        let mut popup = FlowPopup::default();

        assert_eq!(
            popup.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &flows()),
            FlowPopupAction::Accept(None)
        );
    }

    #[test]
    fn plain_r_types_into_query_and_ctrl_r_reloads() {
        let mut popup = FlowPopup::default();
        let flows = flows();

        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                &flows
            ),
            FlowPopupAction::None
        );
        assert_eq!(
            popup.handle_key(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                &flows
            ),
            FlowPopupAction::Reload
        );
    }

    #[test]
    fn new_preselects_the_active_flow_row() {
        let popup = FlowPopup::new(Some(1), &flows());

        let rows = popup.matches(&flows());
        assert_eq!(rows[popup.selected], Some(1));
    }
}
