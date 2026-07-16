use std::process::Command;

use anyhow::Context;

use crate::cli::ClaudeLaunchConfig;

pub fn claude_argv(config: &ClaudeLaunchConfig, prompt: &str) -> Vec<String> {
    let mut args = vec![
        config.claude_bin.clone(),
        "--dangerously-skip-permissions".to_string(),
    ];
    if let Some(model) = &config.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    args.extend(config.args.iter().cloned());
    args.push("--".to_string());
    args.push(prompt.to_string());
    args
}

pub fn print_command(config: &ClaudeLaunchConfig, prompt: &str) {
    let rendered = claude_argv(config, prompt)
        .into_iter()
        .map(|arg| shell_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    let env = config
        .env
        .iter()
        .map(|(key, value)| format!("{key}={} ", shell_quote(value)))
        .collect::<String>();
    println!("{env}{rendered}");
}

pub fn launch(config: &ClaudeLaunchConfig, prompt: &str) -> anyhow::Result<()> {
    let mut command = Command::new(&config.claude_bin);
    command.arg("--dangerously-skip-permissions");
    if let Some(model) = &config.model {
        command.arg("--model").arg(model);
    }
    command.args(&config.args);
    command.arg("--").arg(prompt);
    command.current_dir(&config.cwd);
    command.envs(config.env.iter().map(|(key, value)| (key, value)));

    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", config.claude_bin))?;
    if !status.success() {
        anyhow::bail!("claude exited with {status}");
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '='))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn base_config() -> ClaudeLaunchConfig {
        ClaudeLaunchConfig {
            claude_bin: "claude".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            model: None,
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    #[test]
    fn prompt_is_final_arg_after_delimiter() {
        let config = base_config();

        assert_eq!(
            claude_argv(&config, "fix this\nplease"),
            vec![
                "claude".to_string(),
                "--dangerously-skip-permissions".to_string(),
                "--".to_string(),
                "fix this\nplease".to_string(),
            ]
        );
    }

    #[test]
    fn model_and_extra_args_precede_prompt() {
        let mut config = base_config();
        config.model = Some("opus".to_string());
        config.args = vec!["--verbose".to_string()];

        assert_eq!(
            claude_argv(&config, "-hyphen prompt"),
            vec![
                "claude".to_string(),
                "--dangerously-skip-permissions".to_string(),
                "--model".to_string(),
                "opus".to_string(),
                "--verbose".to_string(),
                "--".to_string(),
                "-hyphen prompt".to_string(),
            ]
        );
    }
}
