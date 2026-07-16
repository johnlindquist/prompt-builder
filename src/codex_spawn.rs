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
    args.extend(config.args.iter().cloned());
    args.push("--".to_string());
    args.push(prompt.to_string());
    args
}

pub fn resume_argv(config: &LaunchConfig, thread_id: &str, prompt: &str) -> Vec<String> {
    let mut args = vec![
        config.codex_bin.clone(),
        "resume".to_string(),
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
    args.extend(config.args.iter().cloned());
    args.push("--".to_string());
    args.push(thread_id.to_string());
    args.push(prompt.to_string());
    args
}

pub fn print_command(config: &LaunchConfig, prompt: &str, thread_name: Option<&str>) {
    let rendered = codex_argv(config, prompt)
        .into_iter()
        .map(|arg| shell_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    let mut prefix = env_prefix(&config.env);
    if let Some(thread_name) = thread_name {
        prefix.push_str(&format!("CODEX_THREAD_NAME={} ", shell_quote(thread_name)));
    }
    println!("{prefix}{rendered}");
}

pub fn env_prefix(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(key, value)| format!("{key}={} ", shell_quote(value)))
        .collect()
}

pub fn launch(
    config: &LaunchConfig,
    prompt: &str,
    thread_name: Option<&str>,
) -> anyhow::Result<()> {
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
    command.args(&config.args);
    command.arg("--");
    command.arg(prompt);
    command.envs(config.env.iter().map(|(key, value)| (key, value)));
    if let Some(thread_name) = thread_name {
        command.env("CODEX_THREAD_NAME", thread_name);
    }

    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", config.codex_bin))?;
    if !status.success() {
        anyhow::bail!("codex exited with {status}");
    }
    Ok(())
}

pub fn launch_resume(
    config: &LaunchConfig,
    thread_id: &str,
    prompt: &str,
    thread_name: Option<&str>,
) -> anyhow::Result<()> {
    let mut command = Command::new(&config.codex_bin);
    command
        .arg("resume")
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
    command.args(&config.args);
    command.arg("--").arg(thread_id).arg(prompt);
    command.envs(config.env.iter().map(|(key, value)| (key, value)));
    if let Some(thread_name) = thread_name {
        command.env("CODEX_THREAD_NAME", thread_name);
    }

    let status = command
        .status()
        .with_context(|| format!("failed to launch {} resume", config.codex_bin))?;
    if !status.success() {
        anyhow::bail!("codex resume exited with {status}");
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
            args: Vec::new(),
            env: Vec::new(),
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
            args: Vec::new(),
            env: Vec::new(),
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
                "--".to_string(),
                "fix".to_string(),
            ]
        );
    }

    #[test]
    fn resume_argv_preserves_overrides_before_thread_id_and_prompt() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            profile: Some("fixit".to_string()),
            model: Some("gpt-5.5".to_string()),
            config: vec!["developer_instructions=debug carefully".to_string()],
            args: Vec::new(),
            env: Vec::new(),
        };

        assert_eq!(
            resume_argv(&config, "thread-id", "fix"),
            vec![
                "codex".to_string(),
                "resume".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                "-C".to_string(),
                "/tmp/project".to_string(),
                "--profile".to_string(),
                "fixit".to_string(),
                "--model".to_string(),
                "gpt-5.5".to_string(),
                "-c".to_string(),
                "developer_instructions=debug carefully".to_string(),
                "--".to_string(),
                "thread-id".to_string(),
                "fix".to_string(),
            ]
        );
    }

    #[test]
    fn codex_argv_delimits_hyphen_prefixed_prompt() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp"),
            profile: None,
            model: None,
            config: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
        };

        assert_eq!(
            codex_argv(&config, "-create an imp").as_slice(),
            [
                "codex",
                "--dangerously-bypass-approvals-and-sandbox",
                "-C",
                "/tmp",
                "--",
                "-create an imp",
            ]
        );
    }

    #[test]
    fn resume_argv_delimits_hyphen_prefixed_prompt() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp"),
            profile: None,
            model: None,
            config: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
        };

        assert_eq!(
            resume_argv(&config, "thread-id", "-create an imp").as_slice(),
            [
                "codex",
                "resume",
                "--dangerously-bypass-approvals-and-sandbox",
                "-C",
                "/tmp",
                "--",
                "thread-id",
                "-create an imp",
            ]
        );
    }

    #[test]
    fn printed_command_can_include_thread_name_env() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            profile: None,
            model: None,
            config: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
        };

        assert_eq!(
            render_command_for_test(&config, "fix", Some("/tmp/project:Fix")),
            "CODEX_THREAD_NAME=/tmp/project:Fix codex --dangerously-bypass-approvals-and-sandbox -C /tmp/project -- fix"
        );
    }

    fn render_command_for_test(
        config: &LaunchConfig,
        prompt: &str,
        thread_name: Option<&str>,
    ) -> String {
        let rendered = codex_argv(config, prompt)
            .into_iter()
            .map(|arg| shell_quote(&arg))
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(thread_name) = thread_name {
            format!("CODEX_THREAD_NAME={} {rendered}", shell_quote(thread_name))
        } else {
            rendered
        }
    }
}
