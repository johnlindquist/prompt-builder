use std::io;
use std::time::Duration;

use codex_tui::ComposerAction;
use codex_tui::ComposerInput;
use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::execute;
use crossterm::terminal;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::prelude::*;
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

pub fn run(initial_prompt: String, skills: Vec<Skill>) -> anyhow::Result<AppExit> {
    let mut terminal = setup_terminal()?;
    let result = run_inner(&mut terminal, initial_prompt, skills);
    restore_terminal()?;
    result
}

fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial_prompt: String,
    skills: Vec<Skill>,
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
        terminal.draw(|frame| draw(frame, &composer, &skills, skill_popup.as_ref()))?;

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
                composer.handle_paste(text);
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
) {
    let area = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let title = Paragraph::new("Prompt Builder\nEnter sends to Codex. Ctrl+C cancels.")
        .block(Block::default().borders(Borders::ALL).title("Codex input"));
    frame.render_widget(title, layout[0]);

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

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    Terminal::new(backend).map_err(Into::into)
}

pub fn restore_terminal() -> anyhow::Result<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, Show)?;
    Ok(())
}
