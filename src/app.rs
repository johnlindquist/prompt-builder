use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableFocusChange;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::EnableFocusChange;
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
use crossterm::Command as CrosstermCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::prelude::*;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;

use crate::cli::enabled_option_argv;
use crate::cli::ToggleOption;
use crate::composer_input::normalize_key_for_binding;
use crate::composer_input::ComposerAction;
use crate::composer_input::ComposerInput;
use crate::file_popup::FilePopup;
use crate::file_popup::FilePopupAction;
use crate::file_search;
use crate::history::History;
use crate::skill_popup::SkillPopup;
use crate::skill_popup::SkillPopupAction;
use crate::skills::Skill;
use crate::slash_popup::SlashPopup;
use crate::slash_popup::SlashPopupAction;
use crate::target_popup::TargetPopup;
use crate::target_popup::TargetPopupAction;
use crate::targets::Target;
use crate::theme::LoadedTheme;
use crate::theme::Theme;

pub enum AppExit {
    Submit(Box<SubmittedPrompt>),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmittedPrompt {
    pub prompt: String,
    /// User-visible submitted Name without the automatic cwd prefix.
    pub conversation_name: Option<String>,
    /// Existing target/session name including the automatic cwd prefix.
    pub thread_name: Option<String>,
    pub toggled_argv: Vec<String>,
    pub target: Option<Target>,
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

#[allow(clippy::too_many_arguments)]
pub fn run(
    initial_prompt: String,
    initial_name: String,
    skills: Vec<Skill>,
    cwd: PathBuf,
    template: Option<TemplateInfo>,
    launch_options: Vec<ToggleOption>,
    targets: Vec<Target>,
    initial_target: usize,
    loaded_theme: LoadedTheme,
    debug_keys: Option<PathBuf>,
) -> anyhow::Result<AppExit> {
    let mut terminal = setup_terminal()?;
    let header = HeaderInfo::new(&cwd, template);
    let result = run_inner(
        &mut terminal,
        initial_prompt,
        initial_name,
        skills,
        header,
        launch_options,
        targets,
        initial_target,
        loaded_theme,
        debug_keys,
    );
    match (result, restore_terminal()) {
        (Ok(exit), Ok(())) => Ok(exit),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial_prompt: String,
    initial_name: String,
    skills: Vec<Skill>,
    header: HeaderInfo,
    mut launch_options: Vec<ToggleOption>,
    mut targets: Vec<Target>,
    initial_target: usize,
    loaded_theme: LoadedTheme,
    debug_keys: Option<PathBuf>,
) -> anyhow::Result<AppExit> {
    let theme = loaded_theme.theme;
    let mut target_index = initial_target.min(targets.len().saturating_sub(1));
    let mut target_popup: Option<TargetPopup> = None;
    let mut pending_target_edit: Option<PendingTargetEdit> = None;
    let mut composer = ComposerInput::new();
    if let Some(diagnostic) = loaded_theme.diagnostic {
        composer.set_notice(diagnostic);
    }
    composer.set_hint_items(vec![
        ("Tab", "field"),
        ("Enter", "send"),
        ("Shift+Enter", "newline"),
        ("↑", "history"),
        ("Ctrl+R", "search"),
        ("Ctrl+G", "editor"),
        ("Ctrl+C", "quit"),
    ]);
    if !initial_prompt.is_empty() {
        composer.set_initial_text(&initial_prompt);
    }
    let mut name_input = NameInput::new();
    name_input.set_text(&initial_name);
    let mut focus = initial_focus();
    let mut skill_mentions = skills.iter().map(Skill::mention).collect::<Vec<_>>();
    skill_mentions.sort_by_key(|mention| std::cmp::Reverse(mention.chars().count()));
    let mut skill_popup: Option<SkillPopup> = None;
    let mut file_popup: Option<FilePopup> = None;
    let mut slash_popup: Option<SlashPopup> = None;
    let mut cached_files: Option<Vec<String>> = None;
    let mut history = History::default_paths();
    let mut history_search: Option<HistorySearchState> = None;
    let mut key_debug = debug_keys.map(KeyDebug::create).transpose()?;
    if let Some(key_debug) = key_debug.as_mut() {
        key_debug.log_startup();
    }

    loop {
        terminal.draw(|frame| {
            draw(
                frame,
                DrawState {
                    name_input: &name_input,
                    composer: &composer,
                    focus,
                    skills: &skills,
                    skill_popup: skill_popup.as_ref(),
                    file_popup: file_popup.as_ref(),
                    slash_popup: slash_popup.as_ref(),
                    header: &header,
                    launch_options: &launch_options,
                    targets: &targets,
                    target_index,
                    target_popup: target_popup.as_ref(),
                    theme: &theme,
                    skill_mentions: &skill_mentions,
                },
            )
        })?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        // Drain every queued event before redrawing so large pastes and fast
        // typing render once instead of once per character.
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) => {
                    let lines_before = composer_line_count(&composer);
                    if matches!(
                        key.kind,
                        event::KeyEventKind::Press | event::KeyEventKind::Repeat
                    ) {
                        composer.clear_notice();
                    }
                    if target_popup.is_some() {
                        let action = target_popup
                            .as_mut()
                            .map(|popup| popup.handle_key(key, targets.len()))
                            .unwrap_or(TargetPopupAction::None);
                        match action {
                            TargetPopupAction::None => {}
                            TargetPopupAction::Cancel => target_popup = None,
                            TargetPopupAction::Use(index) => {
                                target_index = index.min(targets.len().saturating_sub(1));
                                target_popup = None;
                            }
                            TargetPopupAction::Reload => {
                                if let Some(popup) = target_popup.as_mut() {
                                    let previous = targets.get(target_index).cloned();
                                    match crate::targets::load_targets() {
                                        Ok(reloaded) => {
                                            target_index = reselect_target_index(
                                                previous.as_ref(),
                                                target_index,
                                                &reloaded,
                                            );
                                            targets = reloaded;
                                            popup.set_selected(target_index, targets.len());
                                            popup.set_notice("targets reloaded");
                                            pending_target_edit = None;
                                        }
                                        Err(err) => popup.set_notice(format!("reload: {err:#}")),
                                    }
                                }
                            }
                            TargetPopupAction::EditDocument => {
                                if let Some(popup) = target_popup.as_mut() {
                                    edit_targets_document(
                                        terminal,
                                        popup,
                                        &mut targets,
                                        &mut target_index,
                                        &mut pending_target_edit,
                                    )?;
                                }
                            }
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "target_popup",
                            "handled",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if let Some(search) = history_search.as_mut() {
                        if !handle_search_key(search, &mut history, &mut composer, key) {
                            history_search = None;
                            composer.clear_title();
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "history_search",
                            "handled",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if is_ctrl_r_press(key) && focus == FocusTarget::Prompt {
                        skill_popup = None;
                        file_popup = None;
                        slash_popup = None;
                        let search = HistorySearchState::new(composer.submission_text());
                        composer.set_title(search.title(true));
                        history_search = Some(search);
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "history_search",
                            "open",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if is_ctrl_g_press(key) && focus == FocusTarget::Prompt {
                        skill_popup = None;
                        file_popup = None;
                        slash_popup = None;
                        let seed = composer.submission_text();
                        let edited = with_suspended_terminal(terminal, || {
                            crate::external_editor::edit_text(&seed)
                        })?;
                        match edited {
                            Ok(text) => composer.set_text_end(&text),
                            Err(err) => composer.set_notice(format!("editor: {err}")),
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "app",
                            "external_editor",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if is_ctrl_d_press(key) && focus == FocusTarget::Prompt && composer.is_empty() {
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "app",
                            "cancel_eof",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        return Ok(AppExit::Cancel);
                    }
                    if is_ctrl_c_press(key) {
                        let draft = composer.submission_text();
                        if handle_ctrl_c(&mut name_input, &mut composer, focus) {
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "app",
                                "cancel",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            return Ok(AppExit::Cancel);
                        }
                        // A cleared draft stays recoverable via Up-arrow history,
                        // matching Codex's Ctrl+C behavior.
                        if composer.is_empty() && !draft.trim().is_empty() {
                            history.record(&draft);
                        }
                        file_popup = None;
                        slash_popup = None;
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "app",
                            "clear",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if let Some(popup) = skill_popup.as_mut() {
                        match popup.handle_key(key, &skills) {
                            SkillPopupAction::None => {
                                log_key_debug(
                                    &mut key_debug,
                                    focus,
                                    key,
                                    "skill_popup",
                                    "handled",
                                    lines_before,
                                    composer_line_count(&composer),
                                );
                                continue;
                            }
                            SkillPopupAction::Cancel => {
                                skill_popup = None;
                                log_key_debug(
                                    &mut key_debug,
                                    focus,
                                    key,
                                    "skill_popup",
                                    "cancel",
                                    lines_before,
                                    composer_line_count(&composer),
                                );
                                continue;
                            }
                            SkillPopupAction::Insert(text) => {
                                skill_popup = None;
                                insert_text(&mut composer, &text);
                                log_key_debug(
                                    &mut key_debug,
                                    focus,
                                    key,
                                    "skill_popup",
                                    "insert",
                                    lines_before,
                                    composer_line_count(&composer),
                                );
                                continue;
                            }
                            SkillPopupAction::Forward => {}
                        }
                    }
                    if file_popup.is_some() {
                        match handle_file_popup_key(&mut file_popup, &mut composer, key) {
                            FileKeyOutcome::Handled => {
                                log_key_debug(
                                    &mut key_debug,
                                    focus,
                                    key,
                                    "file_popup",
                                    "handled",
                                    lines_before,
                                    composer_line_count(&composer),
                                );
                                continue;
                            }
                            FileKeyOutcome::Forward => {}
                        }
                    }
                    if slash_popup.is_some() {
                        match handle_slash_popup_key(&mut slash_popup, &mut composer, key) {
                            SlashKeyOutcome::Handled => {
                                log_key_debug(
                                    &mut key_debug,
                                    focus,
                                    key,
                                    "slash_popup",
                                    "handled",
                                    lines_before,
                                    composer_line_count(&composer),
                                );
                                continue;
                            }
                            SlashKeyOutcome::Forward => {}
                        }
                    }
                    if is_tab_press(key) {
                        focus = focus.next(launch_options.len(), !targets.is_empty());
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "app",
                            "focus_next",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if focus == FocusTarget::Name {
                        file_popup = None;
                        slash_popup = None;
                        if is_plain_enter_press(key) {
                            focus = FocusTarget::Prompt;
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "name",
                                "focus_prompt",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            continue;
                        }
                        name_input.input(key);
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "name",
                            "input",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if focus == FocusTarget::TargetSelect {
                        if is_ctrl_g_press(key) {
                            skill_popup = None;
                            file_popup = None;
                            slash_popup = None;
                            target_popup = Some(TargetPopup::new(target_index, targets.len()));
                            if let Some(popup) = target_popup.as_mut() {
                                edit_targets_document(
                                    terminal,
                                    popup,
                                    &mut targets,
                                    &mut target_index,
                                    &mut pending_target_edit,
                                )?;
                            }
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "target_select",
                                "open_editor",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            continue;
                        }
                        if matches!(
                            key.kind,
                            event::KeyEventKind::Press | event::KeyEventKind::Repeat
                        ) {
                            match key.code {
                                KeyCode::Char(' ') | KeyCode::Right | KeyCode::Down => {
                                    target_index =
                                        next_target_index(target_index, targets.len(), 1);
                                }
                                KeyCode::Left | KeyCode::Up => {
                                    target_index =
                                        next_target_index(target_index, targets.len(), -1);
                                }
                                KeyCode::Enter => {
                                    skill_popup = None;
                                    file_popup = None;
                                    slash_popup = None;
                                    target_popup =
                                        Some(TargetPopup::new(target_index, targets.len()));
                                }
                                _ => {}
                            }
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "target_select",
                            "handled",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if let FocusTarget::Options(index) = focus {
                        match key.code {
                            KeyCode::Char(' ')
                                if matches!(
                                    key.kind,
                                    event::KeyEventKind::Press | event::KeyEventKind::Repeat
                                ) =>
                            {
                                if let Some(option) = launch_options.get_mut(index) {
                                    option.enabled = !option.enabled;
                                }
                            }
                            KeyCode::Enter
                                if matches!(
                                    key.kind,
                                    event::KeyEventKind::Press | event::KeyEventKind::Repeat
                                ) =>
                            {
                                focus = focus.next(launch_options.len(), !targets.is_empty());
                            }
                            _ => {}
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "options",
                            "handled",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if key.code == KeyCode::Char('$')
                        && matches!(
                            key.kind,
                            event::KeyEventKind::Press | event::KeyEventKind::Repeat
                        )
                        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                        && !skills.is_empty()
                        && file_popup.is_none()
                        && slash_popup.is_none()
                    {
                        skill_popup = Some(SkillPopup::default());
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "app",
                            "open_skill_popup",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if skill_popup.is_none() && file_popup.is_none() && slash_popup.is_none() {
                        if let Some(text) = handle_history_key(&mut history, &composer, key) {
                            composer.set_text_end(&text);
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "history",
                                "recall",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            continue;
                        }
                    }
                    match composer.input(key) {
                        ComposerAction::Submitted(text) => {
                            history.record(&text);
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "composer",
                                "submit",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            let conversation_name = submitted_conversation_name(name_input.text());
                            let thread_name = conversation_name.as_deref().and_then(|name| {
                                prefixed_thread_name(name, &cwd_name_prefix(&header.cwd))
                            });
                            return Ok(AppExit::Submit(Box::new(SubmittedPrompt {
                                prompt: text,
                                conversation_name,
                                thread_name,
                                toggled_argv: enabled_option_argv(&launch_options),
                                target: targets.get(target_index).cloned(),
                            })));
                        }
                        ComposerAction::None => {
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "composer",
                                composer_action_label(lines_before, composer_line_count(&composer)),
                                lines_before,
                                composer_line_count(&composer),
                            );
                        }
                    }
                    sync_file_popup(
                        &mut file_popup,
                        &composer,
                        &mut cached_files,
                        &header.cwd_path,
                    );
                    sync_slash_popup(&mut slash_popup, &composer);
                }
                Event::Paste(text) => {
                    if target_popup.is_some() {
                        continue;
                    }
                    let lines_before = composer_line_count(&composer);
                    skill_popup = None;
                    file_popup = None;
                    slash_popup = None;
                    match focus {
                        FocusTarget::Name => name_input.handle_paste(&text),
                        FocusTarget::Prompt => {
                            composer.handle_paste(text);
                            sync_file_popup(
                                &mut file_popup,
                                &composer,
                                &mut cached_files,
                                &header.cwd_path,
                            );
                            sync_slash_popup(&mut slash_popup, &composer);
                        }
                        FocusTarget::TargetSelect | FocusTarget::Options(_) => {}
                    }
                    log_event_debug(
                        &mut key_debug,
                        "paste",
                        focus,
                        "event",
                        "paste",
                        lines_before,
                        composer_line_count(&composer),
                    );
                }
                Event::Resize(_, _) => {
                    let lines = composer_line_count(&composer);
                    log_event_debug(
                        &mut key_debug,
                        "resize",
                        focus,
                        "event",
                        "resize",
                        lines,
                        lines,
                    );
                }
                Event::FocusGained => {
                    let lines = composer_line_count(&composer);
                    log_event_debug(
                        &mut key_debug,
                        "focus_gained",
                        focus,
                        "event",
                        "focus_gained",
                        lines,
                        lines,
                    );
                }
                Event::FocusLost => {
                    let lines = composer_line_count(&composer);
                    log_event_debug(
                        &mut key_debug,
                        "focus_lost",
                        focus,
                        "event",
                        "focus_lost",
                        lines,
                        lines,
                    );
                }
                Event::Mouse(_) => {
                    let lines = composer_line_count(&composer);
                    log_event_debug(
                        &mut key_debug,
                        "mouse",
                        focus,
                        "event",
                        "mouse",
                        lines,
                        lines,
                    );
                }
            }
        }
    }
}

/// Bash-style reverse-i-search over prompt history, active while Some.
struct HistorySearchState {
    query: String,
    match_index: Option<usize>,
    draft: String,
}

impl HistorySearchState {
    fn new(draft: String) -> Self {
        Self {
            query: String::new(),
            match_index: None,
            draft,
        }
    }

    fn title(&self, found: bool) -> String {
        if found || self.query.is_empty() {
            format!("Search history: {}", self.query)
        } else {
            format!("Search history (no match): {}", self.query)
        }
    }
}

/// Handles one key while reverse search is active. Returns false when the
/// search mode should exit.
fn handle_search_key(
    search: &mut HistorySearchState,
    history: &mut History,
    composer: &mut ComposerInput,
    key: KeyEvent,
) -> bool {
    if !matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) {
        return true;
    }

    let key = normalize_key_for_binding(key);
    let is_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            composer.set_text_end(&search.draft);
            return false;
        }
        KeyCode::Char('c') if is_ctrl => {
            composer.set_text_end(&search.draft);
            return false;
        }
        KeyCode::Enter => return false,
        KeyCode::Char('r') if is_ctrl => {
            if let Some((index, text)) = history.search_older(&search.query, search.match_index) {
                search.match_index = Some(index);
                composer.set_text_end(&text);
                composer.set_title(search.title(true));
            } else {
                composer.set_title(search.title(search.match_index.is_some()));
            }
        }
        KeyCode::Backspace => {
            search.query.pop();
            apply_search_from_newest(search, history, composer);
        }
        KeyCode::Char(ch) if !is_ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
            search.query.push(ch);
            apply_search_from_newest(search, history, composer);
        }
        _ => {}
    }
    true
}

fn apply_search_from_newest(
    search: &mut HistorySearchState,
    history: &mut History,
    composer: &mut ComposerInput,
) {
    match history.search_older(&search.query, None) {
        Some((index, text)) => {
            search.match_index = Some(index);
            composer.set_text_end(&text);
            composer.set_title(search.title(true));
        }
        None => {
            search.match_index = None;
            composer.set_title(search.title(false));
        }
    }
}

/// Routes Up/Down to cross-session history when the composer is empty or a
/// recall is already in progress. Any other key detaches the recalled text so
/// it can be edited as a normal draft.
fn handle_history_key(
    history: &mut History,
    composer: &ComposerInput,
    key: KeyEvent,
) -> Option<String> {
    if !matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) {
        return None;
    }

    match key.code {
        KeyCode::Up
            if key.modifiers.is_empty() && (composer.is_empty() || history.is_browsing()) =>
        {
            history.navigate_up(&composer.text())
        }
        KeyCode::Down if key.modifiers.is_empty() && history.is_browsing() => {
            history.navigate_down()
        }
        _ => {
            history.stop_browsing();
            None
        }
    }
}

enum FileKeyOutcome {
    Handled,
    Forward,
}

enum SlashKeyOutcome {
    Handled,
    Forward,
}

fn handle_file_popup_key(
    file_popup: &mut Option<FilePopup>,
    composer: &mut ComposerInput,
    key: KeyEvent,
) -> FileKeyOutcome {
    let token = composer.current_at_token(true);
    let token_query = token.as_ref().map(|token| token.query.as_str());
    let Some(popup) = file_popup.as_mut() else {
        return FileKeyOutcome::Forward;
    };

    match popup.handle_key(key, token_query) {
        FilePopupAction::None => FileKeyOutcome::Handled,
        FilePopupAction::Cancel | FilePopupAction::Close => {
            *file_popup = None;
            FileKeyOutcome::Handled
        }
        FilePopupAction::Accept(path) => {
            if let Some(token) = token {
                composer.replace_char_range(
                    token.start,
                    token.end,
                    &file_search::quote_path_for_insert(&path),
                );
            }
            *file_popup = None;
            FileKeyOutcome::Handled
        }
        FilePopupAction::Forward => FileKeyOutcome::Forward,
    }
}

fn handle_slash_popup_key(
    slash_popup: &mut Option<SlashPopup>,
    composer: &mut ComposerInput,
    key: KeyEvent,
) -> SlashKeyOutcome {
    let token = composer.current_slash_token();
    let token_query = token.as_ref().map(|token| token.query.as_str());
    let Some(popup) = slash_popup.as_mut() else {
        return SlashKeyOutcome::Forward;
    };

    match popup.handle_key(key, token_query) {
        SlashPopupAction::None => SlashKeyOutcome::Handled,
        SlashPopupAction::Cancel | SlashPopupAction::Close => {
            *slash_popup = None;
            SlashKeyOutcome::Handled
        }
        SlashPopupAction::Accept(command) => {
            if let Some(token) = token {
                let replacement = if token.has_space_after {
                    format!("/{command}")
                } else {
                    format!("/{command} ")
                };
                composer.replace_char_range(token.start, token.end, &replacement);
            }
            *slash_popup = None;
            SlashKeyOutcome::Handled
        }
        SlashPopupAction::Forward => SlashKeyOutcome::Forward,
    }
}

fn sync_file_popup(
    file_popup: &mut Option<FilePopup>,
    composer: &ComposerInput,
    cached_files: &mut Option<Vec<String>>,
    cwd: &Path,
) {
    let Some(token) = composer.current_at_token(true) else {
        if let Some(popup) = file_popup.as_mut() {
            popup.clear_dismissed_token();
        }
        *file_popup = None;
        return;
    };

    let cached_files = cached_files.get_or_insert_with(|| file_search::load_file_list(cwd));
    let matches = file_search::search_files(&token.query, cached_files);
    match file_popup {
        Some(popup) if popup.dismissed_token() == Some(token.query.as_str()) => {}
        Some(popup) => popup.set_query(&token.query, matches),
        None => {
            let mut popup = FilePopup::default();
            popup.set_query(&token.query, matches);
            *file_popup = Some(popup);
        }
    }
}

fn sync_slash_popup(slash_popup: &mut Option<SlashPopup>, composer: &ComposerInput) {
    let Some(token) = composer.current_slash_token() else {
        if let Some(popup) = slash_popup.as_mut() {
            popup.clear_dismissed_token();
        }
        *slash_popup = None;
        return;
    };

    match slash_popup {
        Some(popup) if popup.dismissed_token() == Some(token.query.as_str()) => {}
        Some(popup) => popup.set_query(&token.query),
        None => {
            let mut popup = SlashPopup::default();
            popup.set_query(&token.query);
            *slash_popup = Some(popup);
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

fn is_ctrl_r_press(key: KeyEvent) -> bool {
    let key = normalize_key_for_binding(key);
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && key.code == KeyCode::Char('r')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl_d_press(key: KeyEvent) -> bool {
    let key = normalize_key_for_binding(key);
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && key.code == KeyCode::Char('d')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl_g_press(key: KeyEvent) -> bool {
    let key = normalize_key_for_binding(key);
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && key.code == KeyCode::Char('g')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_tab_press(key: KeyEvent) -> bool {
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
}

fn is_plain_enter_press(key: KeyEvent) -> bool {
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && key.code == KeyCode::Enter
        && key.modifiers.is_empty()
}

#[derive(Debug)]
struct KeyDebug {
    file: File,
}

impl KeyDebug {
    fn create(path: PathBuf) -> io::Result<Self> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .map(|file| Self { file })
    }

    fn log_startup(&mut self) {
        let _ = writeln!(
            self.file,
            "{}",
            serde_json::json!({
                "kind": "startup",
                "keyboard_enhancement_requested": true,
                "keyboard_enhancement_flags": format!("{:?}", keyboard_enhancement_flags()),
                "modify_other_keys_requested": tmux_should_enable_modify_other_keys(),
                "env": debug_env(),
            })
        );
    }

    fn log(
        &mut self,
        focus: FocusTarget,
        key: KeyEvent,
        route: &str,
        action: &str,
        composer_lines_before: usize,
        composer_lines_after: usize,
    ) {
        let normalized = normalize_key_for_binding(key);
        let _ = writeln!(
            self.file,
            "{}",
            serde_json::json!({
                "kind": "key",
                "focus": format!("{focus:?}"),
                "raw": format!("{key:?}"),
                "normalized": format!("{normalized:?}"),
                "route": route,
                "action": action,
                "composer_lines_before": composer_lines_before,
                "composer_lines_after": composer_lines_after,
                "env": debug_env(),
            })
        );
    }

    fn log_event(
        &mut self,
        event: &str,
        focus: FocusTarget,
        route: &str,
        action: &str,
        composer_lines_before: usize,
        composer_lines_after: usize,
    ) {
        let _ = writeln!(
            self.file,
            "{}",
            serde_json::json!({
                "kind": "event",
                "event": event,
                "focus": format!("{focus:?}"),
                "route": route,
                "action": action,
                "composer_lines_before": composer_lines_before,
                "composer_lines_after": composer_lines_after,
                "env": debug_env(),
            })
        );
    }
}

fn log_key_debug(
    key_debug: &mut Option<KeyDebug>,
    focus: FocusTarget,
    key: KeyEvent,
    route: &str,
    action: &str,
    composer_lines_before: usize,
    composer_lines_after: usize,
) {
    if let Some(key_debug) = key_debug {
        key_debug.log(
            focus,
            key,
            route,
            action,
            composer_lines_before,
            composer_lines_after,
        );
    }
}

fn log_event_debug(
    key_debug: &mut Option<KeyDebug>,
    event: &str,
    focus: FocusTarget,
    route: &str,
    action: &str,
    composer_lines_before: usize,
    composer_lines_after: usize,
) {
    if let Some(key_debug) = key_debug {
        key_debug.log_event(
            event,
            focus,
            route,
            action,
            composer_lines_before,
            composer_lines_after,
        );
    }
}

fn debug_env() -> serde_json::Value {
    serde_json::json!({
        "TERM": std::env::var("TERM").ok(),
        "TERM_PROGRAM": std::env::var("TERM_PROGRAM").ok(),
        "TERM_PROGRAM_VERSION": std::env::var("TERM_PROGRAM_VERSION").ok(),
        "TERMINFO": std::env::var("TERMINFO").ok(),
        "CMUX_BUNDLE_ID": std::env::var("CMUX_BUNDLE_ID").ok(),
        "CMUX_PANEL_ID": std::env::var("CMUX_PANEL_ID").ok(),
        "TMUX": std::env::var("TMUX").ok(),
        "TMUX_PANE": std::env::var("TMUX_PANE").ok(),
    })
}

fn composer_line_count(composer: &ComposerInput) -> usize {
    composer.text().split('\n').count().max(1)
}

fn composer_action_label(lines_before: usize, lines_after: usize) -> &'static str {
    if lines_after > lines_before {
        "insert_newline"
    } else {
        "input"
    }
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

pub(crate) fn submitted_conversation_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub fn prefixed_thread_name(raw: &str, cwd: &str) -> Option<String> {
    let name = submitted_conversation_name(raw)?;
    let prefix = format!("{cwd}:");
    if name.starts_with(&prefix) {
        Some(name)
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
    TargetSelect,
    Options(usize),
}

impl FocusTarget {
    fn next(self, option_count: usize, has_target_select: bool) -> Self {
        match self {
            Self::Name => Self::Prompt,
            Self::Prompt if has_target_select => Self::TargetSelect,
            Self::Prompt if option_count == 0 => Self::Name,
            Self::Prompt => Self::Options(0),
            Self::TargetSelect if option_count == 0 => Self::Name,
            Self::TargetSelect => Self::Options(0),
            Self::Options(index) if index + 1 < option_count => Self::Options(index + 1),
            Self::Options(_) => Self::Name,
        }
    }
}

fn next_target_index(current: usize, count: usize, step: isize) -> usize {
    if count == 0 {
        return 0;
    }
    let count = count as isize;
    (current as isize + step).rem_euclid(count) as usize
}

type AppTerminal = Terminal<CrosstermBackend<io::Stdout>>;

/// Runs `operation` with the terminal restored to normal mode, then puts the
/// TUI back regardless of the operation's outcome.
fn with_suspended_terminal<T>(
    terminal: &mut AppTerminal,
    operation: impl FnOnce() -> T,
) -> anyhow::Result<T> {
    restore_terminal()?;
    let result = operation();
    *terminal = setup_terminal()?;
    terminal.clear()?;
    Ok(result)
}

/// An edited targets document that failed to commit; kept so reopening the
/// editor shows the user's text instead of discarding it.
struct PendingTargetEdit {
    snapshot: crate::targets::TargetEditSnapshot,
    draft: String,
}

fn edit_targets_document(
    terminal: &mut AppTerminal,
    popup: &mut TargetPopup,
    targets: &mut Vec<Target>,
    target_index: &mut usize,
    pending_target_edit: &mut Option<PendingTargetEdit>,
) -> anyhow::Result<()> {
    let mut edit = match pending_target_edit.take() {
        Some(edit) => edit,
        None => match crate::targets::begin_edit() {
            Ok(snapshot) => PendingTargetEdit {
                draft: snapshot.seed().to_string(),
                snapshot,
            },
            Err(err) => {
                popup.set_notice(format!("targets: {err:#}"));
                return Ok(());
            }
        },
    };
    let previous = targets.get(*target_index).cloned();
    let edited =
        with_suspended_terminal(terminal, || crate::external_editor::edit_toml(&edit.draft))?;
    match edited {
        Err(err) => {
            popup.set_notice(format!("editor: {err:#}"));
            *pending_target_edit = Some(edit);
        }
        Ok(text) => {
            edit.draft = text;
            match crate::targets::commit_edit(&edit.snapshot, &edit.draft) {
                Ok(updated) => {
                    *target_index =
                        reselect_target_index(previous.as_ref(), *target_index, &updated);
                    *targets = updated;
                    popup.set_selected(*target_index, targets.len());
                    popup.set_notice("targets saved");
                }
                Err(err) => {
                    popup.set_notice(format!("targets: {err:#}"));
                    *pending_target_edit = Some(edit);
                }
            }
        }
    }
    Ok(())
}

/// Picks the selection after the target list changed: same name first, then a
/// name-only rename (same settings), then the clamped previous position.
fn reselect_target_index(
    previous: Option<&Target>,
    previous_index: usize,
    updated: &[Target],
) -> usize {
    if updated.is_empty() {
        return 0;
    }
    let Some(previous) = previous else {
        return previous_index.min(updated.len() - 1);
    };
    if let Some(index) = updated
        .iter()
        .position(|candidate| candidate.name == previous.name)
    {
        return index;
    }
    if let Some((index, _)) = updated
        .iter()
        .enumerate()
        .filter(|(_, candidate)| same_target_except_name(candidate, previous))
        .min_by_key(|(index, _)| index.abs_diff(previous_index))
    {
        return index;
    }
    previous_index.min(updated.len() - 1)
}

fn same_target_except_name(left: &Target, right: &Target) -> bool {
    left.kind == right.kind
        && left.bin == right.bin
        && left.env == right.env
        && left.profile == right.profile
        && left.model == right.model
        && left.config == right.config
        && left.args == right.args
}

fn initial_focus() -> FocusTarget {
    FocusTarget::Name
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

    fn render_ref(&self, area: Rect, focused: bool, theme: &Theme, buf: &mut Buffer) {
        let title = if focused { "Name *" } else { "Name" };
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

        clear_area(inner, theme.panel_style(), buf);
        let content_width = inner.width as usize;
        if self.text.is_empty() {
            buf.set_stringn(
                inner.x,
                inner.y,
                "Optional conversation name",
                content_width,
                theme.muted_style(),
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
        buf.set_stringn(inner.x, inner.y, visible, content_width, theme.text_style());
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

fn clear_area(area: Rect, style: Style, buf: &mut Buffer) {
    let blank = " ".repeat(area.width as usize);
    for y in area.y..area.bottom() {
        buf.set_string(area.x, y, &blank, style);
    }
}

struct DrawState<'a> {
    name_input: &'a NameInput,
    composer: &'a ComposerInput,
    focus: FocusTarget,
    skills: &'a [Skill],
    skill_popup: Option<&'a SkillPopup>,
    file_popup: Option<&'a FilePopup>,
    slash_popup: Option<&'a SlashPopup>,
    header: &'a HeaderInfo,
    launch_options: &'a [ToggleOption],
    targets: &'a [Target],
    target_index: usize,
    target_popup: Option<&'a TargetPopup>,
    theme: &'a Theme,
    skill_mentions: &'a [String],
}

fn draw(frame: &mut Frame<'_>, state: DrawState<'_>) {
    let DrawState {
        name_input,
        composer,
        focus,
        skills,
        skill_popup,
        file_popup,
        slash_popup,
        header,
        launch_options,
        targets,
        target_index,
        target_popup,
        theme,
        skill_mentions,
    } = state;
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.root_style()), area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header.height()), Constraint::Min(1)])
        .split(area);

    frame.render_widget(
        Paragraph::new(header.lines(theme)).block(
            Block::default()
                .borders(Borders::ALL)
                .style(theme.panel_style())
                .border_style(theme.border_style(false)),
        ),
        layout[0],
    );

    let has_target_select = !targets.is_empty();
    let options_height = if launch_options.is_empty() && !has_target_select {
        0
    } else {
        1
    };
    let fixed_input_height = 3 + options_height;
    let composer_height = composer.desired_height(layout[1].width).clamp(
        3,
        layout[1].height.saturating_sub(fixed_input_height).max(3),
    );
    let input_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(composer_height),
            Constraint::Length(options_height),
        ])
        .split(layout[1]);

    name_input.render_ref(
        input_rows[1],
        focus == FocusTarget::Name,
        theme,
        frame.buffer_mut(),
    );
    composer.render_ref(
        input_rows[2],
        focus == FocusTarget::Prompt,
        theme,
        skill_mentions,
        frame.buffer_mut(),
    );
    if let Some(popup) = skill_popup {
        let popup_area = popup_area(input_rows[2], popup, skills);
        popup.render(popup_area, frame.buffer_mut(), skills, theme);
    }
    if let Some(popup) = file_popup {
        let popup_area = file_popup_area(input_rows[2], popup);
        popup.render(popup_area, frame.buffer_mut(), theme);
    }
    if let Some(popup) = slash_popup {
        let popup_area = slash_popup_area(input_rows[2], popup);
        popup.render(popup_area, frame.buffer_mut(), theme);
    }
    if options_height > 0 {
        let target_select = if has_target_select {
            targets
                .get(target_index)
                .map(|target| (target.name.as_str(), focus == FocusTarget::TargetSelect))
        } else {
            None
        };
        render_launch_options(
            input_rows[3],
            target_select,
            launch_options,
            focused_option_index(focus),
            theme,
            frame.buffer_mut(),
        );
    }
    if let Some(popup) = target_popup {
        let popup_area = target_popup_area(area, popup, targets);
        popup.render(popup_area, frame.buffer_mut(), targets, target_index, theme);
    }
    let cursor = if target_popup.is_some() {
        None
    } else {
        match focus {
            FocusTarget::Name => name_input.cursor_pos(input_rows[1]),
            FocusTarget::Prompt => composer.cursor_pos(input_rows[2]),
            FocusTarget::TargetSelect | FocusTarget::Options(_) => None,
        }
    };
    if let Some((x, y)) = cursor {
        frame.set_cursor_position((x, y));
    }
}

fn focused_option_index(focus: FocusTarget) -> Option<usize> {
    match focus {
        FocusTarget::Options(index) => Some(index),
        _ => None,
    }
}

fn render_launch_options(
    area: Rect,
    target_select: Option<(&str, bool)>,
    options: &[ToggleOption],
    focused_index: Option<usize>,
    theme: &Theme,
    buf: &mut Buffer,
) {
    clear_area(area, theme.panel_style(), buf);
    if area.width == 0 || area.height == 0 || (options.is_empty() && target_select.is_none()) {
        return;
    }

    let mut spans = Vec::new();
    if let Some((name, focused)) = target_select {
        spans.push(Span::styled("Target ", theme.muted_style()));
        let label = format!("‹{name}›");
        if focused {
            spans.push(Span::styled(label, theme.selected_style()));
            spans.push(Span::styled(" Enter manage", theme.muted_style()));
        } else {
            spans.push(Span::styled(label, theme.text_style()));
        }
        if !options.is_empty() {
            spans.push("  ".into());
        }
    }
    if !options.is_empty() {
        spans.push(Span::styled("Options ", theme.muted_style()));
    }
    for (index, option) in options.iter().enumerate() {
        if index > 0 {
            spans.push("  ".into());
        }
        let checkbox = if option.enabled { "[x] " } else { "[ ] " };
        let label = format!("{checkbox}{}", option.label);
        if focused_index == Some(index) {
            spans.push(Span::styled(label, theme.selected_style()));
        } else if option.enabled {
            spans.push(Span::styled(label, theme.text_style()));
        } else {
            spans.push(Span::styled(label, theme.muted_style()));
        }
    }
    buf.set_line(area.x, area.y, &Line::from(spans), area.width);
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

fn target_popup_area(frame_area: Rect, popup: &TargetPopup, targets: &[Target]) -> Rect {
    // Never exceed the frame: tiny terminals get a tiny (or empty) popup
    // instead of an out-of-bounds render.
    let width = frame_area
        .width
        .saturating_sub(4)
        .clamp(24, 72)
        .min(frame_area.width);
    let height = popup.required_height(targets).max(4).min(frame_area.height);
    let x = frame_area.x + frame_area.width.saturating_sub(width) / 2;
    let y = frame_area.y + frame_area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

fn file_popup_area(composer_area: Rect, popup: &FilePopup) -> Rect {
    let width = composer_area.width.saturating_sub(2).clamp(20, 72);
    let height = popup
        .required_height()
        .min(composer_area.y.saturating_sub(1))
        .max(3);
    let x = composer_area.x.saturating_add(1);
    let y = composer_area.y.saturating_sub(height);
    Rect::new(x, y, width, height)
}

fn slash_popup_area(composer_area: Rect, popup: &SlashPopup) -> Rect {
    let width = composer_area.width.saturating_sub(2).clamp(28, 88);
    let height = popup
        .required_height()
        .min(composer_area.y.saturating_sub(1))
        .max(3);
    let x = composer_area.x.saturating_add(1);
    let y = composer_area.y.saturating_sub(height);
    Rect::new(x, y, width, height)
}

struct HeaderInfo {
    cwd: String,
    cwd_path: PathBuf,
    git: String,
    template: Option<TemplateInfo>,
}

impl HeaderInfo {
    fn new(cwd: &Path, template: Option<TemplateInfo>) -> Self {
        Self {
            cwd: display_cwd(cwd),
            cwd_path: cwd.to_path_buf(),
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

    fn lines(&self, theme: &Theme) -> Vec<Line<'_>> {
        let mut lines = vec![Line::from(Span::styled(
            self.cwd.clone(),
            theme.text_style().add_modifier(Modifier::BOLD),
        ))];
        if let Some(template) = &self.template {
            lines.push(template_line(template, theme));
        }
        lines.push(Line::from(vec![
            Span::styled("git  ", theme.muted_style()),
            Span::styled(self.git.clone(), theme.secondary_style()),
        ]));
        lines
    }
}

fn template_line<'a>(template: &'a TemplateInfo, theme: &Theme) -> Line<'a> {
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
        Span::styled(
            format!("{label}: "),
            theme.title_style(true).add_modifier(Modifier::BOLD),
        ),
        Span::styled(description.to_string(), theme.text_style()),
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
    let Ok(output) = ProcessCommand::new("git")
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
    execute!(io::stdout(), EnableBracketedPaste)?;
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _ = execute!(
        io::stdout(),
        DisableModifyOtherKeys,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );
    if tmux_should_enable_modify_other_keys() {
        let _ = execute!(io::stdout(), EnableModifyOtherKeys);
    }
    let _ = execute!(io::stdout(), EnableFocusChange);
    let backend = CrosstermBackend::new(io::stdout());
    Terminal::new(backend).map_err(Into::into)
}

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

fn tmux_should_enable_modify_other_keys() -> bool {
    tmux_session_detected(
        std::env::var("TMUX").ok().as_deref(),
        std::env::var("TMUX_PANE").ok().as_deref(),
    ) && read_tmux_extended_keys_format().as_deref() == Some("csi-u")
}

fn tmux_session_detected(tmux: Option<&str>, tmux_pane: Option<&str>) -> bool {
    tmux.is_some() || tmux_pane.is_some()
}

fn read_tmux_extended_keys_format() -> Option<String> {
    for args in [
        ["display-message", "-p", "#{extended-keys-format}"],
        ["show-options", "-gqv", "extended-keys-format"],
    ] {
        let output = ProcessCommand::new("tmux")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;

        if !output.status.success() {
            continue;
        }

        if let Some(value) = String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return Some(value);
        }
    }

    None
}

pub fn restore_terminal() -> anyhow::Result<()> {
    let mut first_error = None;
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        ResetKeyboardEnhancementFlags,
        DisableModifyOtherKeys
    );
    if let Err(err) = execute!(io::stdout(), DisableBracketedPaste) {
        first_error.get_or_insert_with(|| anyhow::Error::from(err));
    }
    let _ = execute!(io::stdout(), DisableFocusChange);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResetKeyboardEnhancementFlags;

impl CrosstermCommand for ResetKeyboardEnhancementFlags {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[<u")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "keyboard enhancement reset is not implemented for the legacy Windows API",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableModifyOtherKeys;

impl CrosstermCommand for EnableModifyOtherKeys {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[>4;2m")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "modifyOtherKeys enable is not implemented for the legacy Windows API",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableModifyOtherKeys;

impl CrosstermCommand for DisableModifyOtherKeys {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[>4;0m")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "modifyOtherKeys reset is not implemented for the legacy Windows API",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ansi_for(command: impl CrosstermCommand) -> String {
        let mut out = String::new();
        command.write_ansi(&mut out).unwrap();
        out
    }

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
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            ComposerAction::None
        ));
        assert!(matches!(
            composer.input(KeyEvent::from(KeyCode::Char('b'))),
            ComposerAction::None
        ));
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
    fn slash_popup_enter_accepts_without_submitting() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/re");
        let mut slash_popup = Some(SlashPopup::default());
        sync_slash_popup(&mut slash_popup, &composer);

        assert!(matches!(
            handle_slash_popup_key(
                &mut slash_popup,
                &mut composer,
                KeyEvent::from(KeyCode::Enter)
            ),
            SlashKeyOutcome::Handled
        ));
        assert_eq!(composer.text(), "/review ");
        assert!(slash_popup.is_none());
    }

    #[test]
    fn slash_popup_shift_enter_reaches_composer() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/re");
        let mut slash_popup = Some(SlashPopup::default());
        sync_slash_popup(&mut slash_popup, &composer);

        assert!(matches!(
            handle_slash_popup_key(
                &mut slash_popup,
                &mut composer,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
            ),
            SlashKeyOutcome::Forward
        ));
        assert!(matches!(
            composer.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            ComposerAction::None
        ));
        assert_eq!(composer.text(), "/re\n");
    }

    #[test]
    fn slash_popup_accept_does_not_double_space_existing_args() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/re args");
        for _ in 0.." args".chars().count() {
            composer.input(KeyEvent::from(KeyCode::Left));
        }
        let mut slash_popup = Some(SlashPopup::default());
        sync_slash_popup(&mut slash_popup, &composer);

        assert!(matches!(
            handle_slash_popup_key(
                &mut slash_popup,
                &mut composer,
                KeyEvent::from(KeyCode::Tab)
            ),
            SlashKeyOutcome::Handled
        ));
        assert_eq!(composer.text(), "/review args");
    }

    #[test]
    fn slash_key_accepts_slash_popup_selection() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("/m");
        let mut slash_popup = Some(SlashPopup::default());
        sync_slash_popup(&mut slash_popup, &composer);

        assert!(matches!(
            handle_slash_popup_key(
                &mut slash_popup,
                &mut composer,
                KeyEvent::from(KeyCode::Char('/'))
            ),
            SlashKeyOutcome::Handled
        ));
        assert_eq!(composer.text(), "/model ");
        assert!(slash_popup.is_none());
    }

    #[test]
    fn history_up_recalls_only_into_empty_composer() {
        let dir =
            std::env::temp_dir().join(format!("prompt-builder-app-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("history.jsonl");
        std::fs::write(&path, "{\"ts\":1,\"text\":\"old prompt\"}\n").expect("write history");
        let mut history = History::new(None, vec![path]);

        let mut composer = ComposerInput::new();
        composer.set_initial_text("draft");
        assert_eq!(
            handle_history_key(&mut history, &composer, KeyEvent::from(KeyCode::Up)),
            None
        );

        let empty = ComposerInput::new();
        assert_eq!(
            handle_history_key(&mut history, &empty, KeyEvent::from(KeyCode::Up)),
            Some("old prompt".to_string())
        );
        // Typing detaches the recalled entry from history browsing.
        assert_eq!(
            handle_history_key(&mut history, &empty, KeyEvent::from(KeyCode::Char('x'))),
            None
        );
        assert!(!history.is_browsing());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reverse_search_recalls_matches_and_esc_restores_draft() {
        let dir =
            std::env::temp_dir().join(format!("prompt-builder-app-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("history.jsonl");
        std::fs::write(
            &path,
            "{\"ts\":1,\"text\":\"fix parser\"}\n{\"ts\":2,\"text\":\"add tests\"}\n{\"ts\":3,\"text\":\"fix bug\"}\n",
        )
        .expect("write history");
        let mut history = History::new(None, vec![path]);
        let mut composer = ComposerInput::new();
        composer.set_initial_text("draft");
        let mut search = HistorySearchState::new(composer.submission_text());

        // Typing "fix" recalls the newest match.
        for ch in ['f', 'i', 'x'] {
            assert!(handle_search_key(
                &mut search,
                &mut history,
                &mut composer,
                KeyEvent::from(KeyCode::Char(ch)),
            ));
        }
        assert_eq!(composer.text(), "fix bug");

        // Ctrl+R steps to the older match.
        assert!(handle_search_key(
            &mut search,
            &mut history,
            &mut composer,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        ));
        assert_eq!(composer.text(), "fix parser");

        // Esc restores the original draft and exits search.
        assert!(!handle_search_key(
            &mut search,
            &mut history,
            &mut composer,
            KeyEvent::from(KeyCode::Esc),
        ));
        assert_eq!(composer.text(), "draft");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reverse_search_enter_accepts_current_match() {
        let dir = std::env::temp_dir().join(format!(
            "prompt-builder-app-search-accept-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("history.jsonl");
        std::fs::write(&path, "{\"ts\":1,\"text\":\"ship it\"}\n").expect("write history");
        let mut history = History::new(None, vec![path]);
        let mut composer = ComposerInput::new();
        let mut search = HistorySearchState::new(String::new());

        for ch in ['s', 'h'] {
            handle_search_key(
                &mut search,
                &mut history,
                &mut composer,
                KeyEvent::from(KeyCode::Char(ch)),
            );
        }
        assert!(!handle_search_key(
            &mut search,
            &mut history,
            &mut composer,
            KeyEvent::from(KeyCode::Enter),
        ));
        assert_eq!(composer.text(), "ship it");
        let _ = std::fs::remove_dir_all(dir);
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
    fn plain_enter_press_ignores_release_and_modified_enter() {
        assert!(is_plain_enter_press(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
        assert!(!is_plain_enter_press(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(!is_plain_enter_press(KeyEvent::new_with_kind(
            KeyCode::Enter,
            KeyModifiers::NONE,
            event::KeyEventKind::Release
        )));
    }

    #[test]
    fn tab_press_is_focus_navigation() {
        assert!(is_tab_press(KeyEvent::from(KeyCode::Tab)));
        assert!(is_tab_press(KeyEvent::from(KeyCode::BackTab)));
        assert!(!is_tab_press(KeyEvent::from(KeyCode::Char('\t'))));
    }

    #[test]
    fn keyboard_enhancement_flags_match_codex_enter_contract() {
        let flags = keyboard_enhancement_flags();

        assert!(flags.contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS));
    }

    #[test]
    fn keyboard_mode_ansi_matches_codex_contract() {
        assert_eq!(ansi_for(ResetKeyboardEnhancementFlags), "\x1b[<u");
        assert_eq!(ansi_for(EnableModifyOtherKeys), "\x1b[>4;2m");
        assert_eq!(ansi_for(DisableModifyOtherKeys), "\x1b[>4;0m");
    }

    #[test]
    fn tmux_session_detection_accepts_tmux_or_tmux_pane() {
        assert!(tmux_session_detected(Some("/tmp/tmux/default,1,0"), None));
        assert!(tmux_session_detected(None, Some("%1")));
        assert!(!tmux_session_detected(None, None));
    }

    #[test]
    fn focus_cycles_through_options_when_present() {
        assert_eq!(FocusTarget::Name.next(2, false), FocusTarget::Prompt);
        assert_eq!(FocusTarget::Prompt.next(2, false), FocusTarget::Options(0));
        assert_eq!(
            FocusTarget::Options(0).next(2, false),
            FocusTarget::Options(1)
        );
        assert_eq!(FocusTarget::Options(1).next(2, false), FocusTarget::Name);
    }

    #[test]
    fn focus_cycles_through_target_selector_when_present() {
        assert_eq!(FocusTarget::Prompt.next(0, true), FocusTarget::TargetSelect);
        assert_eq!(FocusTarget::TargetSelect.next(0, true), FocusTarget::Name);
        assert_eq!(
            FocusTarget::TargetSelect.next(2, true),
            FocusTarget::Options(0)
        );
        assert_eq!(FocusTarget::Options(1).next(2, true), FocusTarget::Name);
    }

    #[test]
    fn options_row_renders_target_selector_before_options() {
        let area = Rect::new(0, 0, 60, 1);
        let mut buf = Buffer::empty(area);
        let options = vec![ToggleOption {
            label: "compact".to_string(),
            argv: vec!["--compact".to_string()],
            enabled: true,
        }];

        render_launch_options(
            area,
            Some(("claude", true)),
            &options,
            None,
            &Theme::catppuccin(),
            &mut buf,
        );

        let row: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(row.starts_with("Target ‹claude› Enter manage  Options [x] compact"));
    }

    #[test]
    fn options_row_renders_selector_without_options() {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);

        render_launch_options(
            area,
            Some(("egghead", false)),
            &[],
            None,
            &Theme::catppuccin(),
            &mut buf,
        );

        let row: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(row.starts_with("Target ‹egghead›"));
        assert!(!row.contains("Options"));
    }

    #[test]
    fn target_index_cycles_and_wraps_in_both_directions() {
        assert_eq!(next_target_index(0, 3, 1), 1);
        assert_eq!(next_target_index(2, 3, 1), 0);
        assert_eq!(next_target_index(0, 3, -1), 2);
        assert_eq!(next_target_index(0, 0, 1), 0);
    }

    fn named_target(name: &str) -> Target {
        Target {
            name: name.to_string(),
            ..Target::default()
        }
    }

    #[test]
    fn reselect_prefers_same_name_after_reorder() {
        let previous = named_target("b");
        let updated = vec![named_target("b"), named_target("a")];

        assert_eq!(reselect_target_index(Some(&previous), 1, &updated), 0);
    }

    #[test]
    fn reselect_follows_name_only_rename() {
        let mut previous = named_target("old");
        previous.model = Some("gpt-5.5".to_string());
        let mut renamed = named_target("new");
        renamed.model = Some("gpt-5.5".to_string());
        let updated = vec![named_target("other"), renamed];

        assert_eq!(reselect_target_index(Some(&previous), 0, &updated), 1);
    }

    #[test]
    fn reselect_uses_successor_after_middle_remove() {
        let mut previous = named_target("gone");
        previous.model = Some("unique".to_string());
        let updated = vec![named_target("a"), named_target("c")];

        assert_eq!(reselect_target_index(Some(&previous), 1, &updated), 1);
    }

    #[test]
    fn reselect_clamps_after_last_remove_and_empty_input() {
        let mut previous = named_target("gone");
        previous.model = Some("unique".to_string());
        let updated = vec![named_target("a"), named_target("b")];

        assert_eq!(reselect_target_index(Some(&previous), 5, &updated), 1);
        assert_eq!(reselect_target_index(Some(&previous), 5, &[]), 0);
        assert_eq!(reselect_target_index(None, 3, &updated), 1);
    }

    #[test]
    fn initial_focus_starts_in_name_field() {
        assert_eq!(initial_focus(), FocusTarget::Name);
    }

    #[test]
    fn focus_cycle_skips_options_when_none_are_present() {
        assert_eq!(FocusTarget::Name.next(0, false), FocusTarget::Prompt);
        assert_eq!(FocusTarget::Prompt.next(0, false), FocusTarget::Name);
    }

    #[test]
    fn enabled_options_are_flattened_for_submit() {
        let options = vec![
            ToggleOption {
                label: "fork from: last".to_string(),
                argv: vec!["--fork-from".to_string(), "last".to_string()],
                enabled: true,
            },
            ToggleOption {
                label: "compact".to_string(),
                argv: vec!["--compact".to_string()],
                enabled: false,
            },
        ];

        assert_eq!(
            enabled_option_argv(&options),
            vec!["--fork-from".to_string(), "last".to_string()]
        );
    }

    #[test]
    fn tab_accepts_file_popup_selection_instead_of_navigating_focus() {
        let mut composer = ComposerInput::new();
        composer.set_initial_text("inspect @ma");
        let mut popup = FilePopup::default();
        popup.set_query(
            "ma",
            vec![file_search::FileMatch {
                path: "src/main.rs".to_string(),
                score: 1,
            }],
        );
        let mut popup = Some(popup);

        let outcome =
            handle_file_popup_key(&mut popup, &mut composer, KeyEvent::from(KeyCode::Tab));

        assert!(matches!(outcome, FileKeyOutcome::Handled));
        assert!(popup.is_none());
        assert_eq!(composer.text(), "inspect src/main.rs ");
    }

    #[test]
    fn name_input_is_single_line() {
        let mut input = NameInput::new();

        input.handle_paste("Fix\nthis\tthing");

        assert_eq!(input.text(), "Fix this thing");
    }

    #[test]
    fn submitted_conversation_name_trims_outer_whitespace_and_skips_blank() {
        assert_eq!(submitted_conversation_name(" \t "), None);
        assert_eq!(
            submitted_conversation_name(" Fix parser "),
            Some("Fix parser".to_string())
        );
        assert_eq!(
            submitted_conversation_name("Fix parser: phase 2"),
            Some("Fix parser: phase 2".to_string())
        );
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
