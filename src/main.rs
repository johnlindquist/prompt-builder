mod app;
mod app_server_launch;
mod cli;
mod codex_spawn;
mod composer_input;
mod file_popup;
mod file_search;
mod handoff;
mod launch_manifest;
mod skill_popup;
mod skills;
mod slash_commands;
mod slash_popup;

use std::io::IsTerminal;
use std::io::Read;

use clap::Parser;

use crate::app::AppExit;
use crate::app::SubmittedPrompt;
use crate::app::TemplateInfo;
use crate::cli::enabled_option_argv;
use crate::cli::Cli;
use crate::cli::HandoffConfig;
use crate::cli::ToggleOption;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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

    let launch_config = cli.launch_config();
    let handoff_config = cli.handoff_config();
    let raw_toggle_options = cli.toggle_options();
    warn_for_ignored_toggle_options(&raw_toggle_options, handoff_config.as_ref());
    let toggle_options = effective_toggle_options(&cli, handoff_config.as_ref());
    let initial_toggled_argv = enabled_option_argv(&toggle_options);
    let thread_name = cli
        .name
        .as_deref()
        .and_then(|name| app::prefixed_thread_name(name, &cwd_name_prefix(&launch_config.cwd)));

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
            match thread_name.as_deref() {
                Some(thread_name) => {
                    app_server_launch::print_command(&launch_config, &prompt, thread_name)
                }
                None => codex_spawn::print_command(&launch_config, &prompt, None),
            }
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
                launch_manifest::default_launch_json(
                    &launch_config,
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
            &launch_config,
            handoff_config.as_ref(),
            SubmittedPrompt {
                prompt,
                thread_name,
                toggled_argv: initial_toggled_argv,
            },
            cli.dry_run,
        );
    }

    let skills_dirs = cli::default_skills_dirs(&cli.skills_dirs);
    let skills = skills::load_skills(&skills_dirs);
    let template = TemplateInfo::from_parts(cli.template_label, cli.template_description);
    match app::run(
        prompt,
        cli.name.unwrap_or_default(),
        skills,
        launch_config.cwd.clone(),
        template,
        toggle_options,
        cli.debug_keys,
    )? {
        AppExit::Submit(submission) => finish_submit(
            &launch_config,
            handoff_config.as_ref(),
            submission,
            cli.dry_run,
        ),
        AppExit::Cancel => Ok(()),
    }
}

fn finish_submit(
    config: &cli::LaunchConfig,
    handoff_config: Option<&cli::HandoffConfig>,
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
        return handoff::launch(
            handoff_config,
            prompt,
            submission.thread_name.as_deref(),
            &submission.toggled_argv,
        );
    }
    if !submission.toggled_argv.is_empty() {
        eprintln!("warning: fork/compact options ignored without --handoff-command");
    }
    if let Some(thread_name) = submission.thread_name.as_deref() {
        if dry_run {
            app_server_launch::print_command(config, prompt, thread_name);
            return Ok(());
        }
        return app_server_launch::launch(config, prompt, thread_name);
    }
    if dry_run {
        codex_spawn::print_command(config, prompt, None);
        return Ok(());
    }
    codex_spawn::launch(config, prompt, None)
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
    use std::path::PathBuf;

    use super::*;

    fn cli_with_launch_options(handoff_command: Option<String>) -> Cli {
        Cli {
            prompt: None,
            name: None,
            stdin: false,
            submit: false,
            print_prompt: false,
            print_command: false,
            print_launch_json: false,
            dry_run: false,
            cwd: PathBuf::from("."),
            profile: None,
            model: None,
            config: Vec::new(),
            instructions: None,
            developer_instructions: None,
            template_label: None,
            template_description: None,
            skills_dirs: Vec::new(),
            codex_bin: "codex".to_string(),
            handoff_command,
            handoff_args: Vec::new(),
            fork_from: Some("last".to_string()),
            compact: true,
            launch_options: vec!["trace=--trace".to_string()],
            debug_keys: None,
        }
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
}
