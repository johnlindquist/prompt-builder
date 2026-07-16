use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Resolves the external editor command from `$VISUAL` then `$EDITOR`,
/// splitting on whitespace so values like `code --wait` work.
pub fn resolve_editor_command() -> anyhow::Result<Vec<String>> {
    for var in ["VISUAL", "EDITOR"] {
        let Ok(value) = env::var(var) else {
            continue;
        };
        let parts = value
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Ok(parts);
        }
    }
    anyhow::bail!("set $VISUAL or $EDITOR to edit in an external editor")
}

/// Edits `seed` as a temp markdown file and returns the contents with
/// trailing whitespace trimmed. The caller is responsible for
/// suspending/restoring the TUI around this call.
pub fn edit_text(seed: &str) -> anyhow::Result<String> {
    edit_temp(seed, "md").map(|text| text.trim_end().to_string())
}

/// Edits `seed` as a temp TOML file and returns the exact edited contents,
/// including comments and the final newline.
pub fn edit_toml(seed: &str) -> anyhow::Result<String> {
    edit_temp(seed, "toml")
}

fn edit_temp(seed: &str, extension: &str) -> anyhow::Result<String> {
    let editor = resolve_editor_command()?;
    let path = temp_edit_path(extension);
    fs::write(&path, seed)?;

    let status = Command::new(&editor[0])
        .args(&editor[1..])
        .arg(&path)
        .status();
    let result = match status {
        Ok(status) if status.success() => fs::read_to_string(&path).map_err(anyhow::Error::from),
        Ok(status) => Err(anyhow::anyhow!("editor exited with {status}")),
        Err(err) => Err(anyhow::anyhow!("failed to launch {}: {err}", editor[0])),
    };
    let _ = fs::remove_file(&path);
    result
}

fn temp_edit_path(extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!(
        "prompt-builder-edit-{}-{nanos}.{extension}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Single test so VISUAL/EDITOR mutation cannot race across test threads.
    #[test]
    fn editor_resolution_and_round_trip() {
        let prev_visual = env::var("VISUAL").ok();
        let prev_editor = env::var("EDITOR").ok();

        env::set_var("VISUAL", "code --wait");
        assert_eq!(
            resolve_editor_command().expect("editor resolves"),
            vec!["code".to_string(), "--wait".to_string()]
        );

        // `true` exits 0 and leaves the seeded file untouched, so the seed
        // round-trips: trimmed for markdown, byte-exact for TOML.
        env::set_var("VISUAL", "true");
        let edited = edit_text("hello editor\n").expect("edit succeeds");
        assert_eq!(edited, "hello editor");
        let toml = edit_toml("# comment\nname = \"x\"\n").expect("toml edit succeeds");
        assert_eq!(toml, "# comment\nname = \"x\"\n");

        match prev_visual {
            Some(value) => env::set_var("VISUAL", value),
            None => env::remove_var("VISUAL"),
        }
        match prev_editor {
            Some(value) => env::set_var("EDITOR", value),
            None => env::remove_var("EDITOR"),
        }
    }
}
