mod app;
mod cli;
mod codex_spawn;
mod composer_input;
mod handoff;
mod skill_popup;
mod skills;

use std::io::IsTerminal;
use std::io::Read;

use clap::Parser;

use crate::app::AppExit;
use crate::app::SubmittedPrompt;
use crate::app::TemplateInfo;
use crate::cli::Cli;

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
    let thread_name = cli
        .name
        .as_deref()
        .and_then(|name| app::prefixed_thread_name(name, &cwd_name_prefix(&launch_config.cwd)));

    if cli.print_prompt {
        println!("{prompt}");
    }
    if cli.print_command {
        if let Some(handoff_config) = &handoff_config {
            handoff::print_command(handoff_config, &prompt, thread_name.as_deref());
        } else {
            codex_spawn::print_command(&launch_config, &prompt, thread_name.as_deref());
        }
    }
    if cli.print_prompt || cli.print_command {
        return Ok(());
    }

    if cli.submit {
        return finish_submit(
            &launch_config,
            handoff_config.as_ref(),
            SubmittedPrompt {
                prompt,
                thread_name,
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
            handoff::print_command(handoff_config, prompt, submission.thread_name.as_deref());
            return Ok(());
        }
        return handoff::launch(handoff_config, prompt, submission.thread_name.as_deref());
    }
    if dry_run {
        codex_spawn::print_command(config, prompt, submission.thread_name.as_deref());
        return Ok(());
    }
    codex_spawn::launch(config, prompt, submission.thread_name.as_deref())
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
