use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::PopKeyboardEnhancementFlags;
use crossterm::event::PushKeyboardEnhancementFlags;
use crossterm::execute;
use crossterm::terminal;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;

use crate::composer_input::ComposerAction;
use crate::composer_input::ComposerInput;
use crate::skill_popup::SkillPopup;
use crate::skill_popup::SkillPopupAction;
use crate::skills::Skill;

pub enum AppExit {
    Submit(SubmittedPrompt),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmittedPrompt {
    pub prompt: String,
    pub thread_name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemplateInfo {
    label: Option<String>,
    description: Option<String>,
}

impl TemplateInfo {
    pub fn from_parts(label: Option<String>, description: Option<String>) -> Option<Self> {
        if label.as_deref().unwrap_or_default().trim().is_empty()
            && description.as_deref().unwrap_or_default().trim().is_empty()
        {
            return None;
        }

        Some(Self { label, description })
    }
}

pub fn run(
    initial_prompt: String,
    initial_name: String,
    skills: Vec<Skill>,
    cwd: PathBuf,
    template: Option<TemplateInfo>,
) -> anyhow::Result<AppExit> {
    let mut terminal = setup_terminal()?;
    let header = HeaderInfo::new(&cwd, template);
    let result = run_inner(&mut terminal, initial_prompt, initial_name, skills, header);
    match (result, restore_terminal()) {
        (Ok(exit), Ok(())) => Ok(exit),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err),
    }
}

fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial_prompt: String,
    initial_name: String,
    skills: Vec<Skill>,
    header: HeaderInfo,
) -> anyhow::Result<AppExit> {
    let mut composer = ComposerInput::new();
    composer.set_hint_items(vec![
        ("Tab", "field"),
        ("Enter", "send"),
        ("Shift+Enter", "newline"),
        ("Ctrl+C", "quit"),
    ]);
    if !initial_prompt.is_empty() {
        composer.set_initial_text(&initial_prompt);
        composer.flush_paste_burst_if_due();
    }
    let mut name_input = NameInput::new();
    name_input.set_text(&initial_name);
    let mut focus = FocusTarget::Name;
    let mut skill_popup: Option<SkillPopup> = None;

    loop {
        terminal.draw(|frame| {
            draw(
                frame,
                &name_input,
                &composer,
                focus,
                &skills,
                skill_popup.as_ref(),
                &header,
            )
        })?;

        if composer.is_in_paste_burst() {
            std::thread::sleep(ComposerInput::recommended_flush_delay());
            composer.flush_paste_burst_if_due();
            continue;
        }

        if !event::poll(Duration::from_millis(250))? {
            composer.flush_paste_burst_if_due();
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                if is_ctrl_c_press(key) {
                    if handle_ctrl_c(&mut name_input, &mut composer, focus) {
                        return Ok(AppExit::Cancel);
                    }
                    continue;
                }
                if let Some(popup) = skill_popup.as_mut() {
                    match popup.handle_key(key, &skills) {
                        SkillPopupAction::None => {}
                        SkillPopupAction::Cancel => skill_popup = None,
                        SkillPopupAction::Insert(text) => {
                            skill_popup = None;
                            insert_text(&mut composer, &text);
                        }
                    }
                    continue;
                }
                if is_tab_press(key) {
                    focus = focus.toggled();
                    continue;
                }
                if focus == FocusTarget::Name {
                    if key.code == KeyCode::Enter {
                        focus = FocusTarget::Prompt;
                        continue;
                    }
                    name_input.input(key);
                    continue;
                }
                if key.code == KeyCode::Char('$')
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !skills.is_empty()
                {
                    skill_popup = Some(SkillPopup::default());
                    continue;
                }
                match composer.input(key) {
                    ComposerAction::Submitted(text) => {
                        return Ok(AppExit::Submit(SubmittedPrompt {
                            prompt: text,
                            thread_name: prefixed_thread_name(
                                name_input.text(),
                                &cwd_name_prefix(&header.cwd),
                            ),
                        }));
                    }
                    ComposerAction::None => {}
                }
            }
            Event::Paste(text) => {
                skill_popup = None;
                if focus == FocusTarget::Name {
                    name_input.handle_paste(&text);
                } else {
                    composer.handle_paste(text);
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn is_ctrl_c_press(key: KeyEvent) -> bool {
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_tab_press(key: KeyEvent) -> bool {
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
}

fn handle_ctrl_c(
    name_input: &mut NameInput,
    composer: &mut ComposerInput,
    focus: FocusTarget,
) -> bool {
    match focus {
        FocusTarget::Name if !name_input.is_empty() => {
            name_input.clear();
            false
        }
        FocusTarget::Prompt if !composer.is_empty() => {
            composer.clear();
            false
        }
        _ if !composer.is_empty() => {
            composer.clear();
            false
        }
        _ if !name_input.is_empty() => {
            name_input.clear();
            false
        }
        _ => true,
    }
}

pub fn prefixed_thread_name(raw: &str, cwd: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty() {
        return None;
    }
    let prefix = format!("{cwd}:");
    if name.starts_with(&prefix) {
        Some(name.to_string())
    } else {
        Some(format!("{prefix}{name}"))
    }
}

fn cwd_name_prefix(cwd: &str) -> String {
    let path = Path::new(cwd);
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| cwd.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    Name,
    Prompt,
}

impl FocusTarget {
    fn toggled(self) -> Self {
        match self {
            Self::Name => Self::Prompt,
            Self::Prompt => Self::Name,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct NameInput {
    text: String,
    cursor: usize,
}

impl NameInput {
    fn new() -> Self {
        Self::default()
    }

    fn set_text(&mut self, text: &str) {
        self.text = single_line_text(text);
        self.cursor = self.text.chars().count();
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn input(&mut self, key: KeyEvent) {
        if !matches!(
            key.kind,
            event::KeyEventKind::Press | event::KeyEventKind::Repeat
        ) {
            return;
        }

        match key.code {
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_char(c)
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.char_len(),
            _ => {}
        }
    }

    fn handle_paste(&mut self, text: &str) {
        let normalized = single_line_text(text);
        for c in normalized.chars() {
            self.insert_char(c);
        }
    }

    fn render_ref(&self, area: Rect, focused: bool, buf: &mut Buffer) {
        let title = if focused { "Name *" } else { "Name" };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        clear_area(inner, buf);
        let content_width = inner.width as usize;
        if self.text.is_empty() {
            buf.set_stringn(
                inner.x,
                inner.y,
                "Optional conversation name",
                content_width,
                Style::default().dim(),
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
        buf.set_stringn(inner.x, inner.y, visible, content_width, Style::default());
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
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

fn single_line_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clear_area(area: Rect, buf: &mut Buffer) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buf[(x, y)].reset();
        }
    }
}

fn draw(
    frame: &mut Frame<'_>,
    name_input: &NameInput,
    composer: &ComposerInput,
    focus: FocusTarget,
    skills: &[Skill],
    skill_popup: Option<&SkillPopup>,
    header: &HeaderInfo,
) {
    let area = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header.height()), Constraint::Min(1)])
        .split(area);

    frame.render_widget(
        Paragraph::new(header.lines()).block(Block::default().borders(Borders::ALL)),
        layout[0],
    );

    let composer_height = composer
        .desired_height(layout[1].width)
        .clamp(3, layout[1].height.saturating_sub(3).max(3));
    let input_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(composer_height),
        ])
        .split(layout[1]);

    name_input.render_ref(
        input_rows[1],
        focus == FocusTarget::Name,
        frame.buffer_mut(),
    );
    composer.render_ref(input_rows[2], frame.buffer_mut());
    if let Some(popup) = skill_popup {
        let popup_area = popup_area(input_rows[2], popup, skills);
        popup.render(popup_area, frame.buffer_mut(), skills);
    }
    let cursor = if focus == FocusTarget::Name {
        name_input.cursor_pos(input_rows[1])
    } else {
        composer.cursor_pos(input_rows[2])
    };
    if let Some((x, y)) = cursor {
        frame.set_cursor_position((x, y));
    }
}

fn insert_text(composer: &mut ComposerInput, text: &str) {
    for c in text.chars() {
        let key = KeyEvent::from(KeyCode::Char(c));
        let _ = composer.input(key);
    }
}

fn popup_area(composer_area: Rect, popup: &SkillPopup, skills: &[Skill]) -> Rect {
    let width = composer_area.width.saturating_sub(2).clamp(20, 64);
    let height = popup
        .required_height(skills)
        .min(composer_area.y.saturating_sub(1))
        .max(3);
    let x = composer_area.x.saturating_add(1);
    let y = composer_area.y.saturating_sub(height);
    Rect::new(x, y, width, height)
}

struct HeaderInfo {
    cwd: String,
    git: String,
    template: Option<TemplateInfo>,
}

impl HeaderInfo {
    fn new(cwd: &Path, template: Option<TemplateInfo>) -> Self {
        Self {
            cwd: display_cwd(cwd),
            git: git_status(cwd),
            template,
        }
    }

    fn height(&self) -> u16 {
        if self.template.is_some() {
            4
        } else {
            3
        }
    }

    fn lines(&self) -> Vec<Line<'_>> {
        let mut lines = vec![Line::from(self.cwd.clone())];
        if let Some(template) = &self.template {
            lines.push(template_line(template));
        }
        lines.push(Line::from(vec!["git  ".into(), self.git.clone().into()]));
        lines
    }
}

fn template_line(template: &TemplateInfo) -> Line<'_> {
    let label = template
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or("Template");
    let description = template
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .unwrap_or_default();
    Line::from(vec![
        format!("{label}: ").bold(),
        description.to_string().into(),
    ])
}

fn display_cwd(cwd: &Path) -> String {
    let path = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(cwd))
            .unwrap_or_else(|_| cwd.to_path_buf())
    };
    path.canonicalize()
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn git_status(cwd: &Path) -> String {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--short", "--branch"])
        .output()
    else {
        return "git unavailable".to_string();
    };
    if !output.status.success() {
        return "not a git repo".to_string();
    }

    summarize_git_status(String::from_utf8_lossy(&output.stdout).lines())
}

fn summarize_git_status<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut branch = "unknown".to_string();
    let mut changed = 0usize;
    for line in lines {
        if let Some(rest) = line.strip_prefix("## ") {
            branch = rest.split("...").next().unwrap_or(rest).trim().to_string();
        } else if !line.trim().is_empty() {
            changed += 1;
        }
    }

    if changed == 0 {
        format!("{branch} clean")
    } else if changed == 1 {
        format!("{branch} 1 change")
    } else {
        format!("{branch} {changed} changes")
    }
}

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let _ = execute!(
        io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    Terminal::new(backend).map_err(Into::into)
}

pub fn restore_terminal() -> anyhow::Result<()> {
    let mut first_error = None;
    let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    if let Err(err) = execute!(io::stdout(), DisableBracketedPaste) {
        first_error.get_or_insert_with(|| anyhow::Error::from(err));
    }
    if let Err(err) = terminal::disable_raw_mode() {
        first_error.get_or_insert_with(|| anyhow::Error::from(err));
    }
    if let Err(err) = execute!(io::stdout(), LeaveAlternateScreen, Show) {
        first_error.get_or_insert_with(|| anyhow::Error::from(err));
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_info_ignores_blank_metadata() {
        assert_eq!(TemplateInfo::from_parts(None, None), None);
        assert_eq!(
            TemplateInfo::from_parts(Some(" ".to_string()), Some("\t".to_string())),
            None
        );
    }

    #[test]
    fn template_info_keeps_label_and_description() {
        assert_eq!(
            TemplateInfo::from_parts(
                Some("Fix".to_string()),
                Some("Run Fusion and verify.".to_string())
            ),
            Some(TemplateInfo {
                label: Some("Fix".to_string()),
                description: Some("Run Fusion and verify.".to_string()),
            })
        );
    }

    #[test]
    fn composer_shift_enter_newlines_and_enter_submits() {
        let mut composer = ComposerInput::new();

        assert!(matches!(
            composer.input(KeyEvent::from(KeyCode::Char('a'))),
            ComposerAction::None
        ));
        std::thread::sleep(ComposerInput::recommended_flush_delay());
        composer.flush_paste_burst_if_due();
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            ComposerAction::None
        ));
        std::thread::sleep(ComposerInput::recommended_flush_delay());
        composer.flush_paste_burst_if_due();
        assert!(matches!(
            composer.input(KeyEvent::from(KeyCode::Char('b'))),
            ComposerAction::None
        ));
        std::thread::sleep(ComposerInput::recommended_flush_delay());
        composer.flush_paste_burst_if_due();
        match composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            ComposerAction::Submitted(text) => assert_eq!(text, "a\nb"),
            ComposerAction::None => panic!("plain Enter should submit"),
        }
    }

    #[test]
    fn ctrl_c_clears_nonempty_composer_before_canceling() {
        let mut name_input = NameInput::new();
        let mut composer = ComposerInput::new();
        composer.set_initial_text("draft");

        assert!(!handle_ctrl_c(
            &mut name_input,
            &mut composer,
            FocusTarget::Prompt
        ));
        assert!(composer.is_empty());
        assert!(handle_ctrl_c(
            &mut name_input,
            &mut composer,
            FocusTarget::Prompt
        ));
    }

    #[test]
    fn ctrl_c_clears_nonempty_name_before_canceling() {
        let mut name_input = NameInput::new();
        let mut composer = ComposerInput::new();
        name_input.set_text("draft name");

        assert!(!handle_ctrl_c(
            &mut name_input,
            &mut composer,
            FocusTarget::Name
        ));
        assert!(name_input.is_empty());
        assert!(handle_ctrl_c(
            &mut name_input,
            &mut composer,
            FocusTarget::Name
        ));
    }

    #[test]
    fn ctrl_c_release_is_ignored() {
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            event::KeyEventKind::Release,
        );

        assert!(!is_ctrl_c_press(release));
    }

    #[test]
    fn tab_press_is_focus_navigation() {
        assert!(is_tab_press(KeyEvent::from(KeyCode::Tab)));
        assert!(is_tab_press(KeyEvent::from(KeyCode::BackTab)));
        assert!(!is_tab_press(KeyEvent::from(KeyCode::Char('\t'))));
    }

    #[test]
    fn name_input_is_single_line() {
        let mut input = NameInput::new();

        input.handle_paste("Fix\nthis\tthing");

        assert_eq!(input.text(), "Fix this thing");
    }

    #[test]
    fn prefixed_thread_name_uses_cwd_basename_and_skips_blanks() {
        let prefix = cwd_name_prefix("/tmp/project");

        assert_eq!(prefixed_thread_name("  ", &prefix), None);
        assert_eq!(
            prefixed_thread_name("Fix the bug", &prefix),
            Some("project:Fix the bug".to_string())
        );
        assert_eq!(
            prefixed_thread_name("project:Fix the bug", &prefix),
            Some("project:Fix the bug".to_string())
        );
    }
}
