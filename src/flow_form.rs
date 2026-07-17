use crossterm::event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::flow::FieldKind;
use crate::flow::FieldSpec;
use crate::line_input::LineInput;
use crate::theme::Theme;

const FIELD_HEIGHT: u16 = 3;

/// Dynamic input form for the selected mdflow flow: one row per template
/// var / `_inputs` entry, rendered between the composer and the options row.
pub(crate) struct FlowForm {
    pub fields: Vec<FormField>,
    /// False when the flow never references `{{ _1 }}`/`{{ _prompt }}`, i.e.
    /// launching it would silently drop the composed prompt.
    pub prompt_capable: bool,
}

pub(crate) struct FormField {
    pub spec: FieldSpec,
    pub value: FieldValue,
}

pub(crate) enum FieldValue {
    /// Text and Number both edit as text; Number validates on submit.
    Text(LineInput),
    /// Index into `spec.options`.
    Select(usize),
    Confirm(bool),
}

impl FlowForm {
    pub fn new(specs: Vec<FieldSpec>, prompt_capable: bool) -> Self {
        let fields = specs
            .into_iter()
            .map(|spec| {
                let value = match spec.kind {
                    FieldKind::Select => {
                        let selected = spec
                            .default
                            .as_deref()
                            .and_then(|default| {
                                spec.options.iter().position(|option| option == default)
                            })
                            .unwrap_or(0);
                        FieldValue::Select(selected)
                    }
                    FieldKind::Confirm => {
                        FieldValue::Confirm(spec.default.as_deref() == Some("true"))
                    }
                    FieldKind::Text | FieldKind::Number => {
                        let title = if spec.required {
                            format!("{} *", spec.label)
                        } else {
                            spec.label.clone()
                        };
                        let placeholder = match (&spec.default, spec.kind) {
                            (Some(default), _) => format!("default: {default}"),
                            (None, FieldKind::Number) => "number".to_string(),
                            (None, _) => String::new(),
                        };
                        FieldValue::Text(LineInput::new(title, placeholder))
                    }
                };
                FormField { spec, value }
            })
            .collect();
        Self {
            fields,
            prompt_capable,
        }
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn height(&self) -> u16 {
        self.fields.len() as u16 * FIELD_HEIGHT
    }

    /// First field that blocks submission: a required field left empty or a
    /// number that does not parse.
    pub fn first_invalid(&self) -> Option<(usize, String)> {
        self.fields.iter().enumerate().find_map(|(index, field)| {
            let FieldValue::Text(input) = &field.value else {
                return None;
            };
            let text = input.text().trim();
            if text.is_empty() {
                return (field.spec.required && field.spec.default.is_none()).then(|| {
                    (
                        index,
                        format!("flow input {:?} is required", field.spec.name),
                    )
                });
            }
            if field.spec.kind == FieldKind::Number && text.parse::<f64>().is_err() {
                return Some((
                    index,
                    format!("flow input {:?} must be a number", field.spec.name),
                ));
            }
            None
        })
    }

    /// Collected values as (bare var name, value) pairs for --_name=value.
    /// Empty text fields fall back to the flow's own defaults by omission.
    pub fn values(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = match &field.value {
                    FieldValue::Text(input) => {
                        let text = input.text().trim();
                        if text.is_empty() {
                            return None;
                        }
                        text.to_string()
                    }
                    FieldValue::Select(index) => field.spec.options.get(*index)?.clone(),
                    FieldValue::Confirm(flag) => flag.to_string(),
                };
                Some((field.spec.name.clone(), value))
            })
            .collect()
    }

    pub fn handle_key(&mut self, index: usize, key: KeyEvent) {
        if !matches!(
            key.kind,
            event::KeyEventKind::Press | event::KeyEventKind::Repeat
        ) {
            return;
        }
        let Some(field) = self.fields.get_mut(index) else {
            return;
        };
        match &mut field.value {
            FieldValue::Text(input) => input.input(key),
            FieldValue::Select(selected) => {
                let count = field.spec.options.len();
                if count == 0 {
                    return;
                }
                match key.code {
                    KeyCode::Char(' ') | KeyCode::Right | KeyCode::Down => {
                        *selected = (*selected + 1) % count;
                    }
                    KeyCode::Left | KeyCode::Up => {
                        *selected = (*selected + count - 1) % count;
                    }
                    _ => {}
                }
            }
            FieldValue::Confirm(flag) => match key.code {
                KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => *flag = !*flag,
                _ => {}
            },
        }
    }

    pub fn handle_paste(&mut self, index: usize, text: &str) {
        if let Some(FormField {
            value: FieldValue::Text(input),
            ..
        }) = self.fields.get_mut(index)
        {
            input.handle_paste(text);
        }
    }

    /// Clears the focused text field; true if there was something to clear.
    pub fn clear_field(&mut self, index: usize) -> bool {
        if let Some(FormField {
            value: FieldValue::Text(input),
            ..
        }) = self.fields.get_mut(index)
        {
            if !input.is_empty() {
                input.clear();
                return true;
            }
        }
        false
    }

    pub fn field_area(&self, form_area: Rect, index: usize) -> Rect {
        Rect::new(
            form_area.x,
            form_area.y + index as u16 * FIELD_HEIGHT,
            form_area.width,
            FIELD_HEIGHT.min(form_area.height.saturating_sub(index as u16 * FIELD_HEIGHT)),
        )
    }

    pub fn cursor_pos(&self, form_area: Rect, index: usize) -> Option<(u16, u16)> {
        match self.fields.get(index)? {
            FormField {
                value: FieldValue::Text(input),
                ..
            } => input.cursor_pos(self.field_area(form_area, index)),
            _ => None,
        }
    }

    pub fn render(&self, form_area: Rect, focused: Option<usize>, theme: &Theme, buf: &mut Buffer) {
        for (index, field) in self.fields.iter().enumerate() {
            let area = self.field_area(form_area, index);
            if area.height == 0 {
                break;
            }
            let is_focused = focused == Some(index);
            match &field.value {
                FieldValue::Text(input) => input.render_ref(area, is_focused, theme, buf),
                FieldValue::Select(selected) => {
                    let value = field
                        .spec
                        .options
                        .get(*selected)
                        .cloned()
                        .unwrap_or_default();
                    render_choice_field(
                        area,
                        &field.spec.label,
                        &format!("‹{value}›"),
                        is_focused,
                        theme,
                        buf,
                    );
                }
                FieldValue::Confirm(flag) => {
                    let value = if *flag { "[x] yes" } else { "[ ] no" };
                    render_choice_field(area, &field.spec.label, value, is_focused, theme, buf);
                }
            }
        }
    }
}

fn render_choice_field(
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    theme: &Theme,
    buf: &mut Buffer,
) {
    use ratatui::widgets::Block;
    use ratatui::widgets::Borders;
    use ratatui::widgets::Widget;

    let title = if focused {
        format!("{label} *")
    } else {
        label.to_string()
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
    crate::line_input::clear_area(inner, theme.panel_style(), buf);
    let style = if focused {
        theme.selected_style()
    } else {
        theme.text_style()
    };
    let hint = if focused { "  Space cycle" } else { "" };
    buf.set_stringn(
        inner.x,
        inner.y,
        format!("{value}{hint}"),
        inner.width as usize,
        style,
    );
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;
    use crate::flow::extract_fields;
    use crate::flow::ExplainOutput;

    fn form_from(json: &str) -> FlowForm {
        let explain: ExplainOutput = serde_json::from_str(json).expect("parse");
        let (fields, prompt_capable) = extract_fields(&explain);
        FlowForm::new(fields, prompt_capable)
    }

    fn review_form() -> FlowForm {
        form_from(
            r#"{
                "prompt": "Review [MISSING: _focus] at high severity, max 10 files, deep=false.\n\nTask: [MISSING: _1]",
                "inputs": [
                    {"name": "_severity", "type": "select", "default": "high", "options": ["low", "high"]},
                    {"name": "_max_files", "type": "number", "default": 10},
                    {"name": "_focus", "type": "text", "message": "Area to focus on", "default": null},
                    {"name": "_deep", "type": "confirm", "default": false}
                ]
            }"#,
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn defaults_prefill_select_and_confirm() {
        let form = review_form();

        assert_eq!(form.field_count(), 4);
        assert!(matches!(form.fields[0].value, FieldValue::Select(1)));
        assert!(matches!(form.fields[3].value, FieldValue::Confirm(false)));
        assert!(form.prompt_capable);
    }

    #[test]
    fn required_empty_text_blocks_submit_and_fills_after_typing() {
        let mut form = review_form();

        let (index, message) = form.first_invalid().expect("focus is required");
        assert_eq!(index, 2);
        assert!(message.contains("focus"));

        for c in "auth".chars() {
            form.handle_key(2, key(KeyCode::Char(c)));
        }
        assert_eq!(form.first_invalid(), None);
    }

    #[test]
    fn number_field_rejects_non_numeric_text() {
        let mut form = review_form();
        for c in "auth".chars() {
            form.handle_key(2, key(KeyCode::Char(c)));
        }
        for c in "ten".chars() {
            form.handle_key(1, key(KeyCode::Char(c)));
        }

        let (index, message) = form.first_invalid().expect("bad number");
        assert_eq!(index, 1);
        assert!(message.contains("number"));
    }

    #[test]
    fn values_include_choices_and_skip_empty_optional_text() {
        let mut form = review_form();
        for c in "auth".chars() {
            form.handle_key(2, key(KeyCode::Char(c)));
        }
        form.handle_key(0, key(KeyCode::Char(' '))); // high -> low (wraps)
        form.handle_key(3, key(KeyCode::Char(' '))); // false -> true

        let values = form.values();

        assert_eq!(
            values,
            vec![
                ("severity".to_string(), "low".to_string()),
                ("focus".to_string(), "auth".to_string()),
                ("deep".to_string(), "true".to_string()),
            ]
        );
    }

    #[test]
    fn select_cycles_both_directions_and_wraps() {
        let mut form = review_form();

        form.handle_key(0, key(KeyCode::Right));
        assert!(matches!(form.fields[0].value, FieldValue::Select(0)));
        form.handle_key(0, key(KeyCode::Left));
        assert!(matches!(form.fields[0].value, FieldValue::Select(1)));
    }

    #[test]
    fn clear_field_only_affects_text_fields() {
        let mut form = review_form();
        for c in "auth".chars() {
            form.handle_key(2, key(KeyCode::Char(c)));
        }

        assert!(form.clear_field(2));
        assert!(!form.clear_field(2), "already empty");
        assert!(!form.clear_field(0), "select is not clearable");
    }

    #[test]
    fn form_height_is_three_rows_per_field() {
        assert_eq!(review_form().height(), 12);
        assert_eq!(
            form_from(r#"{"prompt": "Say hi.", "inputs": []}"#).height(),
            0
        );
    }
}
