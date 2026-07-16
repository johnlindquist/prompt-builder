use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::SystemTime;

use anyhow::Context;

use crate::cli::PiLaunchConfig;

pub fn pi_argv(config: &PiLaunchConfig, prompt: &str, session_name: Option<&str>) -> Vec<String> {
    let mut args = vec![config.pi_bin.clone()];
    if let Some(model) = &config.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    if let Some(session_name) = session_name {
        args.push("--name".to_string());
        args.push(session_name.to_string());
    }
    args.extend(config.args.iter().cloned());
    args.push(pi_prompt_arg(prompt));
    args
}

fn pi_prompt_arg(prompt: &str) -> String {
    // Pi parses leading '@' as a file and leading '-' as an option. Its parser
    // has no `--` delimiter, so protect the positional prompt in transit.
    if prompt.starts_with('@') || prompt.starts_with('-') {
        format!(" {prompt}")
    } else {
        prompt.to_string()
    }
}

pub fn print_command(config: &PiLaunchConfig, prompt: &str, session_name: Option<&str>) {
    let rendered = pi_argv(config, prompt, session_name)
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

pub fn launch(
    config: &PiLaunchConfig,
    prompt: &str,
    session_name: Option<&str>,
) -> anyhow::Result<()> {
    remove_stale_incompatible_auth_lock(config);
    let argv = pi_argv(config, prompt, session_name);
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(&config.cwd)
        .envs(config.env.iter().map(|(key, value)| (key, value)));

    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", config.pi_bin))?;
    if !status.success() {
        anyhow::bail!("pi exited with {status}");
    }
    Ok(())
}

fn remove_stale_incompatible_auth_lock(config: &PiLaunchConfig) {
    remove_incompatible_auth_lock_older_than(config, Duration::from_secs(30));
}

fn remove_incompatible_auth_lock_older_than(config: &PiLaunchConfig, stale_after: Duration) {
    let agent_dir = config
        .env
        .iter()
        .find(|(key, _)| key == "PI_CODING_AGENT_DIR")
        .map(|(_, value)| PathBuf::from(value))
        .or_else(|| std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(".pi/agent")));
    let Some(agent_dir) = agent_dir else {
        return;
    };
    let lock_path = agent_dir.join("auth.json.lock");
    let Ok(metadata) = std::fs::symlink_metadata(&lock_path) else {
        return;
    };

    // Pi uses proper-lockfile, whose valid lock is a directory. A regular file
    // at this path makes every fresh Pi process silently load zero credentials.
    let is_stale_regular_file = metadata.file_type().is_file()
        && metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= stale_after);
    if is_stale_regular_file {
        match std::fs::remove_file(&lock_path) {
            Ok(()) => eprintln!(
                "warning: removed stale incompatible Pi auth lock {}",
                lock_path.display()
            ),
            Err(err) => eprintln!(
                "warning: could not remove stale incompatible Pi auth lock {}: {err}",
                lock_path.display()
            ),
        }
    }
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

    fn base_config() -> PiLaunchConfig {
        PiLaunchConfig {
            pi_bin: "pi".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            model: None,
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    #[test]
    fn prompt_is_one_final_arg_without_print_mode_or_delimiter() {
        assert_eq!(
            pi_argv(&base_config(), "fix this\nplease", None),
            vec!["pi".to_string(), "fix this\nplease".to_string()]
        );
    }

    #[test]
    fn model_name_and_extra_args_precede_prompt() {
        let mut config = base_config();
        config.model = Some("openai/gpt-4o".to_string());
        config.args = vec!["--thinking".to_string(), "high".to_string()];

        assert_eq!(
            pi_argv(&config, "fix", Some("project:Fix")),
            vec![
                "pi",
                "--model",
                "openai/gpt-4o",
                "--name",
                "project:Fix",
                "--thinking",
                "high",
                "fix",
            ]
        );
    }

    #[test]
    fn option_like_and_file_like_prompts_are_protected() {
        let option_prompt = pi_argv(&base_config(), "-fix this", None);
        assert_eq!(option_prompt.last().map(String::as_str), Some(" -fix this"));

        let file_prompt = pi_argv(&base_config(), "@literal syntax", None);
        assert_eq!(
            file_prompt.last().map(String::as_str),
            Some(" @literal syntax")
        );
    }

    #[test]
    fn removes_only_stale_regular_auth_locks() -> anyhow::Result<()> {
        let agent_dir = std::env::temp_dir().join(format!(
            "prompt-builder-pi-auth-lock-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&agent_dir);
        std::fs::create_dir_all(&agent_dir)?;
        let lock_path = agent_dir.join("auth.json.lock");
        std::fs::write(&lock_path, [])?;

        let mut config = base_config();
        config.env.push((
            "PI_CODING_AGENT_DIR".to_string(),
            agent_dir.to_string_lossy().to_string(),
        ));
        remove_stale_incompatible_auth_lock(&config);
        assert!(lock_path.exists(), "fresh lock must be preserved");

        remove_incompatible_auth_lock_older_than(&config, Duration::ZERO);
        assert!(!lock_path.exists(), "stale regular lock must be removed");

        std::fs::create_dir(&lock_path)?;
        remove_stale_incompatible_auth_lock(&config);
        assert!(
            lock_path.is_dir(),
            "proper-lockfile directory must be preserved"
        );
        let _ = std::fs::remove_dir_all(&agent_dir);
        Ok(())
    }
}
