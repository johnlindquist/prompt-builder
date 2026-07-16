use std::path::PathBuf;

use clap::Args;
use clap::Parser;
use clap::Subcommand;

use crate::targets::Target;
use crate::targets::TargetKind;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Terminal prompt builder for handing prompts to Pi, Codex, or Claude Code"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Prompt text to prefill or submit.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Launch target (profile) name from ~/.prompt-builder/targets.toml.
    #[arg(long, short = 't', value_name = "NAME")]
    pub target: Option<String>,

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

    /// Print the selected target command argv and exit.
    #[arg(long)]
    pub print_command: bool,

    /// Print structured launch JSON and exit.
    #[arg(long)]
    pub print_launch_json: bool,

    /// Do not launch the selected target.
    #[arg(long)]
    pub dry_run: bool,

    /// Working directory. Codex receives -C; Pi and Claude use it as process cwd.
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

    /// User skills directory to scan instead of ~/.agents/skills. Repeatable.
    /// Project-local .agents/skills directories are discovered from --cwd.
    #[arg(long = "skills-dir", value_name = "DIR")]
    pub skills_dirs: Vec<PathBuf>,

    /// Pi executable to launch for pi targets.
    #[arg(long = "pi-bin", env = "PROMPT_BUILDER_PI_BIN", default_value = "pi")]
    pub pi_bin: String,

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

    /// Render --fork-from as a disabled toggle by default.
    #[arg(long = "fork-default-off")]
    pub fork_default_off: bool,

    /// Enable compacting in wrapper/app-server handoff flows. Rendered as a toggle in the TUI.
    #[arg(long)]
    pub compact: bool,

    /// Additional wrapper/app-server option toggle as LABEL=ARG[,ARG...]. Repeatable.
    #[arg(long = "launch-option", value_name = "LABEL=ARG[,ARG...]")]
    pub launch_options: Vec<String>,

    /// Claude Code executable to launch for claude targets.
    #[arg(
        long = "claude-bin",
        env = "PROMPT_BUILDER_CLAUDE_BIN",
        default_value = "claude"
    )]
    pub claude_bin: String,

    /// Write structured key routing events as JSON lines for terminal debugging.
    #[arg(
        long = "debug-keys",
        env = "PROMPT_BUILDER_DEBUG_KEYS",
        value_name = "FILE"
    )]
    pub debug_keys: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage launch targets (profiles) in ~/.prompt-builder/targets.toml.
    #[command(subcommand)]
    Target(TargetCommand),
}

#[derive(Debug, Subcommand)]
pub enum TargetCommand {
    /// List configured targets.
    List,
    /// Add a target, or update the target with the same name.
    Add(TargetAddArgs),
    /// Remove a target by name.
    Remove {
        /// Target name to remove.
        name: String,
    },
    /// Print the targets file path.
    Path,
}

#[derive(Debug, Args)]
pub struct TargetAddArgs {
    /// Target name shown in the selector.
    pub name: String,

    /// Target kind: pi, codex, or claude.
    #[arg(long, value_name = "pi|codex|claude", default_value = "pi")]
    pub kind: String,

    /// Executable override. Defaults to `pi`, `codex`, or `claude` by kind.
    #[arg(long)]
    pub bin: Option<String>,

    /// Environment variable set on launch, e.g. CODEX_HOME=~/.codex-egghead. Repeatable.
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,

    /// Codex profile passed as --profile. Codex targets only.
    #[arg(long)]
    pub profile: Option<String>,

    /// Model passed as --model.
    #[arg(long)]
    pub model: Option<String>,

    /// Codex config override passed as -c KEY=VALUE. Repeatable. Codex targets only.
    #[arg(long = "config", short = 'c', value_name = "KEY=VALUE")]
    pub config: Vec<String>,

    /// Extra argument inserted before the prompt. Repeatable.
    #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
    pub args: Vec<String>,
}

impl TargetAddArgs {
    pub fn to_target(&self) -> anyhow::Result<Target> {
        let name = self.name.trim();
        if name.is_empty() {
            anyhow::bail!("target name is empty");
        }
        let mut env = std::collections::BTreeMap::new();
        for entry in &self.env {
            let (key, value) = entry
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--env {entry:?} is not KEY=VALUE"))?;
            env.insert(key.trim().to_string(), value.to_string());
        }
        Ok(Target {
            name: name.to_string(),
            kind: TargetKind::parse(&self.kind)?,
            bin: self.bin.clone(),
            env,
            profile: self.profile.clone(),
            model: self.model.clone(),
            config: self.config.clone(),
            args: self.args.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub codex_bin: String,
    pub cwd: PathBuf,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub config: Vec<String>,
    /// Extra argv inserted after config overrides, before the prompt.
    pub args: Vec<String>,
    /// Environment variables set on the launched process.
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiLaunchConfig {
    pub pi_bin: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    /// Pi-specific argv inserted before the prompt.
    pub args: Vec<String>,
    /// Environment variables set on the launched process.
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeLaunchConfig {
    pub claude_bin: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    /// Extra argv inserted before the prompt.
    pub args: Vec<String>,
    /// Environment variables set on the launched process.
    pub env: Vec<(String, String)>,
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
    /// Codex launch config for a codex-kind target. Target values fill in
    /// wherever the CLI did not override them; target config entries come
    /// first so CLI -c entries win in Codex's last-wins semantics.
    pub fn launch_config_for(&self, target: &Target) -> LaunchConfig {
        let mut config = target.config.clone();
        config.extend(self.config.iter().cloned());
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
            codex_bin: target.bin.clone().unwrap_or_else(|| self.codex_bin.clone()),
            cwd: self.cwd.clone(),
            profile: self.profile.clone().or_else(|| target.profile.clone()),
            model: self.model.clone().or_else(|| target.model.clone()),
            config,
            args: target.args.clone(),
            env: target.env_pairs(),
        }
    }

    /// Pi launch config for a pi-kind target.
    pub fn pi_launch_config_for(&self, target: &Target) -> PiLaunchConfig {
        PiLaunchConfig {
            pi_bin: target.bin.clone().unwrap_or_else(|| self.pi_bin.clone()),
            cwd: self.cwd.clone(),
            model: self.model.clone().or_else(|| target.model.clone()),
            args: target.args.clone(),
            env: target.env_pairs(),
        }
    }

    /// Claude Code launch config for a claude-kind target.
    pub fn claude_launch_config_for(&self, target: &Target) -> ClaudeLaunchConfig {
        ClaudeLaunchConfig {
            claude_bin: target
                .bin
                .clone()
                .unwrap_or_else(|| self.claude_bin.clone()),
            cwd: self.cwd.clone(),
            model: self.model.clone().or_else(|| target.model.clone()),
            args: target.args.clone(),
            env: target.env_pairs(),
        }
    }

    /// Codex-only CLI options that a non-Codex target cannot honor.
    pub fn codex_only_options_in_use(&self) -> Vec<&'static str> {
        let mut ignored = Vec::new();
        if self.profile.is_some() {
            ignored.push("--profile");
        }
        if !self.config.is_empty() {
            ignored.push("-c/--config");
        }
        if self.instructions.is_some() {
            ignored.push("--instructions");
        }
        if self.developer_instructions.is_some() {
            ignored.push("--developer-instructions");
        }
        ignored
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
                enabled: !self.fork_default_off,
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
pub(crate) fn base_cli() -> Cli {
    Cli {
        command: None,
        prompt: None,
        target: None,
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
        pi_bin: "pi".to_string(),
        codex_bin: "codex".to_string(),
        claude_bin: "claude".to_string(),
        handoff_command: None,
        handoff_args: Vec::new(),
        fork_from: None,
        fork_default_off: false,
        compact: false,
        launch_options: Vec::new(),
        debug_keys: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::TargetKind;

    fn codex_target() -> Target {
        Target {
            name: "codex".to_string(),
            kind: TargetKind::Codex,
            ..Target::default()
        }
    }

    #[test]
    fn shorthand_config_values_are_toml_strings() {
        let mut cli = base_cli();
        cli.instructions = Some("base\nrules".to_string());
        cli.developer_instructions = Some("debug \"carefully\"".to_string());

        assert_eq!(
            cli.launch_config_for(&codex_target()).config,
            vec![
                "instructions=\"base\\nrules\"".to_string(),
                "developer_instructions=\"debug \\\"carefully\\\"\"".to_string(),
            ]
        );
    }

    #[test]
    fn target_values_fill_in_when_cli_does_not_override() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("CODEX_HOME".to_string(), "/homes/egghead".to_string());
        let target = Target {
            name: "egghead".to_string(),
            kind: TargetKind::Codex,
            bin: Some("/opt/codex".to_string()),
            env,
            profile: Some("teach".to_string()),
            model: Some("gpt-5.5".to_string()),
            config: vec!["cli_auth_credentials_store=\"file\"".to_string()],
            args: vec!["--verbose".to_string()],
        };
        let mut cli = base_cli();
        cli.model = Some("gpt-6".to_string());
        cli.config = vec!["sandbox_mode=\"off\"".to_string()];

        let config = cli.launch_config_for(&target);

        assert_eq!(config.codex_bin, "/opt/codex");
        assert_eq!(config.profile, Some("teach".to_string()));
        assert_eq!(config.model, Some("gpt-6".to_string()));
        assert_eq!(
            config.config,
            vec![
                "cli_auth_credentials_store=\"file\"".to_string(),
                "sandbox_mode=\"off\"".to_string(),
            ]
        );
        assert_eq!(config.args, vec!["--verbose".to_string()]);
        assert_eq!(
            config.env,
            vec![("CODEX_HOME".to_string(), "/homes/egghead".to_string())]
        );
    }

    #[test]
    fn claude_launch_config_uses_target_bin_env_and_model() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("CLAUDE_CONFIG_DIR".to_string(), "/homes/second".to_string());
        let target = Target {
            name: "claude-second".to_string(),
            kind: TargetKind::Claude,
            env,
            model: Some("opus".to_string()),
            args: vec!["--verbose".to_string()],
            ..Target::default()
        };
        let cli = base_cli();

        let config = cli.claude_launch_config_for(&target);

        assert_eq!(config.claude_bin, "claude");
        assert_eq!(config.model, Some("opus".to_string()));
        assert_eq!(config.args, vec!["--verbose".to_string()]);
        assert_eq!(
            config.env,
            vec![("CLAUDE_CONFIG_DIR".to_string(), "/homes/second".to_string())]
        );
    }

    #[test]
    fn pi_launch_config_uses_target_overrides_and_cli_model() {
        let mut env = std::collections::BTreeMap::new();
        env.insert(
            "PI_CODING_AGENT_DIR".to_string(),
            "/homes/work/agent".to_string(),
        );
        let target = Target {
            name: "pi-work".to_string(),
            kind: TargetKind::Pi,
            bin: Some("/opt/pi".to_string()),
            env,
            model: Some("anthropic/claude-sonnet".to_string()),
            args: vec!["--thinking".to_string(), "high".to_string()],
            ..Target::default()
        };
        let mut cli = base_cli();
        cli.model = Some("openai/gpt-5".to_string());

        let config = cli.pi_launch_config_for(&target);

        assert_eq!(config.pi_bin, "/opt/pi");
        assert_eq!(config.model, Some("openai/gpt-5".to_string()));
        assert_eq!(config.args, vec!["--thinking", "high"]);
        assert_eq!(
            config.env,
            vec![(
                "PI_CODING_AGENT_DIR".to_string(),
                "/homes/work/agent".to_string()
            )]
        );
    }

    #[test]
    fn target_add_defaults_to_pi() {
        let cli = Cli::try_parse_from(["prompt-builder", "target", "add", "work"])
            .expect("CLI should parse");
        let Some(Command::Target(TargetCommand::Add(args))) = cli.command else {
            panic!("expected target add command");
        };

        assert_eq!(args.to_target().expect("target").kind, TargetKind::Pi);
    }

    #[test]
    fn codex_only_options_are_reported_for_non_codex_targets() {
        let mut cli = base_cli();
        cli.profile = Some("fixit".to_string());
        cli.instructions = Some("base".to_string());

        assert_eq!(
            cli.codex_only_options_in_use(),
            vec!["--profile", "--instructions"]
        );
    }

    #[test]
    fn toggle_options_default_to_enabled_when_flags_are_present() {
        let mut cli = base_cli();
        cli.fork_from = Some("last".to_string());
        cli.compact = true;

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
    fn fork_default_off_renders_fork_toggle_disabled() {
        let mut cli = base_cli();
        cli.fork_from = Some("last".to_string());
        cli.fork_default_off = true;

        let options = cli.toggle_options();

        assert_eq!(
            options,
            vec![ToggleOption {
                label: "fork from: last".to_string(),
                argv: vec!["--fork-from".to_string(), "last".to_string()],
                enabled: false,
            }]
        );
        assert_eq!(enabled_option_argv(&options), Vec::<String>::new());
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
