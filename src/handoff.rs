use std::process::Command;

use anyhow::Context;

use crate::cli::HandoffConfig;

pub fn handoff_argv(config: &HandoffConfig, prompt: &str) -> Vec<String> {
    let mut args = Vec::with_capacity(config.args.len() + 2);
    args.push(config.command.clone());
    args.extend(config.args.iter().cloned());
    args.push(prompt.to_string());
    args
}

pub fn print_command(config: &HandoffConfig, prompt: &str, thread_name: Option<&str>) {
    let rendered = handoff_argv(config, prompt)
        .into_iter()
        .map(|arg| shell_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(thread_name) = thread_name {
        println!("CODEX_THREAD_NAME={} {rendered}", shell_quote(thread_name));
    } else {
        println!("{rendered}");
    }
}

pub fn launch(
    config: &HandoffConfig,
    prompt: &str,
    thread_name: Option<&str>,
) -> anyhow::Result<()> {
    if prompt.trim().is_empty() {
        anyhow::bail!("prompt is empty");
    }

    let mut command = Command::new(&config.command);
    command.args(&config.args).arg(prompt);
    if let Some(thread_name) = thread_name {
        command.env("CODEX_THREAD_NAME", thread_name);
    }
    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", config.command))?;
    if !status.success() {
        anyhow::bail!("handoff command exited with {status}");
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
    use super::*;

    #[test]
    fn prompt_is_final_argv_element() {
        let config = HandoffConfig {
            command: "x".to_string(),
            args: vec!["resume".to_string(), "thread-id".to_string()],
        };
        let prompt = "fix this\nand keep $HOME literal";

        assert_eq!(
            handoff_argv(&config, prompt),
            vec![
                "x".to_string(),
                "resume".to_string(),
                "thread-id".to_string(),
                "fix this\nand keep $HOME literal".to_string(),
            ]
        );
    }

    #[test]
    fn handoff_args_precede_prompt() {
        let config = HandoffConfig {
            command: "x".to_string(),
            args: vec!["fork".to_string(), "--last".to_string()],
        };

        assert_eq!(
            handoff_argv(&config, "continue"),
            vec![
                "x".to_string(),
                "fork".to_string(),
                "--last".to_string(),
                "continue".to_string(),
            ]
        );
    }

    #[test]
    fn empty_prompt_is_rejected() {
        let config = HandoffConfig {
            command: "/bin/echo".to_string(),
            args: Vec::new(),
        };

        let err = launch(&config, " \n\t ", None).expect_err("empty prompt should fail");

        assert!(err.to_string().contains("prompt is empty"));
    }

    #[test]
    fn command_render_can_include_thread_name_env() {
        let config = HandoffConfig {
            command: "x".to_string(),
            args: vec!["resume".to_string()],
        };

        assert_eq!(
            render_command_for_test(&config, "continue", Some("/tmp/project:Fix it")),
            "CODEX_THREAD_NAME='/tmp/project:Fix it' x resume continue"
        );
    }

    fn render_command_for_test(
        config: &HandoffConfig,
        prompt: &str,
        thread_name: Option<&str>,
    ) -> String {
        let rendered = handoff_argv(config, prompt)
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
