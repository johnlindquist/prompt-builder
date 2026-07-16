use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Pi,
    // Preserve legacy targets that omitted `kind`.
    #[default]
    Codex,
    Claude,
}

impl TargetKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub fn default_bin(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pi" => Ok(Self::Pi),
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::Claude),
            other => anyhow::bail!("unknown target kind {other:?}; expected pi, codex, or claude"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub name: String,
    #[serde(default)]
    pub kind: TargetKind,
    /// Executable override. Defaults to `pi`, `codex`, or `claude` by kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<String>,
    /// Environment variables set on launch, e.g. CODEX_HOME or CLAUDE_CONFIG_DIR.
    /// Values may start with `~/` to reference the home directory.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Codex profile passed as --profile. Codex targets only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Model passed as --model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Codex config overrides passed as -c KEY=VALUE. Codex targets only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config: Vec<String>,
    /// Extra argv inserted before the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl Target {
    pub fn bin(&self) -> &str {
        self.bin.as_deref().unwrap_or(self.kind.default_bin())
    }

    pub fn env_pairs(&self) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|(key, value)| (key.clone(), expand_home(value)))
            .collect()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetsFile {
    #[serde(default, rename = "targets")]
    targets: Vec<Target>,
}

pub fn default_targets() -> Vec<Target> {
    vec![
        Target {
            name: "pi".to_string(),
            kind: TargetKind::Pi,
            ..Target::default()
        },
        Target {
            name: "codex".to_string(),
            kind: TargetKind::Codex,
            ..Target::default()
        },
        Target {
            name: "claude".to_string(),
            kind: TargetKind::Claude,
            ..Target::default()
        },
    ]
}

pub fn targets_file_path() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".prompt-builder").join("targets.toml"))
        .context("could not determine home directory")
}

/// Targets from the targets file, or the built-in pi/codex/claude set when the
/// file does not exist.
pub fn load_targets() -> anyhow::Result<Vec<Target>> {
    load_targets_from(&targets_file_path()?)
}

pub fn load_targets_from(path: &Path) -> anyhow::Result<Vec<Target>> {
    if !path.exists() {
        return Ok(default_targets());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let targets = parse_targets(&text).with_context(|| format!("in {}", path.display()))?;
    if targets.is_empty() {
        return Ok(default_targets());
    }
    Ok(targets)
}

pub fn save_targets(targets: &[Target]) -> anyhow::Result<PathBuf> {
    let path = targets_file_path()?;
    validate_targets(targets)?;
    atomic_write(&path, &format_targets(targets)?)?;
    Ok(path)
}

pub fn format_targets(targets: &[Target]) -> anyhow::Result<String> {
    toml::to_string_pretty(&TargetsFile {
        targets: targets.to_vec(),
    })
    .context("failed to serialize targets")
}

pub fn parse_targets(text: &str) -> anyhow::Result<Vec<Target>> {
    let file: TargetsFile = toml::from_str(text).context("failed to parse targets TOML")?;
    validate_targets(&file.targets)?;
    Ok(file.targets)
}

/// Shared validation policy for every writer: the TUI editor, the CLI
/// `target` subcommands, and startup loading.
pub fn validate_targets(targets: &[Target]) -> anyhow::Result<()> {
    let mut seen = std::collections::HashSet::new();
    for target in targets {
        let name = target.name.as_str();
        if name.trim().is_empty() {
            anyhow::bail!("target with empty name");
        }
        if name != name.trim() {
            anyhow::bail!("target name {name:?} has leading or trailing whitespace");
        }
        if !seen.insert(name.to_string()) {
            anyhow::bail!("duplicate target name {name:?}");
        }
        if let Some(bin) = &target.bin {
            if bin.trim().is_empty() {
                anyhow::bail!("target {name:?} has an empty bin");
            }
            reject_nul(name, "bin", bin)?;
        }
        for (key, value) in &target.env {
            if key.trim().is_empty() {
                anyhow::bail!("target {name:?} has an empty env key");
            }
            if key.contains('=') {
                anyhow::bail!("target {name:?} env key {key:?} contains '='");
            }
            reject_nul(name, "env key", key)?;
            reject_nul(name, "env value", value)?;
        }
        if let Some(profile) = &target.profile {
            reject_nul(name, "profile", profile)?;
        }
        if let Some(model) = &target.model {
            reject_nul(name, "model", model)?;
        }
        for entry in &target.config {
            reject_nul(name, "config entry", entry)?;
        }
        for arg in &target.args {
            reject_nul(name, "arg", arg)?;
        }
    }
    Ok(())
}

fn reject_nul(target: &str, field: &str, value: &str) -> anyhow::Result<()> {
    if value.contains('\0') {
        anyhow::bail!("target {target:?} {field} contains a NUL byte");
    }
    Ok(())
}

/// Snapshot of the targets file taken before an external edit, used to seed
/// the editor and to detect concurrent changes on commit.
#[derive(Clone, Debug)]
pub struct TargetEditSnapshot {
    path: PathBuf,
    original_text: Option<String>,
    seed: String,
}

impl TargetEditSnapshot {
    pub fn seed(&self) -> &str {
        &self.seed
    }
}

pub fn begin_edit() -> anyhow::Result<TargetEditSnapshot> {
    begin_edit_at(&targets_file_path()?)
}

pub fn begin_edit_at(path: &Path) -> anyhow::Result<TargetEditSnapshot> {
    let original_text = read_optional_text(path)?;
    let seed = match &original_text {
        Some(text) => text.clone(),
        None => default_edit_document()?,
    };
    Ok(TargetEditSnapshot {
        path: path.to_path_buf(),
        original_text,
        seed,
    })
}

pub fn commit_edit(
    snapshot: &TargetEditSnapshot,
    edited_text: &str,
) -> anyhow::Result<Vec<Target>> {
    // Validate before touching the live file.
    let parsed = parse_targets(edited_text)?;
    if parsed.is_empty() {
        anyhow::bail!("targets file must contain at least one [[targets]] block");
    }

    // Do not silently overwrite a change made by another process/editor.
    let current_text = read_optional_text(&snapshot.path)?;
    if current_text != snapshot.original_text {
        anyhow::bail!(
            "{} changed on disk while it was being edited; reload and retry",
            snapshot.path.display()
        );
    }

    // Write the exact edited text so comments, ordering, and formatting survive.
    atomic_write(&snapshot.path, edited_text)?;
    Ok(parsed)
}

fn default_edit_document() -> anyhow::Result<String> {
    let formatted = format_targets(&default_targets())?;
    Ok(format!(
        "# Launch targets for prompt-builder.\n\
         # Fields: name, kind (pi|codex|claude), bin, model, args; Codex also supports\n\
         # profile and config. Environment variables belong under [targets.env].\n\n\
         {formatted}"
    ))
}

fn read_optional_text(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

/// Writes via a temp file in the destination directory plus rename, so a
/// crash mid-write cannot leave a truncated targets file.
fn atomic_write(path: &Path, text: &str) -> anyhow::Result<()> {
    let parent = path.parent().context("targets path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "targets.toml".to_string()),
        std::process::id()
    ));
    std::fs::write(&temp, text).with_context(|| format!("failed to write {}", temp.display()))?;
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            Err(err).with_context(|| format!("failed to replace {}", path.display()))
        }
    }
}

fn expand_home(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "prompt-builder-targets-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn missing_fields_default_and_kind_parses() {
        let targets = parse_targets(
            r#"
[[targets]]
name = "pi"
kind = "pi"

[[targets]]
name = "codex"

[[targets]]
name = "egghead"
kind = "codex"
env = { CODEX_HOME = "~/.codex-egghead" }
config = ['cli_auth_credentials_store="file"']

[[targets]]
name = "claude-second"
kind = "claude"
env = { CLAUDE_CONFIG_DIR = "~/.claude-second" }
"#,
        )
        .expect("targets should parse");

        assert_eq!(targets.len(), 4);
        assert_eq!(targets[0].kind, TargetKind::Pi);
        assert_eq!(targets[0].bin(), "pi");
        assert_eq!(targets[1].kind, TargetKind::Codex);
        assert_eq!(targets[1].bin(), "codex");
        assert_eq!(
            targets[2].config,
            vec!["cli_auth_credentials_store=\"file\"".to_string()]
        );
        assert_eq!(targets[3].kind, TargetKind::Claude);
        assert_eq!(targets[3].bin(), "claude");
    }

    #[test]
    fn env_pairs_expand_home_prefix() {
        let mut env = BTreeMap::new();
        env.insert("CODEX_HOME".to_string(), "~/.codex-egghead".to_string());
        env.insert("PLAIN".to_string(), "value".to_string());
        let target = Target {
            name: "egghead".to_string(),
            env,
            ..Target::default()
        };

        let pairs = target.env_pairs();
        let home = dirs::home_dir().expect("home dir");

        assert_eq!(
            pairs[0],
            (
                "CODEX_HOME".to_string(),
                home.join(".codex-egghead").to_string_lossy().into_owned()
            )
        );
        assert_eq!(pairs[1], ("PLAIN".to_string(), "value".to_string()));
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let err = parse_targets(
            r#"
[[targets]]
name = "codex"

[[targets]]
name = "codex"
"#,
        )
        .expect_err("duplicates should fail");

        assert!(err.to_string().contains("duplicate target name"));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = parse_targets(
            r#"
[[targets]]
name = "codex"
modle = "gpt-5.5"
"#,
        )
        .expect_err("typo field should fail");

        assert!(err.to_string().contains("parse"));
    }

    #[test]
    fn whitespace_padded_name_is_rejected() {
        let err =
            parse_targets("[[targets]]\nname = \" codex\"\n").expect_err("padded name should fail");

        assert!(err.to_string().contains("whitespace"));
    }

    #[test]
    fn blank_bin_and_bad_env_key_are_rejected() {
        let blank_bin = Target {
            name: "x".to_string(),
            bin: Some("  ".to_string()),
            ..Target::default()
        };
        assert!(validate_targets(&[blank_bin])
            .expect_err("blank bin should fail")
            .to_string()
            .contains("empty bin"));

        let mut env = BTreeMap::new();
        env.insert("BAD=KEY".to_string(), "value".to_string());
        let bad_env = Target {
            name: "x".to_string(),
            env,
            ..Target::default()
        };
        assert!(validate_targets(&[bad_env])
            .expect_err("env key with '=' should fail")
            .to_string()
            .contains("contains '='"));
    }

    #[test]
    fn targets_round_trip_through_toml() {
        let mut env = BTreeMap::new();
        env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            "~/.claude-second".to_string(),
        );
        let targets = vec![
            Target {
                name: "codex".to_string(),
                kind: TargetKind::Codex,
                ..Target::default()
            },
            Target {
                name: "claude-second".to_string(),
                kind: TargetKind::Claude,
                env,
                model: Some("opus".to_string()),
                args: vec!["--verbose".to_string()],
                ..Target::default()
            },
        ];

        let text = format_targets(&targets).expect("serialize");
        let parsed = parse_targets(&text).expect("parse");

        assert_eq!(parsed, targets);
    }

    #[test]
    fn default_targets_put_pi_first_and_cover_all_harnesses() {
        let defaults = default_targets();

        assert_eq!(defaults.len(), 3);
        assert_eq!(defaults[0].name, "pi");
        assert_eq!(defaults[0].kind, TargetKind::Pi);
        assert_eq!(defaults[1].name, "codex");
        assert_eq!(defaults[1].kind, TargetKind::Codex);
        assert_eq!(defaults[2].name, "claude");
        assert_eq!(defaults[2].kind, TargetKind::Claude);
    }

    #[test]
    fn missing_file_edit_seed_contains_default_targets() {
        let dir = temp_dir("seed");
        let path = dir.join("targets.toml");

        let snapshot = begin_edit_at(&path).expect("begin edit");

        assert!(snapshot.seed().starts_with("# Launch targets"));
        let pi = snapshot.seed().find("name = \"pi\"").expect("pi target");
        let codex = snapshot
            .seed()
            .find("name = \"codex\"")
            .expect("codex target");
        let claude = snapshot
            .seed()
            .find("name = \"claude\"")
            .expect("claude target");
        assert!(pi < codex && codex < claude);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_edit_commits_exact_text_and_preserves_comments() {
        let dir = temp_dir("commit");
        let path = dir.join("targets.toml");
        std::fs::write(&path, "[[targets]]\nname = \"old\"\n").expect("seed file");
        let snapshot = begin_edit_at(&path).expect("begin edit");

        let edited = "# keep this comment\n[[targets]]\nname = \"work\"\nkind = \"codex\"\nargs = [\"--verbose\"]\n\n[targets.env]\nCODEX_HOME = \"~/.codex-work\"\n";
        let parsed = commit_edit(&snapshot, edited).expect("commit");

        assert_eq!(std::fs::read_to_string(&path).expect("read back"), edited);
        assert_eq!(parsed[0].args, vec!["--verbose".to_string()]);
        // No leftover temp file from the atomic write.
        let leftovers = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_edit_does_not_replace_existing_file() {
        let dir = temp_dir("invalid");
        let path = dir.join("targets.toml");
        std::fs::write(&path, "[[targets]]\nname = \"old\"\n").expect("seed file");
        let before = std::fs::read_to_string(&path).expect("read");
        let snapshot = begin_edit_at(&path).expect("begin edit");

        let err = commit_edit(&snapshot, "[[targets]]\nname =").expect_err("invalid TOML");
        assert!(err.to_string().contains("parse"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), before);

        let err = commit_edit(&snapshot, "# nothing\n").expect_err("empty target list should fail");
        assert!(err.to_string().contains("at least one"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_file_change_refuses_commit() {
        let dir = temp_dir("conflict");
        let path = dir.join("targets.toml");
        std::fs::write(&path, "[[targets]]\nname = \"old\"\n").expect("seed file");
        let snapshot = begin_edit_at(&path).expect("begin edit");

        std::fs::write(&path, "[[targets]]\nname = \"raced\"\n").expect("concurrent write");

        let err = commit_edit(&snapshot, "[[targets]]\nname = \"mine\"\n")
            .expect_err("conflict should fail");
        assert!(err.to_string().contains("changed on disk"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "[[targets]]\nname = \"raced\"\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
