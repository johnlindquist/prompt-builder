use std::process::Command;

use anyhow::Context;

use crate::cli::LaunchConfig;

pub fn codex_argv(config: &LaunchConfig, prompt: &str) -> Vec<String> {
    let mut args = vec![
        config.codex_bin.clone(),
        "--dangerously-bypass-approvals-and-sandbox".to_string(),
        "-C".to_string(),
        config.cwd.to_string_lossy().to_string(),
    ];
    if let Some(profile) = &config.profile {
        args.push("--profile".to_string());
        args.push(profile.clone());
    }
    if let Some(model) = &config.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    for entry in &config.config {
        args.push("-c".to_string());
        args.push(entry.clone());
    }
    args.push(prompt.to_string());
    args
}

pub fn print_command(config: &LaunchConfig, prompt: &str) {
    let rendered = codex_argv(config, prompt)
        .into_iter()
        .map(|arg| shell_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    println!("{rendered}");
}

pub fn launch(config: &LaunchConfig, prompt: &str) -> anyhow::Result<()> {
    let mut command = Command::new(&config.codex_bin);
    command
        .arg("--dangerously-bypass-approvals-and-sandbox")
        .arg("-C")
        .arg(&config.cwd);
    if let Some(profile) = &config.profile {
        command.arg("--profile").arg(profile);
    }
    if let Some(model) = &config.model {
        command.arg("--model").arg(model);
    }
    for entry in &config.config {
        command.arg("-c").arg(entry);
    }
    command.arg(prompt);

    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", config.codex_bin))?;
    if !status.success() {
        anyhow::bail!("codex exited with {status}");
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

    #[test]
    fn multiline_prompt_is_one_arg() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp"),
            profile: None,
            model: None,
            config: Vec::new(),
        };

        let args = codex_argv(&config, "one\ntwo");

        assert_eq!(args.last(), Some(&"one\ntwo".to_string()));
    }

    #[test]
    fn config_value_with_spaces_is_preserved() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp"),
            profile: None,
            model: None,
            config: vec!["developer_instructions=debug carefully".to_string()],
        };

        assert_eq!(
            codex_argv(&config, "fix"),
            vec![
                "codex".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                "-C".to_string(),
                "/tmp".to_string(),
                "-c".to_string(),
                "developer_instructions=debug carefully".to_string(),
                "fix".to_string(),
            ]
        );
    }
}
