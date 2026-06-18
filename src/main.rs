mod app;
mod cli;
mod codex_spawn;
mod skill_popup;
mod skills;

use std::io::IsTerminal;
use std::io::Read;

use clap::Parser;

use crate::app::AppExit;
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

    if cli.print_prompt {
        println!("{prompt}");
    }
    if cli.print_command {
        codex_spawn::print_command(&launch_config, &prompt);
    }
    if cli.print_prompt || cli.print_command {
        return Ok(());
    }

    if cli.submit {
        return finish_submit(&launch_config, &prompt, cli.dry_run);
    }

    let skills_dirs = cli::default_skills_dirs(&cli.skills_dirs);
    let skills = skills::load_skills(&skills_dirs);
    match app::run(prompt, skills, launch_config.cwd.clone())? {
        AppExit::Submit(text) => finish_submit(&launch_config, &text, cli.dry_run),
        AppExit::Cancel => Ok(()),
    }
}

fn finish_submit(config: &cli::LaunchConfig, prompt: &str, dry_run: bool) -> anyhow::Result<()> {
    if prompt.trim().is_empty() {
        anyhow::bail!("prompt is empty");
    }
    if dry_run {
        codex_spawn::print_command(config, prompt);
        return Ok(());
    }
    codex_spawn::launch(config, prompt)
}
