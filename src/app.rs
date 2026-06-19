use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use codex_tui::ComposerAction;
use codex_tui::ComposerInput;
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

use crate::skill_popup::SkillPopup;
use crate::skill_popup::SkillPopupAction;
use crate::skills::Skill;

pub enum AppExit {
    Submit(String),
    Cancel,
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
    skills: Vec<Skill>,
    cwd: PathBuf,
    template: Option<TemplateInfo>,
) -> anyhow::Result<AppExit> {
    let mut terminal = setup_terminal()?;
    let header = HeaderInfo::new(&cwd, template);
    let result = run_inner(&mut terminal, initial_prompt, skills, header);
    match (result, restore_terminal()) {
        (Ok(exit), Ok(())) => Ok(exit),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err),
    }
}

fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial_prompt: String,
    skills: Vec<Skill>,
    header: HeaderInfo,
) -> anyhow::Result<AppExit> {
    let mut composer = ComposerInput::new();
    composer.set_hint_items(vec![
        ("Enter", "send"),
        ("Shift+Enter", "newline"),
        ("Ctrl+C", "quit"),
    ]);
    if !initial_prompt.is_empty() {
        composer.handle_paste(initial_prompt);
        composer.flush_paste_burst_if_due();
    }
    let mut skill_popup: Option<SkillPopup> = None;

    loop {
        terminal.draw(|frame| draw(frame, &composer, &skills, skill_popup.as_ref(), &header))?;

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
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(AppExit::Cancel);
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
                if key.code == KeyCode::Char('$')
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !skills.is_empty()
                {
                    skill_popup = Some(SkillPopup::default());
                    continue;
                }
                match composer.input(key) {
                    ComposerAction::Submitted(text) => return Ok(AppExit::Submit(text)),
                    ComposerAction::None => {}
                }
            }
            Event::Paste(text) => {
                skill_popup = None;
                composer.handle_paste(text.replace('\r', "\n"));
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn draw(
    frame: &mut Frame<'_>,
    composer: &ComposerInput,
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
        .clamp(3, layout[1].height.max(3));
    let composer_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(composer_height)])
        .split(layout[1]);
    composer.render_ref(composer_rows[1], frame.buffer_mut());
    if let Some(popup) = skill_popup {
        let popup_area = popup_area(composer_rows[1], popup, skills);
        popup.render(popup_area, frame.buffer_mut(), skills);
    }
    if let Some((x, y)) = composer.cursor_pos(composer_rows[1]) {
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
}
