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
use crate::composer_input::PlainEnterOutcome;
use crate::file_popup::FilePopup;
use crate::file_popup::FilePopupAction;
use crate::file_search;
use crate::flow::FlowEntry;
use crate::flow_form::FlowForm;
use crate::flow_popup::FlowPopup;
use crate::flow_popup::FlowPopupAction;
use crate::history::History;
use crate::line_input::clear_area;
use crate::line_input::LineInput;
use crate::skill_popup::SkillPopup;
use crate::skill_popup::SkillPopupAction;
use crate::skills::Skill;
use crate::slash_popup::SlashPopup;
use crate::slash_popup::SlashPopupAction;
use crate::target_popup::TargetPopup;
use crate::target_popup::TargetPopupAction;
use crate::targets::Target;
use crate::targets::TargetKind;
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
    /// mdflow flow selected for this submission; launches via mdflow when set.
    pub flow: Option<SubmittedFlow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmittedFlow {
    pub path: String,
    pub name: String,
    /// Collected template-var values passed as --_name=value.
    pub values: Vec<(String, String)>,
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
    mdflow_bin: String,
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
        mdflow_bin,
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
    mdflow_bin: String,
    loaded_theme: LoadedTheme,
    debug_keys: Option<PathBuf>,
) -> anyhow::Result<AppExit> {
    let theme = loaded_theme.theme;
    let mut target_index = initial_target.min(targets.len().saturating_sub(1));
    let mut target_popup: Option<TargetPopup> = None;
    let mut pending_target_edit: Option<PendingTargetEdit> = None;
    let mut flow_state = FlowState::new(mdflow_bin, &targets);
    let mut flow_popup: Option<FlowPopup> = None;
    let mut pending_flow_intent: Option<PendingFlowIntent> = None;
    let mut composer = ComposerInput::new();
    if let Some(diagnostic) = loaded_theme.diagnostic {
        composer.set_notice(diagnostic);
    }
    composer.set_hint_items(hint_items_for(initial_focus()));
    if !initial_prompt.is_empty() {
        composer.set_initial_text(&initial_prompt);
    }
    let mut name_input = LineInput::new("Name", "Optional conversation name");
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
        composer.set_hint_items(hint_items_for(focus));
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
                    flow_state: &flow_state,
                    flow_popup: flow_popup.as_ref(),
                    theme: &theme,
                    skill_mentions: &skill_mentions,
                },
            )
        })?;

        if let Some(intent) = pending_flow_intent.take() {
            composer.clear_notice();
            match flow_state.load_catalog(&header.cwd_path) {
                Ok(()) => match intent {
                    PendingFlowIntent::OpenPicker => {
                        flow_popup = Some(FlowPopup::new(flow_state.selected, flow_state.flows()));
                    }
                    PendingFlowIntent::Cycle(step) => flow_state.cycle(step),
                },
                Err(err) => composer.set_notice(format!("flows: {err}")),
            }
            focus = clamp_flow_focus(focus, &flow_state);
            continue;
        }

        if flow_state.pending_explain {
            if let Err(err) = flow_state.run_pending_explain(&header.cwd_path) {
                composer.set_notice(format!("flow inputs: {err}"));
            }
            focus = clamp_flow_focus(focus, &flow_state);
            continue;
        }

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
                    if flow_popup.is_some() {
                        let action = flow_popup
                            .as_mut()
                            .map(|popup| popup.handle_key(key, flow_state.flows()))
                            .unwrap_or(FlowPopupAction::None);
                        match action {
                            FlowPopupAction::None => {}
                            FlowPopupAction::Cancel => flow_popup = None,
                            FlowPopupAction::Accept(row) => {
                                flow_state.select(row);
                                if flow_state.pending_explain {
                                    composer.set_notice("loading flow inputs…");
                                }
                                flow_popup = None;
                                focus = clamp_flow_focus(focus, &flow_state);
                            }
                            FlowPopupAction::Reload => {
                                flow_state.form_cache.clear();
                                match flow_state.load_catalog(&header.cwd_path) {
                                    Ok(()) => {
                                        if let Some(popup) = flow_popup.as_mut() {
                                            popup.set_notice("flows reloaded");
                                        }
                                    }
                                    Err(err) => {
                                        if let Some(popup) = flow_popup.as_mut() {
                                            popup.set_notice(format!("reload: {err}"));
                                        }
                                    }
                                }
                            }
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "flow_popup",
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
                        if let FocusTarget::FlowInput(index) = focus {
                            if flow_state
                                .form
                                .as_mut()
                                .is_some_and(|form| form.clear_field(index))
                            {
                                log_key_debug(
                                    &mut key_debug,
                                    focus,
                                    key,
                                    "flow_input",
                                    "clear",
                                    lines_before,
                                    composer_line_count(&composer),
                                );
                                continue;
                            }
                        }
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
                    if skill_popup.is_some() {
                        match handle_skill_popup_key(&mut skill_popup, &mut composer, &skills, key)
                        {
                            SkillKeyOutcome::Handled => {
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
                            SkillKeyOutcome::Forward => {}
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
                    if is_esc_press(key) {
                        if focus == FocusTarget::Prompt {
                            let draft = composer.submission_text();
                            if !draft.trim().is_empty() {
                                // Like Ctrl+C, a cleared draft stays recoverable
                                // via Up-arrow history.
                                history.record(&draft);
                                composer.clear();
                            }
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "app",
                                "esc_clear",
                                lines_before,
                                composer_line_count(&composer),
                            );
                        } else {
                            focus = FocusTarget::Prompt;
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "app",
                                "esc_focus_prompt",
                                lines_before,
                                composer_line_count(&composer),
                            );
                        }
                        continue;
                    }
                    if is_tab_press(key) {
                        let ctx = FocusContext {
                            flow_field_count: flow_state.form_field_count(),
                            has_target_select: !targets.is_empty(),
                            has_flow_select: flow_state.available && !targets.is_empty(),
                            option_count: launch_options.len(),
                        };
                        let (next_focus, action) = if is_back_tab_press(key) {
                            (focus.prev(ctx), "focus_prev")
                        } else {
                            (focus.next(ctx), "focus_next")
                        };
                        focus = next_focus;
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "app",
                            action,
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
                    if let FocusTarget::FlowInput(index) = focus {
                        if is_plain_enter_press(key) {
                            let ctx = FocusContext {
                                flow_field_count: flow_state.form_field_count(),
                                has_target_select: !targets.is_empty(),
                                has_flow_select: flow_state.available && !targets.is_empty(),
                                option_count: launch_options.len(),
                            };
                            focus = focus.next(ctx);
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "flow_input",
                                "focus_next",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            continue;
                        }
                        if let Some(form) = flow_state.form.as_mut() {
                            form.handle_key(index, key);
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "flow_input",
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
                                    flow_popup = None;
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
                    if focus == FocusTarget::FlowSelect {
                        if matches!(
                            key.kind,
                            event::KeyEventKind::Press | event::KeyEventKind::Repeat
                        ) {
                            let loaded = matches!(flow_state.catalog, CatalogState::Loaded(_));
                            let intent = match key.code {
                                KeyCode::Char(' ') | KeyCode::Right | KeyCode::Down => {
                                    Some(PendingFlowIntent::Cycle(1))
                                }
                                KeyCode::Left | KeyCode::Up => Some(PendingFlowIntent::Cycle(-1)),
                                KeyCode::Enter => Some(PendingFlowIntent::OpenPicker),
                                _ => None,
                            };
                            match intent {
                                Some(PendingFlowIntent::Cycle(step)) if loaded => {
                                    flow_state.cycle(step);
                                    if flow_state.pending_explain {
                                        composer.set_notice("loading flow inputs…");
                                    }
                                }
                                Some(PendingFlowIntent::OpenPicker) if loaded => {
                                    skill_popup = None;
                                    file_popup = None;
                                    slash_popup = None;
                                    flow_popup = Some(FlowPopup::new(
                                        flow_state.selected,
                                        flow_state.flows(),
                                    ));
                                }
                                Some(intent) => {
                                    // Paint a loading frame, then fetch on the
                                    // next loop pass and apply the intent.
                                    skill_popup = None;
                                    file_popup = None;
                                    slash_popup = None;
                                    composer.set_notice("loading flows…");
                                    pending_flow_intent = Some(intent);
                                }
                                None => {}
                            }
                        }
                        log_key_debug(
                            &mut key_debug,
                            focus,
                            key,
                            "flow_select",
                            "handled",
                            lines_before,
                            composer_line_count(&composer),
                        );
                        continue;
                    }
                    if let FocusTarget::Options(index) = focus {
                        if matches!(
                            key.kind,
                            event::KeyEventKind::Press | event::KeyEventKind::Repeat
                        ) {
                            let count = launch_options.len();
                            match key.code {
                                KeyCode::Char(' ') => {
                                    if let Some(option) = launch_options.get_mut(index) {
                                        option.enabled = !option.enabled;
                                    }
                                }
                                KeyCode::Enter => {
                                    focus = focus.next(FocusContext {
                                        flow_field_count: flow_state.form_field_count(),
                                        has_target_select: !targets.is_empty(),
                                        has_flow_select: flow_state.available
                                            && !targets.is_empty(),
                                        option_count: count,
                                    });
                                }
                                KeyCode::Right | KeyCode::Down if count > 0 => {
                                    focus = FocusTarget::Options((index + 1) % count);
                                }
                                KeyCode::Left | KeyCode::Up if count > 0 => {
                                    focus = FocusTarget::Options((index + count - 1) % count);
                                }
                                _ => {}
                            }
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
                    if is_plain_enter_press(key) && focus == FocusTarget::Prompt {
                        if let Some(blocker) =
                            flow_submit_blocker(&flow_state, targets.get(target_index))
                        {
                            match composer.plain_enter_gate() {
                                PlainEnterOutcome::InsertedNewline
                                | PlainEnterOutcome::EmptyDraft => {}
                                PlainEnterOutcome::WouldSubmit => {
                                    composer.set_notice(blocker.message);
                                    focus = clamp_flow_focus(blocker.focus, &flow_state);
                                }
                            }
                            log_key_debug(
                                &mut key_debug,
                                focus,
                                key,
                                "flow_gate",
                                "blocked",
                                lines_before,
                                composer_line_count(&composer),
                            );
                            continue;
                        }
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
                                flow: flow_state.selected_flow().map(|entry| SubmittedFlow {
                                    path: entry.path.clone(),
                                    name: entry.name.clone(),
                                    values: flow_state
                                        .form
                                        .as_ref()
                                        .map(FlowForm::values)
                                        .unwrap_or_default(),
                                }),
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
                    sync_skill_popup(&mut skill_popup, &composer, &skills);
                    sync_file_popup(
                        &mut file_popup,
                        &composer,
                        &mut cached_files,
                        &header.cwd_path,
                    );
                    sync_slash_popup(&mut slash_popup, &composer);
                }
                Event::Paste(text) => {
                    if target_popup.is_some() || flow_popup.is_some() {
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
                            sync_skill_popup(&mut skill_popup, &composer, &skills);
                            sync_file_popup(
                                &mut file_popup,
                                &composer,
                                &mut cached_files,
                                &header.cwd_path,
                            );
                            sync_slash_popup(&mut slash_popup, &composer);
                        }
                        FocusTarget::FlowInput(index) => {
                            if let Some(form) = flow_state.form.as_mut() {
                                form.handle_paste(index, &text);
                            }
                        }
                        FocusTarget::TargetSelect
                        | FocusTarget::FlowSelect
                        | FocusTarget::Options(_) => {}
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
            if key.modifiers.is_empty()
                && (composer.is_empty()
                    || (history.is_browsing() && composer.is_on_first_visual_line())) =>
        {
            history.navigate_up(&composer.text())
        }
        KeyCode::Down
            if key.modifiers.is_empty()
                && history.is_browsing()
                && composer.is_on_last_visual_line() =>
        {
            history.navigate_down()
        }
        KeyCode::Up | KeyCode::Down if key.modifiers.is_empty() => None,
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

enum SkillKeyOutcome {
    Handled,
    Forward,
}

fn handle_skill_popup_key(
    skill_popup: &mut Option<SkillPopup>,
    composer: &mut ComposerInput,
    skills: &[Skill],
    key: KeyEvent,
) -> SkillKeyOutcome {
    let token = composer.current_skill_token(true);
    let token_query = token.as_ref().map(|token| token.query.as_str());
    let Some(popup) = skill_popup.as_mut() else {
        return SkillKeyOutcome::Forward;
    };

    match popup.handle_key(key, skills, token_query) {
        SkillPopupAction::None => SkillKeyOutcome::Handled,
        SkillPopupAction::Cancel | SkillPopupAction::Close => {
            *skill_popup = None;
            SkillKeyOutcome::Handled
        }
        SkillPopupAction::Accept(mention) => {
            if let Some(token) = token {
                composer.replace_char_range(token.start, token.end, &format!("{mention} "));
            }
            *skill_popup = None;
            SkillKeyOutcome::Handled
        }
        SkillPopupAction::Forward => SkillKeyOutcome::Forward,
    }
}

fn sync_skill_popup(
    skill_popup: &mut Option<SkillPopup>,
    composer: &ComposerInput,
    skills: &[Skill],
) {
    let Some(token) = composer.current_skill_token(true) else {
        if let Some(popup) = skill_popup.as_mut() {
            popup.clear_dismissed_token();
        }
        *skill_popup = None;
        return;
    };

    // `$` shows up in ordinary prose (prices, shell vars) far more often than
    // `@` or `/`, so only surface the popup while something actually matches.
    if crate::skill_popup::matching_indices(&token.query, skills).is_empty() {
        *skill_popup = None;
        return;
    }

    match skill_popup {
        Some(popup) if popup.dismissed_token() == Some(token.query.as_str()) => {}
        Some(popup) => popup.set_query(&token.query, skills),
        None => {
            let mut popup = SkillPopup::default();
            popup.set_query(&token.query, skills);
            *skill_popup = Some(popup);
        }
    }
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

fn is_back_tab_press(key: KeyEvent) -> bool {
    is_tab_press(key)
        && (key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT))
}

fn is_esc_press(key: KeyEvent) -> bool {
    matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) && key.code == KeyCode::Esc
        && key.modifiers.is_empty()
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

fn hint_items_for(focus: FocusTarget) -> Vec<(&'static str, &'static str)> {
    match focus {
        FocusTarget::Name => vec![
            ("Tab", "field"),
            ("Shift+Tab", "back"),
            ("Enter", "prompt"),
            ("Esc", "prompt"),
            ("Ctrl+C", "quit"),
        ],
        FocusTarget::Prompt => vec![
            ("Tab", "field"),
            ("Enter", "send"),
            ("Shift+Enter", "newline"),
            ("↑", "history"),
            ("Ctrl+R", "search"),
            ("Ctrl+G", "editor"),
            ("Ctrl+C", "quit"),
            // Last on purpose: at narrow widths the hint row clips from the
            // tail, and the seven hints above are the load-bearing ones.
            ("Esc", "clear"),
        ],
        FocusTarget::FlowInput(_) => vec![
            ("Tab", "field"),
            ("Enter", "next"),
            ("Space", "toggle"),
            ("Esc", "prompt"),
            ("Ctrl+C", "quit"),
        ],
        FocusTarget::TargetSelect => vec![
            ("Tab", "field"),
            ("←/→", "target"),
            ("Enter", "targets"),
            ("Ctrl+G", "edit"),
            ("Esc", "prompt"),
            ("Ctrl+C", "quit"),
        ],
        FocusTarget::FlowSelect => vec![
            ("Tab", "field"),
            ("←/→", "flow"),
            ("Enter", "flows"),
            ("Esc", "prompt"),
            ("Ctrl+C", "quit"),
        ],
        FocusTarget::Options(_) => vec![
            ("Tab", "field"),
            ("←/→", "option"),
            ("Space", "toggle"),
            ("Enter", "next"),
            ("Esc", "prompt"),
            ("Ctrl+C", "quit"),
        ],
    }
}

fn composer_action_label(lines_before: usize, lines_after: usize) -> &'static str {
    if lines_after > lines_before {
        "insert_newline"
    } else {
        "input"
    }
}

fn handle_ctrl_c(
    name_input: &mut LineInput,
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
    /// One field of the mdflow flow-input form.
    FlowInput(usize),
    TargetSelect,
    FlowSelect,
    Options(usize),
}

/// Which focusable widgets exist right now; drives Tab order:
/// Name → Prompt → FlowInput(0..n) → TargetSelect → FlowSelect → Options(0..k).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FocusContext {
    flow_field_count: usize,
    has_target_select: bool,
    has_flow_select: bool,
    option_count: usize,
}

impl FocusTarget {
    fn next(self, ctx: FocusContext) -> Self {
        match self {
            Self::Name => Self::Prompt,
            Self::Prompt if ctx.flow_field_count > 0 => Self::FlowInput(0),
            Self::Prompt => Self::after_flow_inputs(ctx),
            Self::FlowInput(index) if index + 1 < ctx.flow_field_count => {
                Self::FlowInput(index + 1)
            }
            Self::FlowInput(_) => Self::after_flow_inputs(ctx),
            Self::TargetSelect if ctx.has_flow_select => Self::FlowSelect,
            Self::TargetSelect => Self::after_flow_select(ctx),
            Self::FlowSelect => Self::after_flow_select(ctx),
            Self::Options(index) if index + 1 < ctx.option_count => Self::Options(index + 1),
            Self::Options(_) => Self::Name,
        }
    }

    fn prev(self, ctx: FocusContext) -> Self {
        match self {
            Self::Name if ctx.option_count > 0 => Self::Options(ctx.option_count - 1),
            Self::Name if ctx.has_flow_select => Self::FlowSelect,
            Self::Name if ctx.has_target_select => Self::TargetSelect,
            Self::Name if ctx.flow_field_count > 0 => Self::FlowInput(ctx.flow_field_count - 1),
            Self::Name => Self::Prompt,
            Self::Prompt => Self::Name,
            Self::FlowInput(0) => Self::Prompt,
            Self::FlowInput(index) => Self::FlowInput(index - 1),
            Self::TargetSelect if ctx.flow_field_count > 0 => {
                Self::FlowInput(ctx.flow_field_count - 1)
            }
            Self::TargetSelect => Self::Prompt,
            Self::FlowSelect if ctx.has_target_select => Self::TargetSelect,
            Self::FlowSelect if ctx.flow_field_count > 0 => {
                Self::FlowInput(ctx.flow_field_count - 1)
            }
            Self::FlowSelect => Self::Prompt,
            Self::Options(0) if ctx.has_flow_select => Self::FlowSelect,
            Self::Options(0) if ctx.has_target_select => Self::TargetSelect,
            Self::Options(0) if ctx.flow_field_count > 0 => {
                Self::FlowInput(ctx.flow_field_count - 1)
            }
            Self::Options(0) => Self::Prompt,
            Self::Options(index) => Self::Options(index - 1),
        }
    }

    fn after_flow_inputs(ctx: FocusContext) -> Self {
        if ctx.has_target_select {
            Self::TargetSelect
        } else if ctx.has_flow_select {
            Self::FlowSelect
        } else if ctx.option_count > 0 {
            Self::Options(0)
        } else {
            Self::Name
        }
    }

    fn after_flow_select(ctx: FocusContext) -> Self {
        if ctx.option_count > 0 {
            Self::Options(0)
        } else {
            Self::Name
        }
    }
}

/// mdflow catalog state; fetched lazily on first Flow-slot interaction.
enum CatalogState {
    NotLoaded,
    Loaded(Vec<FlowEntry>),
    Failed(String),
}

/// Intent queued while the catalog loads, so the "loading flows…" frame
/// paints before the blocking fetch runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingFlowIntent {
    OpenPicker,
    Cycle(isize),
}

struct FlowState {
    catalog: CatalogState,
    /// Index into the loaded catalog; None = "No flow" (launch target directly).
    selected: Option<usize>,
    available: bool,
    mdflow_bin: String,
    /// Input form for the selected flow; built from `mdflow explain --json`.
    form: Option<FlowForm>,
    /// Per-flow-path forms, preserving typed values across reselects.
    form_cache: std::collections::HashMap<String, FlowForm>,
    explain_error: Option<String>,
    /// Explain queued until after the next draw, so a loading notice paints
    /// first and rapid cycling coalesces into one fetch.
    pending_explain: bool,
}

impl FlowState {
    fn new(mdflow_bin: String, targets: &[Target]) -> Self {
        let available = targets
            .iter()
            .any(|target| target.kind == TargetKind::Mdflow)
            || bin_on_path(&mdflow_bin);
        Self {
            catalog: CatalogState::NotLoaded,
            selected: None,
            available,
            mdflow_bin,
            form: None,
            form_cache: std::collections::HashMap::new(),
            explain_error: None,
            pending_explain: false,
        }
    }

    fn flows(&self) -> &[FlowEntry] {
        match &self.catalog {
            CatalogState::Loaded(flows) => flows,
            _ => &[],
        }
    }

    fn selected_flow(&self) -> Option<&FlowEntry> {
        self.selected.and_then(|index| self.flows().get(index))
    }

    fn slot_label(&self) -> String {
        match (&self.catalog, self.selected_flow()) {
            (CatalogState::Failed(_), _) => "unavailable".to_string(),
            (_, Some(flow)) => flow.name.clone(),
            (_, None) => "none".to_string(),
        }
    }

    /// Fetches (or re-fetches) the catalog, preserving the selection by path.
    fn load_catalog(&mut self, cwd: &Path) -> Result<(), String> {
        let previous_path = self.selected_flow().map(|flow| flow.path.clone());
        match crate::flow::fetch_catalog(&self.mdflow_bin, cwd) {
            Ok(catalog) => {
                let selected = previous_path
                    .and_then(|path| catalog.flows.iter().position(|flow| flow.path == path));
                self.catalog = CatalogState::Loaded(catalog.flows);
                self.select(selected);
                Ok(())
            }
            Err(err) => {
                let message = format!("{err:#}");
                self.catalog = CatalogState::Failed(message.clone());
                self.select(None);
                Err(message)
            }
        }
    }

    fn cycle(&mut self, step: isize) {
        let count = self.flows().len();
        if count == 0 {
            self.select(None);
            return;
        }
        // Positions: 0 = "No flow", 1..=count = flows.
        let current = self.selected.map(|index| index + 1).unwrap_or(0) as isize;
        let next = (current + step).rem_euclid(count as isize + 1) as usize;
        self.select(next.checked_sub(1));
    }

    /// Changes the selection, stashing the current form (typed values and
    /// all) and restoring a cached one or queueing an explain fetch.
    fn select(&mut self, row: Option<usize>) {
        self.stash_form();
        self.selected = row;
        self.explain_error = None;
        self.pending_explain = false;
        self.form = None;
        if let Some(path) = self.selected_flow().map(|flow| flow.path.clone()) {
            match self.form_cache.remove(&path) {
                Some(form) => self.form = Some(form),
                None => self.pending_explain = true,
            }
        }
    }

    fn stash_form(&mut self) {
        if let (Some(form), Some(flow)) = (
            self.form.take(),
            self.selected.and_then(|index| match &self.catalog {
                CatalogState::Loaded(flows) => flows.get(index),
                _ => None,
            }),
        ) {
            self.form_cache.insert(flow.path.clone(), form);
        }
    }

    /// Runs the queued explain fetch. Returns an error message for a notice.
    fn run_pending_explain(&mut self, cwd: &Path) -> Result<(), String> {
        self.pending_explain = false;
        let Some(flow) = self.selected_flow() else {
            return Ok(());
        };
        let path = flow.path.clone();
        match crate::flow::explain_flow(&self.mdflow_bin, &path, cwd) {
            Ok(explain) => {
                let (fields, prompt_capable) = crate::flow::extract_fields(&explain);
                self.form = Some(FlowForm::new(fields, prompt_capable));
                self.explain_error = None;
                Ok(())
            }
            Err(err) => {
                let message = format!("{err:#}");
                self.explain_error = Some(message.clone());
                Err(message)
            }
        }
    }

    fn form_field_count(&self) -> usize {
        self.form.as_ref().map_or(0, FlowForm::field_count)
    }

    fn catalog_error(&self) -> Option<&str> {
        match &self.catalog {
            CatalogState::Failed(message) => Some(message.as_str()),
            _ => None,
        }
    }
}

/// Reason a submit cannot proceed; jump focus to the offending widget.
struct FlowSubmitBlocker {
    message: String,
    focus: FocusTarget,
}

fn flow_submit_blocker(
    flow_state: &FlowState,
    target: Option<&Target>,
) -> Option<FlowSubmitBlocker> {
    if let Some(form) = &flow_state.form {
        if let Some((index, message)) = form.first_invalid() {
            return Some(FlowSubmitBlocker {
                message,
                focus: FocusTarget::FlowInput(index),
            });
        }
    }
    if flow_state.selected_flow().is_some() {
        if let Some(error) = &flow_state.explain_error {
            return Some(FlowSubmitBlocker {
                message: format!("flow inputs unavailable: {error}"),
                focus: FocusTarget::FlowSelect,
            });
        }
        return None;
    }
    // No flow selected: an mdflow target needs one (unless it pins its own).
    if let Some(target) = target {
        if target.kind == TargetKind::Mdflow && target.flow.is_none() {
            let message = match flow_state.catalog_error() {
                Some(error) => format!("mdflow target needs a flow (flows: {error})"),
                None => "mdflow target needs a flow: Tab to the Flow slot".to_string(),
            };
            return Some(FlowSubmitBlocker {
                message,
                focus: FocusTarget::FlowSelect,
            });
        }
    }
    None
}

/// Stale FlowInput focus (form rebuilt/cleared) falls back to the prompt.
fn clamp_flow_focus(focus: FocusTarget, flow_state: &FlowState) -> FocusTarget {
    match focus {
        FocusTarget::FlowInput(index) if index >= flow_state.form_field_count() => {
            if flow_state.form_field_count() > 0 {
                FocusTarget::FlowInput(flow_state.form_field_count() - 1)
            } else {
                FocusTarget::Prompt
            }
        }
        other => other,
    }
}

fn bin_on_path(bin: &str) -> bool {
    if bin.contains('/') {
        return Path::new(bin).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
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

struct DrawState<'a> {
    name_input: &'a LineInput,
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
    flow_state: &'a FlowState,
    flow_popup: Option<&'a FlowPopup>,
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
        flow_state,
        flow_popup,
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
    let form_height = flow_state.form.as_ref().map_or(0, FlowForm::height);
    let fixed_input_height = 3 + options_height + form_height;
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
            Constraint::Length(form_height),
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
    if let Some(form) = &flow_state.form {
        let focused_field = match focus {
            FocusTarget::FlowInput(index) => Some(index),
            _ => None,
        };
        form.render(input_rows[3], focused_field, theme, frame.buffer_mut());
    }
    let flow_slot_label = flow_state.slot_label();
    if options_height > 0 {
        let target_select = if has_target_select {
            targets
                .get(target_index)
                .map(|target| (target.name.as_str(), focus == FocusTarget::TargetSelect))
        } else {
            None
        };
        let flow_ignores_prompt = flow_state
            .form
            .as_ref()
            .is_some_and(|form| !form.prompt_capable);
        let flow_select = (flow_state.available && has_target_select).then(|| FlowSlotView {
            label: flow_slot_label.as_str(),
            focused: focus == FocusTarget::FlowSelect,
            ignores_prompt: flow_ignores_prompt,
        });
        render_launch_options(
            input_rows[4],
            target_select,
            flow_select,
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
    if let Some(popup) = flow_popup {
        let popup_area = flow_popup_area(area, popup, flow_state.flows());
        popup.render(
            popup_area,
            frame.buffer_mut(),
            flow_state.flows(),
            flow_state.selected,
            theme,
        );
    }
    let cursor = if target_popup.is_some() || flow_popup.is_some() {
        None
    } else {
        match focus {
            FocusTarget::Name => name_input.cursor_pos(input_rows[1]),
            FocusTarget::Prompt => composer.cursor_pos(input_rows[2]),
            FocusTarget::FlowInput(index) => flow_state
                .form
                .as_ref()
                .and_then(|form| form.cursor_pos(input_rows[3], index)),
            FocusTarget::TargetSelect | FocusTarget::FlowSelect | FocusTarget::Options(_) => None,
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

struct FlowSlotView<'a> {
    label: &'a str,
    focused: bool,
    /// True when the selected flow never references the composed prompt.
    ignores_prompt: bool,
}

fn render_launch_options(
    area: Rect,
    target_select: Option<(&str, bool)>,
    flow_select: Option<FlowSlotView<'_>>,
    options: &[ToggleOption],
    focused_index: Option<usize>,
    theme: &Theme,
    buf: &mut Buffer,
) {
    clear_area(area, theme.panel_style(), buf);
    if area.width == 0
        || area.height == 0
        || (options.is_empty() && target_select.is_none() && flow_select.is_none())
    {
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
        if flow_select.is_some() || !options.is_empty() {
            spans.push("  ".into());
        }
    }
    if let Some(flow) = flow_select {
        spans.push(Span::styled("Flow ", theme.muted_style()));
        let label = format!("‹{}›", flow.label);
        if flow.focused {
            spans.push(Span::styled(label, theme.selected_style()));
            spans.push(Span::styled(" Enter flows", theme.muted_style()));
        } else if flow.label == "none" || flow.label == "unavailable" {
            spans.push(Span::styled(label, theme.muted_style()));
        } else {
            spans.push(Span::styled(label, theme.text_style()));
        }
        if flow.ignores_prompt {
            spans.push(Span::styled(" ⚠ ignores prompt", theme.warning_style()));
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

fn flow_popup_area(frame_area: Rect, popup: &FlowPopup, flows: &[FlowEntry]) -> Rect {
    let width = frame_area
        .width
        .saturating_sub(4)
        .clamp(24, 80)
        .min(frame_area.width);
    let height = popup.required_height(flows).max(5).min(frame_area.height);
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
    fn esc_press_requires_plain_escape() {
        assert!(is_esc_press(KeyEvent::from(KeyCode::Esc)));
        assert!(!is_esc_press(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::ALT
        )));
        assert!(!is_esc_press(KeyEvent::new_with_kind(
            KeyCode::Esc,
            KeyModifiers::NONE,
            event::KeyEventKind::Release
        )));
    }

    #[test]
    fn deleting_into_skill_mention_reopens_search() {
        let skills = vec![Skill {
            name: "fusion".to_string(),
            description: "Run Fusion".to_string(),
            path: PathBuf::from("/tmp/fusion/SKILL.md"),
        }];
        let mut composer = ComposerInput::new();
        composer.set_initial_text("$fusion ");
        let mut popup: Option<SkillPopup> = None;

        // Cursor sits after the trailing space: no active token.
        sync_skill_popup(&mut popup, &composer, &skills);
        assert!(popup.is_none());

        // Backspacing over the space and into the mention re-triggers search.
        composer.input(KeyEvent::from(KeyCode::Backspace));
        composer.input(KeyEvent::from(KeyCode::Backspace));
        sync_skill_popup(&mut popup, &composer, &skills);
        assert!(popup.is_some());

        // Accepting replaces the partial token with the full mention.
        assert!(matches!(
            handle_skill_popup_key(
                &mut popup,
                &mut composer,
                &skills,
                KeyEvent::from(KeyCode::Enter)
            ),
            SkillKeyOutcome::Handled
        ));
        assert_eq!(composer.text(), "$fusion ");
        assert!(popup.is_none());
    }

    #[test]
    fn skill_popup_does_not_open_inside_plain_dollar_text() {
        let skills = vec![Skill {
            name: "fusion".to_string(),
            description: "Run Fusion".to_string(),
            path: PathBuf::from("/tmp/fusion/SKILL.md"),
        }];
        let mut composer = ComposerInput::new();
        composer.set_initial_text("$99");
        let mut popup: Option<SkillPopup> = None;

        sync_skill_popup(&mut popup, &composer, &skills);
        assert!(popup.is_none());
    }

    #[test]
    fn typing_dollar_opens_skill_search() {
        let skills = vec![Skill {
            name: "fusion".to_string(),
            description: "Run Fusion".to_string(),
            path: PathBuf::from("/tmp/fusion/SKILL.md"),
        }];
        let mut composer = ComposerInput::new();
        composer.input(KeyEvent::from(KeyCode::Char('$')));
        let mut popup: Option<SkillPopup> = None;

        sync_skill_popup(&mut popup, &composer, &skills);
        assert!(popup.is_some());
    }

    #[test]
    fn ctrl_c_clears_nonempty_composer_before_canceling() {
        let mut name_input = LineInput::new("Name", "Optional conversation name");
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
        let mut name_input = LineInput::new("Name", "Optional conversation name");
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
    fn history_arrows_traverse_recalled_multiline_text_before_switching_entries() {
        let dir = std::env::temp_dir().join(format!(
            "prompt-builder-app-multiline-history-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("history.jsonl");
        std::fs::write(
            &path,
            "{\"ts\":1,\"text\":\"older\"}\n{\"ts\":2,\"text\":\"first\\nsecond\"}\n",
        )
        .expect("write history");
        let mut history = History::new(None, vec![path]);
        let mut composer = ComposerInput::new();
        composer.desired_height(82);

        let recalled = handle_history_key(&mut history, &composer, KeyEvent::from(KeyCode::Up))
            .expect("recall newest entry");
        composer.set_text_end(&recalled);

        assert_eq!(
            handle_history_key(&mut history, &composer, KeyEvent::from(KeyCode::Up)),
            None
        );
        composer.input(KeyEvent::from(KeyCode::Up));
        assert!(composer.is_on_first_visual_line());
        assert_eq!(
            handle_history_key(&mut history, &composer, KeyEvent::from(KeyCode::Up)),
            Some("older".to_string())
        );

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
    fn back_tab_press_is_reverse_focus_navigation() {
        assert!(is_back_tab_press(KeyEvent::from(KeyCode::BackTab)));
        assert!(is_back_tab_press(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::SHIFT
        )));
        assert!(!is_back_tab_press(KeyEvent::from(KeyCode::Tab)));
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

    fn ctx(
        flow_field_count: usize,
        has_target_select: bool,
        has_flow_select: bool,
        option_count: usize,
    ) -> FocusContext {
        FocusContext {
            flow_field_count,
            has_target_select,
            has_flow_select,
            option_count,
        }
    }

    #[test]
    fn focus_cycles_through_options_when_present() {
        let context = ctx(0, false, false, 2);
        assert_eq!(FocusTarget::Name.next(context), FocusTarget::Prompt);
        assert_eq!(FocusTarget::Prompt.next(context), FocusTarget::Options(0));
        assert_eq!(
            FocusTarget::Options(0).next(context),
            FocusTarget::Options(1)
        );
        assert_eq!(FocusTarget::Options(1).next(context), FocusTarget::Name);
    }

    #[test]
    fn focus_cycles_through_target_selector_when_present() {
        let context = ctx(0, true, false, 0);
        assert_eq!(FocusTarget::Prompt.next(context), FocusTarget::TargetSelect);
        assert_eq!(FocusTarget::TargetSelect.next(context), FocusTarget::Name);
        assert_eq!(
            FocusTarget::TargetSelect.next(ctx(0, true, false, 2)),
            FocusTarget::Options(0)
        );
        assert_eq!(
            FocusTarget::Options(1).next(ctx(0, true, false, 2)),
            FocusTarget::Name
        );
    }

    #[test]
    fn focus_visits_flow_widgets_in_visual_order() {
        let context = ctx(2, true, true, 1);
        assert_eq!(FocusTarget::Prompt.next(context), FocusTarget::FlowInput(0));
        assert_eq!(
            FocusTarget::FlowInput(0).next(context),
            FocusTarget::FlowInput(1)
        );
        assert_eq!(
            FocusTarget::FlowInput(1).next(context),
            FocusTarget::TargetSelect
        );
        assert_eq!(
            FocusTarget::TargetSelect.next(context),
            FocusTarget::FlowSelect
        );
        assert_eq!(
            FocusTarget::FlowSelect.next(context),
            FocusTarget::Options(0)
        );
        assert_eq!(FocusTarget::Options(0).next(context), FocusTarget::Name);
    }

    #[test]
    fn focus_prev_reverses_forward_cycle() {
        for flow_field_count in [0usize, 2] {
            for has_target_select in [false, true] {
                for has_flow_select in [false, true] {
                    for option_count in [0usize, 2] {
                        let context = ctx(
                            flow_field_count,
                            has_target_select,
                            has_flow_select,
                            option_count,
                        );
                        let mut targets = vec![FocusTarget::Name, FocusTarget::Prompt];
                        targets.extend((0..flow_field_count).map(FocusTarget::FlowInput));
                        if has_target_select {
                            targets.push(FocusTarget::TargetSelect);
                        }
                        if has_flow_select {
                            targets.push(FocusTarget::FlowSelect);
                        }
                        targets.extend((0..option_count).map(FocusTarget::Options));
                        for focus in targets {
                            assert_eq!(
                                focus.next(context).prev(context),
                                focus,
                                "prev should undo next from {focus:?} ({context:?})"
                            );
                        }
                    }
                }
            }
        }
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
            None,
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
            None,
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
        let context = ctx(0, false, false, 0);
        assert_eq!(FocusTarget::Name.next(context), FocusTarget::Prompt);
        assert_eq!(FocusTarget::Prompt.next(context), FocusTarget::Name);
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
