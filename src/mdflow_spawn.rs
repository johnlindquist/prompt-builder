use std::process::Command;

use anyhow::Context;

use crate::cli::MdflowLaunchConfig;

/// mdflow has no `--` positional delimiter (it breaks template-var mapping),
/// so hyphen-leading prompts get a protective space like pi_spawn does.
fn guard_prompt(prompt: &str) -> String {
    if prompt.starts_with('-') {
        format!(" {prompt}")
    } else {
        prompt.to_string()
    }
}

/// Argv: mdflow <flow> <target args…> --_name=value… <prompt>.
/// Template-var flags are single `--_key=value` tokens so values may start
/// with `-`; the prompt is the first positional and fills `{{ _1 }}`.
pub fn mdflow_argv(
    config: &MdflowLaunchConfig,
    flow_path: &str,
    values: &[(String, String)],
    prompt: &str,
) -> Vec<String> {
    let mut args = vec![config.mdflow_bin.clone(), flow_path.to_string()];
    args.extend(config.args.iter().cloned());
    for (name, value) in values {
        args.push(format!("--_{name}={value}"));
    }
    args.push(guard_prompt(prompt));
    args
}

pub fn print_command(
    config: &MdflowLaunchConfig,
    flow_path: &str,
    values: &[(String, String)],
    prompt: &str,
) {
    let rendered = mdflow_argv(config, flow_path, values, prompt)
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
    config: &MdflowLaunchConfig,
    flow_path: &str,
    values: &[(String, String)],
    prompt: &str,
) -> anyhow::Result<()> {
    let argv = mdflow_argv(config, flow_path, values, prompt);
    let mut command = Command::new(&config.mdflow_bin);
    command.args(&argv[1..]);
    command.current_dir(&config.cwd);
    command.envs(config.env.iter().map(|(key, value)| (key, value)));

    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", config.mdflow_bin))?;
    if !status.success() {
        anyhow::bail!("mdflow exited with {status}");
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

    fn config() -> MdflowLaunchConfig {
        MdflowLaunchConfig {
            mdflow_bin: "mdflow".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    #[test]
    fn prompt_is_first_positional_after_value_flags() {
        let values = vec![
            ("severity".to_string(), "high".to_string()),
            ("2".to_string(), "French".to_string()),
        ];

        assert_eq!(
            mdflow_argv(&config(), "flows/review.md", &values, "fix the bug"),
            vec![
                "mdflow".to_string(),
                "flows/review.md".to_string(),
                "--_severity=high".to_string(),
                "--_2=French".to_string(),
                "fix the bug".to_string(),
            ]
        );
    }

    #[test]
    fn target_args_come_before_value_flags() {
        let mut config = config();
        config.args = vec!["--events".to_string()];

        assert_eq!(
            mdflow_argv(&config, "flows/review.md", &[], "fix"),
            vec![
                "mdflow".to_string(),
                "flows/review.md".to_string(),
                "--events".to_string(),
                "fix".to_string(),
            ]
        );
    }

    #[test]
    fn hyphen_prefixed_prompt_gets_protective_space() {
        let args = mdflow_argv(&config(), "flows/review.md", &[], "-create an imp");

        assert_eq!(args.last(), Some(&" -create an imp".to_string()));
    }

    #[test]
    fn multiline_prompt_is_one_arg() {
        let args = mdflow_argv(&config(), "flows/review.md", &[], "one\ntwo");

        assert_eq!(args.last(), Some(&"one\ntwo".to_string()));
    }

    #[test]
    fn value_with_spaces_and_leading_dash_stays_single_token() {
        let values = vec![("note".to_string(), "-x has spaces".to_string())];

        let args = mdflow_argv(&config(), "f.md", &values, "go");

        assert_eq!(args[2], "--_note=-x has spaces");
    }
}
