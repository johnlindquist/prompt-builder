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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub codex_bin: String,
    pub cwd: PathBuf,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub config: Vec<String>,
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
            stdin: false,
            submit: false,
            print_prompt: false,
            print_command: false,
            dry_run: false,
            cwd: PathBuf::from("."),
            profile: None,
            model: None,
            config: Vec::new(),
            instructions: Some("base\nrules".to_string()),
            developer_instructions: Some("debug \"carefully\"".to_string()),
            skills_dirs: Vec::new(),
            codex_bin: "codex".to_string(),
        };

        assert_eq!(
            cli.launch_config().config,
            vec![
                "instructions=\"base\\nrules\"".to_string(),
                "developer_instructions=\"debug \\\"carefully\\\"\"".to_string(),
            ]
        );
    }
}
