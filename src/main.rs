mod app;
mod app_server_launch;
mod claude_spawn;
mod cli;
mod codex_spawn;
mod composer_input;
mod external_editor;
mod file_popup;
mod file_search;
mod handoff;
mod herdr;
mod history;
mod launch_manifest;
mod pi_spawn;
mod skill_popup;
mod skills;
mod slash_commands;
mod slash_popup;
mod target_popup;
mod targets;
mod theme;

use std::io::IsTerminal;
use std::io::Read;

use anyhow::Context;
use clap::Parser;

use crate::app::AppExit;
use crate::app::SubmittedPrompt;
use crate::app::TemplateInfo;
use crate::cli::enabled_option_argv;
use crate::cli::Cli;
use crate::cli::Command;
use crate::cli::HandoffConfig;
use crate::cli::TargetCommand;
use crate::cli::ToggleOption;
use crate::targets::Target;
use crate::targets::TargetKind;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Command::Target(command)) = &cli.command {
        return run_target_command(command);
    }

    let mut prompt = cli.prompt.clone().unwrap_or_default();

    if cli.stdin || !std::io::stdin().is_terminal() {
        let mut stdin = String::new();
        std::io::stdin().read_to_string(&mut stdin)?;
        if !stdin.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(stdin.trim_end_matches('\n'));
        }
    }

    let targets = targets::load_targets()?;
    let initial_target = resolve_target_index(&targets, cli.target.as_deref())?;
    let handoff_config = cli.handoff_config();
    let raw_toggle_options = cli.toggle_options();
    warn_for_ignored_toggle_options(&raw_toggle_options, handoff_config.as_ref());
    let toggle_options = effective_toggle_options(&cli, handoff_config.as_ref());
    let initial_toggled_argv = enabled_option_argv(&toggle_options);
    let conversation_name = cli
        .name
        .as_deref()
        .and_then(app::submitted_conversation_name);
    let thread_name = conversation_name
        .as_deref()
        .and_then(|name| app::prefixed_thread_name(name, &cwd_name_prefix(&cli.cwd)));

    if cli.print_prompt {
        println!("{prompt}");
    }
    if cli.print_command {
        if let Some(handoff_config) = &handoff_config {
            handoff::print_command(
                handoff_config,
                &prompt,
                thread_name.as_deref(),
                &initial_toggled_argv,
            );
        } else {
            print_default_command(
                &cli,
                &targets[initial_target],
                &prompt,
                thread_name.as_deref(),
            );
        }
    }
    if cli.print_launch_json {
        if let Some(handoff_config) = &handoff_config {
            println!(
                "{}",
                launch_manifest::handoff_launch_json(
                    handoff_config,
                    &prompt,
                    thread_name.as_deref(),
                    &toggle_options,
                )
            );
        } else {
            println!(
                "{}",
                default_launch_json_for(
                    &cli,
                    &targets[initial_target],
                    &prompt,
                    thread_name.as_deref(),
                )
            );
        }
    }
    if cli.print_prompt || cli.print_command || cli.print_launch_json {
        return Ok(());
    }

    if cli.submit {
        return finish_submit(
            &cli,
            handoff_config.as_ref(),
            SubmittedPrompt {
                prompt,
                conversation_name,
                thread_name,
                toggled_argv: initial_toggled_argv,
                target: handoff_config
                    .is_none()
                    .then(|| targets[initial_target].clone()),
            },
            cli.dry_run,
        );
    }

    let user_skills_dirs = cli::default_skills_dirs(&cli.skills_dirs);
    let skills = skills::load_skills(&user_skills_dirs, &cli.cwd);
    let template =
        TemplateInfo::from_parts(cli.template_label.clone(), cli.template_description.clone());
    // In handoff mode the wrapper command owns the launch, so the target
    // selector is hidden.
    let tui_targets = if handoff_config.is_some() {
        Vec::new()
    } else {
        targets.clone()
    };
    let loaded_theme = theme::load_active();
    match app::run(
        prompt,
        cli.name.clone().unwrap_or_default(),
        skills,
        cli.cwd.clone(),
        template,
        toggle_options,
        tui_targets,
        initial_target,
        loaded_theme,
        cli.debug_keys.clone(),
    )? {
        AppExit::Submit(submission) => {
            finish_submit(&cli, handoff_config.as_ref(), *submission, cli.dry_run)
        }
        AppExit::Cancel => Ok(()),
    }
}

fn run_target_command(command: &TargetCommand) -> anyhow::Result<()> {
    match command {
        TargetCommand::List => {
            let path = targets::targets_file_path()?;
            if !path.exists() {
                println!(
                    "# built-in defaults; run `prompt-builder target add` to create {}",
                    path.display()
                );
            }
            for target in targets::load_targets()? {
                println!("{}", describe_target(&target));
            }
            Ok(())
        }
        TargetCommand::Add(args) => {
            let target = args.to_target()?;
            let mut targets = targets::load_targets()?;
            let verb = if let Some(existing) = targets
                .iter_mut()
                .find(|existing| existing.name == target.name)
            {
                *existing = target.clone();
                "updated"
            } else {
                targets.push(target.clone());
                "added"
            };
            let path = targets::save_targets(&targets)?;
            println!("{verb} target {:?} in {}", target.name, path.display());
            Ok(())
        }
        TargetCommand::Remove { name } => {
            let mut targets = targets::load_targets()?;
            let before = targets.len();
            targets.retain(|target| &target.name != name);
            if targets.len() == before {
                anyhow::bail!("no target named {name:?}");
            }
            let path = targets::save_targets(&targets)?;
            println!("removed target {name:?} from {}", path.display());
            Ok(())
        }
        TargetCommand::Path => {
            println!("{}", targets::targets_file_path()?.display());
            Ok(())
        }
    }
}

fn describe_target(target: &Target) -> String {
    let mut line = format!(
        "{}\t{}\tbin={}",
        target.name,
        target.kind.label(),
        target.bin()
    );
    for (key, value) in &target.env {
        line.push_str(&format!("\t{key}={value}"));
    }
    if let Some(profile) = &target.profile {
        line.push_str(&format!("\tprofile={profile}"));
    }
    if let Some(model) = &target.model {
        line.push_str(&format!("\tmodel={model}"));
    }
    if !target.config.is_empty() {
        line.push_str(&format!("\tconfig={}", target.config.join(" ")));
    }
    if !target.args.is_empty() {
        line.push_str(&format!("\targs={}", target.args.join(" ")));
    }
    line
}

fn resolve_target_index(targets: &[Target], name: Option<&str>) -> anyhow::Result<usize> {
    let Some(name) = name else {
        return Ok(0);
    };
    targets
        .iter()
        .position(|target| target.name == name)
        .with_context(|| {
            let names = targets
                .iter()
                .map(|target| target.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("unknown target {name:?}; available targets: {names}")
        })
}

fn print_default_command(cli: &Cli, target: &Target, prompt: &str, thread_name: Option<&str>) {
    match target.kind {
        TargetKind::Pi => {
            pi_spawn::print_command(&cli.pi_launch_config_for(target), prompt, thread_name)
        }
        TargetKind::Claude => {
            claude_spawn::print_command(&cli.claude_launch_config_for(target), prompt)
        }
        TargetKind::Codex => {
            let config = cli.launch_config_for(target);
            match thread_name {
                Some(thread_name) => app_server_launch::print_command(&config, prompt, thread_name),
                None => codex_spawn::print_command(&config, prompt, None),
            }
        }
    }
}

fn default_launch_json_for(
    cli: &Cli,
    target: &Target,
    prompt: &str,
    thread_name: Option<&str>,
) -> String {
    match target.kind {
        TargetKind::Pi => {
            launch_manifest::pi_launch_json(&cli.pi_launch_config_for(target), prompt, thread_name)
        }
        TargetKind::Claude => {
            launch_manifest::claude_launch_json(&cli.claude_launch_config_for(target), prompt)
        }
        TargetKind::Codex => launch_manifest::default_launch_json(
            &cli.launch_config_for(target),
            prompt,
            thread_name,
        ),
    }
}

fn finish_submit(
    cli: &Cli,
    handoff_config: Option<&HandoffConfig>,
    submission: SubmittedPrompt,
    dry_run: bool,
) -> anyhow::Result<()> {
    let prompt = submission.prompt.as_str();
    if prompt.trim().is_empty() {
        anyhow::bail!("prompt is empty");
    }
    if let Some(handoff_config) = handoff_config {
        if dry_run {
            handoff::print_command(
                handoff_config,
                prompt,
                submission.thread_name.as_deref(),
                &submission.toggled_argv,
            );
            return Ok(());
        }
        return launch_with_herdr_tab_name(submission.conversation_name.as_deref(), || {
            handoff::launch(
                handoff_config,
                prompt,
                submission.thread_name.as_deref(),
                &submission.toggled_argv,
            )
        });
    }
    if !submission.toggled_argv.is_empty() {
        eprintln!("warning: fork/compact options ignored without --handoff-command");
    }
    let target = submission
        .target
        .as_ref()
        .context("submission did not include a launch target")?;
    match target.kind {
        TargetKind::Pi => {
            let ignored = cli.codex_only_options_in_use();
            if !ignored.is_empty() {
                eprintln!(
                    "warning: {} ignored for pi target {:?}",
                    ignored.join(", "),
                    target.name
                );
            }
            let config = cli.pi_launch_config_for(target);
            if dry_run {
                pi_spawn::print_command(&config, prompt, submission.thread_name.as_deref());
                return Ok(());
            }
            launch_with_herdr_tab_name(submission.conversation_name.as_deref(), || {
                pi_spawn::launch(&config, prompt, submission.thread_name.as_deref())
            })
        }
        TargetKind::Claude => {
            let ignored = cli.codex_only_options_in_use();
            if !ignored.is_empty() {
                eprintln!(
                    "warning: {} ignored for claude target {:?}",
                    ignored.join(", "),
                    target.name
                );
            }
            if let Some(thread_name) = submission.thread_name.as_deref() {
                eprintln!(
                    "warning: conversation name {thread_name:?} is ignored for claude targets"
                );
            }
            let config = cli.claude_launch_config_for(target);
            if dry_run {
                claude_spawn::print_command(&config, prompt);
                return Ok(());
            }
            launch_with_herdr_tab_name(submission.conversation_name.as_deref(), || {
                claude_spawn::launch(&config, prompt)
            })
        }
        TargetKind::Codex => {
            let config = cli.launch_config_for(target);
            if let Some(thread_name) = submission.thread_name.as_deref() {
                if dry_run {
                    app_server_launch::print_command(&config, prompt, thread_name);
                    return Ok(());
                }
                return launch_with_herdr_tab_name(submission.conversation_name.as_deref(), || {
                    app_server_launch::launch(&config, prompt, thread_name)
                });
            }
            if dry_run {
                codex_spawn::print_command(&config, prompt, None);
                return Ok(());
            }
            launch_with_herdr_tab_name(submission.conversation_name.as_deref(), || {
                codex_spawn::launch(&config, prompt, None)
            })
        }
    }
}

fn launch_with_herdr_tab_name(
    conversation_name: Option<&str>,
    launch: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if let Some(name) = conversation_name {
        if let Err(err) = herdr::rename_current_tab(name) {
            eprintln!("warning: failed to rename current Herdr tab: {err}");
        }
    }
    launch()
}

fn effective_toggle_options(
    cli: &Cli,
    handoff_config: Option<&HandoffConfig>,
) -> Vec<ToggleOption> {
    if handoff_config.is_some() {
        cli.toggle_options()
    } else {
        Vec::new()
    }
}

fn warn_for_ignored_toggle_options(
    options: &[ToggleOption],
    handoff_config: Option<&HandoffConfig>,
) {
    if handoff_config.is_none() && !options.is_empty() {
        eprintln!("warning: launch options ignored without --handoff-command");
    }
}

fn cwd_name_prefix(cwd: &std::path::Path) -> String {
    let path = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(cwd))
            .unwrap_or_else(|_| cwd.to_path_buf())
    };
    let path = path.canonicalize().unwrap_or(path);
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::base_cli;

    fn cli_with_launch_options(handoff_command: Option<String>) -> Cli {
        let mut cli = base_cli();
        cli.handoff_command = handoff_command;
        cli.fork_from = Some("last".to_string());
        cli.compact = true;
        cli.launch_options = vec!["trace=--trace".to_string()];
        cli
    }

    #[test]
    fn effective_toggle_options_empty_without_handoff() {
        let cli = cli_with_launch_options(None);

        assert_eq!(effective_toggle_options(&cli, None), Vec::new());
    }

    #[test]
    fn effective_toggle_options_kept_with_handoff() {
        let cli = cli_with_launch_options(Some("x".to_string()));
        let handoff = cli.handoff_config();
        let options = effective_toggle_options(&cli, handoff.as_ref());

        assert_eq!(
            enabled_option_argv(&options),
            vec![
                "--fork-from".to_string(),
                "last".to_string(),
                "--compact".to_string(),
                "--trace".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_target_index_defaults_to_first_and_finds_by_name() {
        let targets = targets::default_targets();

        assert_eq!(resolve_target_index(&targets, None).unwrap(), 0);
        assert_eq!(resolve_target_index(&targets, Some("pi")).unwrap(), 0);
        assert_eq!(resolve_target_index(&targets, Some("codex")).unwrap(), 1);
        assert_eq!(resolve_target_index(&targets, Some("claude")).unwrap(), 2);
    }

    #[test]
    fn resolve_target_index_rejects_unknown_names() {
        let targets = targets::default_targets();

        let err = resolve_target_index(&targets, Some("nope")).expect_err("should fail");

        assert!(err.to_string().contains("unknown target"));
        assert!(err.to_string().contains("pi, codex, claude"));
    }

    #[test]
    fn describe_target_lists_kind_bin_and_env() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("CODEX_HOME".to_string(), "~/.codex-egghead".to_string());
        let target = Target {
            name: "egghead".to_string(),
            kind: TargetKind::Codex,
            env,
            ..Target::default()
        };

        assert_eq!(
            describe_target(&target),
            "egghead\tcodex\tbin=codex\tCODEX_HOME=~/.codex-egghead"
        );
    }
}
