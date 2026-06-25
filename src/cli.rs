use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Terminal prompt builder for handing prompts to Codex"
)]
pub struct Cli {
    /// Prompt text to prefill or submit.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Conversation name to prefill or submit. The cwd prefix is added on submit.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Read stdin and append it to the initial prompt.
    #[arg(long)]
    pub stdin: bool,

    /// Submit without opening the TUI. Useful for pipes and shell functions.
    #[arg(long)]
    pub submit: bool,

    /// Print the composed prompt and exit.
    #[arg(long)]
    pub print_prompt: bool,

    /// Print the Codex command argv and exit.
    #[arg(long)]
    pub print_command: bool,

    /// Print structured launch JSON and exit.
    #[arg(long)]
    pub print_launch_json: bool,

    /// Do not launch Codex.
    #[arg(long)]
    pub dry_run: bool,

    /// Working directory passed to Codex with -C.
    #[arg(long = "cwd", short = 'C', value_name = "DIR", default_value = ".")]
    pub cwd: PathBuf,

    /// Codex profile passed through with --profile.
    #[arg(long, short = 'p')]
    pub profile: Option<String>,

    /// Model passed through with --model.
    #[arg(long, short = 'm')]
    pub model: Option<String>,

    /// Codex config override passed through as -c KEY=VALUE. Repeatable.
    #[arg(long = "config", short = 'c', value_name = "KEY=VALUE")]
    pub config: Vec<String>,

    /// Shorthand for -c instructions=<text>.
    #[arg(long)]
    pub instructions: Option<String>,

    /// Shorthand for -c developer_instructions=<text>.
    #[arg(long)]
    pub developer_instructions: Option<String>,

    /// Label for wrapper-specific context shown in the TUI header.
    #[arg(long = "template-label", value_name = "LABEL")]
    pub template_label: Option<String>,

    /// Description for wrapper-specific context shown in the TUI header.
    #[arg(long = "template-description", value_name = "TEXT")]
    pub template_description: Option<String>,

    /// Skills directory to scan. Repeatable.
    #[arg(long = "skills-dir", value_name = "DIR")]
    pub skills_dirs: Vec<PathBuf>,

    /// Codex executable to launch.
    #[arg(
        long = "codex-bin",
        env = "PROMPT_BUILDER_CODEX_BIN",
        default_value = "codex"
    )]
    pub codex_bin: String,

    /// Command to run with the composed prompt as its final argument.
    #[arg(long = "handoff-command", value_name = "PROGRAM")]
    pub handoff_command: Option<String>,

    /// Argument to pass to --handoff-command before the composed prompt. Repeatable.
    #[arg(long = "handoff-arg", value_name = "ARG", allow_hyphen_values = true)]
    pub handoff_args: Vec<String>,

    /// Fork source for wrapper/app-server handoff flows. Rendered as a toggle in the TUI.
    #[arg(long = "fork-from", value_name = "SOURCE")]
    pub fork_from: Option<String>,

    /// Enable compacting in wrapper/app-server handoff flows. Rendered as a toggle in the TUI.
    #[arg(long)]
    pub compact: bool,

    /// Additional wrapper/app-server option toggle as LABEL=ARG[,ARG...]. Repeatable.
    #[arg(long = "launch-option", value_name = "LABEL=ARG[,ARG...]")]
    pub launch_options: Vec<String>,

    /// Write structured key routing events as JSON lines for terminal debugging.
    #[arg(
        long = "debug-keys",
        env = "PROMPT_BUILDER_DEBUG_KEYS",
        value_name = "FILE"
    )]
    pub debug_keys: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub codex_bin: String,
    pub cwd: PathBuf,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub config: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffConfig {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToggleOption {
    pub label: String,
    pub argv: Vec<String>,
    pub enabled: bool,
}

impl Cli {
    pub fn launch_config(&self) -> LaunchConfig {
        let mut config = self.config.clone();
        if let Some(instructions) = &self.instructions {
            config.push(format!("instructions={}", toml_string(instructions)));
        }
        if let Some(developer_instructions) = &self.developer_instructions {
            config.push(format!(
                "developer_instructions={}",
                toml_string(developer_instructions)
            ));
        }

        LaunchConfig {
            codex_bin: self.codex_bin.clone(),
            cwd: self.cwd.clone(),
            profile: self.profile.clone(),
            model: self.model.clone(),
            config,
        }
    }

    pub fn handoff_config(&self) -> Option<HandoffConfig> {
        self.handoff_command.as_ref().map(|command| HandoffConfig {
            command: command.clone(),
            args: self.handoff_args.clone(),
        })
    }

    pub fn toggle_options(&self) -> Vec<ToggleOption> {
        let mut options = Vec::new();
        if let Some(source) = self
            .fork_from
            .as_deref()
            .map(str::trim)
            .filter(|source| !source.is_empty())
        {
            options.push(ToggleOption {
                label: format!("fork from: {source}"),
                argv: vec!["--fork-from".to_string(), source.to_string()],
                enabled: true,
            });
        }
        if self.compact {
            options.push(ToggleOption {
                label: "compact".to_string(),
                argv: vec!["--compact".to_string()],
                enabled: true,
            });
        }
        options.extend(
            self.launch_options
                .iter()
                .filter_map(|option| parse_launch_option(option)),
        );
        options
    }
}

fn parse_launch_option(value: &str) -> Option<ToggleOption> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (label, argv_text) = value.split_once('=').unwrap_or((value, value));
    let label = label.trim();
    let argv = argv_text
        .split(',')
        .map(str::trim)
        .filter(|arg| !arg.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if label.is_empty() || argv.is_empty() {
        return None;
    }
    Some(ToggleOption {
        label: label.to_string(),
        argv,
        enabled: true,
    })
}

pub fn enabled_option_argv(options: &[ToggleOption]) -> Vec<String> {
    options
        .iter()
        .filter(|option| option.enabled)
        .flat_map(|option| option.argv.iter().cloned())
        .collect()
}

pub fn default_skills_dirs(cli_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if !cli_dirs.is_empty() {
        return cli_dirs.to_vec();
    }

    dirs::home_dir()
        .map(|home| vec![home.join(".agents").join("skills")])
        .unwrap_or_default()
}

fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_config_values_are_toml_strings() {
        let cli = Cli {
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
            instructions: Some("base\nrules".to_string()),
            developer_instructions: Some("debug \"carefully\"".to_string()),
            template_label: None,
            template_description: None,
            skills_dirs: Vec::new(),
            codex_bin: "codex".to_string(),
            handoff_command: None,
            handoff_args: Vec::new(),
            fork_from: None,
            compact: false,
            launch_options: Vec::new(),
            debug_keys: None,
        };

        assert_eq!(
            cli.launch_config().config,
            vec![
                "instructions=\"base\\nrules\"".to_string(),
                "developer_instructions=\"debug \\\"carefully\\\"\"".to_string(),
            ]
        );
    }

    #[test]
    fn toggle_options_default_to_enabled_when_flags_are_present() {
        let cli = Cli {
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
            handoff_command: None,
            handoff_args: Vec::new(),
            fork_from: Some("last".to_string()),
            compact: true,
            launch_options: Vec::new(),
            debug_keys: None,
        };

        assert_eq!(
            cli.toggle_options(),
            vec![
                ToggleOption {
                    label: "fork from: last".to_string(),
                    argv: vec!["--fork-from".to_string(), "last".to_string()],
                    enabled: true,
                },
                ToggleOption {
                    label: "compact".to_string(),
                    argv: vec!["--compact".to_string()],
                    enabled: true,
                },
            ]
        );
    }

    #[test]
    fn enabled_option_argv_excludes_disabled_options() {
        let options = vec![
            ToggleOption {
                label: "fork from: last".to_string(),
                argv: vec!["--fork-from".to_string(), "last".to_string()],
                enabled: false,
            },
            ToggleOption {
                label: "compact".to_string(),
                argv: vec!["--compact".to_string()],
                enabled: true,
            },
        ];

        assert_eq!(enabled_option_argv(&options), vec!["--compact".to_string()]);
    }

    #[test]
    fn launch_option_adds_custom_enabled_toggle() {
        let option = parse_launch_option("mode=--mode,review").expect("option should parse");

        assert_eq!(
            option,
            ToggleOption {
                label: "mode".to_string(),
                argv: vec!["--mode".to_string(), "review".to_string()],
                enabled: true,
            }
        );
    }
}
