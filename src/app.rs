use std::io;
use std::time::Duration;

use codex_tui::ComposerAction;
use codex_tui::ComposerInput;
use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
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
use ratatui::style::Stylize;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::Paragraph;

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

    loop {
        terminal.draw(|frame| draw(frame, &composer, &skills))?;

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
                match composer.input(key) {
                    ComposerAction::Submitted(text) => return Ok(AppExit::Submit(text)),
                    ComposerAction::None => {}
                }
            }
            Event::Paste(text) => {
                composer.handle_paste(text);
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn draw(frame: &mut Frame<'_>, composer: &ComposerInput, skills: &[Skill]) {
    let area = frame.area();
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(40), Constraint::Length(34)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(outer[0]);

    let title = Paragraph::new("Prompt Builder\nEnter sends to Codex. Ctrl+C cancels.")
        .block(Block::default().borders(Borders::ALL).title("Codex input"));
    frame.render_widget(title, left[0]);

    let composer_height = composer
        .desired_height(left[1].width)
        .clamp(3, left[1].height.max(3));
    let composer_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(composer_height)])
        .split(left[1]);
    composer.render_ref(composer_rows[1], frame.buffer_mut());
    if let Some((x, y)) = composer.cursor_pos(composer_rows[1]) {
        frame.set_cursor_position((x, y));
    }

    let skill_items = skills.iter().map(render_skill).collect::<Vec<_>>();
    let skills_list = List::new(skill_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Skills ({})", skills.len())),
    );
    frame.render_widget(skills_list, outer[1]);
}

fn render_skill(skill: &Skill) -> ListItem<'_> {
    let mut lines = vec![Line::from(format!("${}", skill.name).cyan())];
    if !skill.description.is_empty() {
        lines.push(Line::from(skill.description.clone()).dim());
    }
    ListItem::new(lines)
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
