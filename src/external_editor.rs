use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// An editor that exits faster than any human interaction almost certainly
/// forked off a GUI window and returned (e.g. `zed` or `code` without
/// `--wait`), so edits made there can never reach us.
const INSTANT_EXIT: Duration = Duration::from_millis(250);

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

    let started = Instant::now();
    let status = Command::new(&editor[0])
        .args(&editor[1..])
        .arg(&path)
        .status();
    let elapsed = started.elapsed();
    let result = match status {
        Ok(status) if status.success() => match fs::read_to_string(&path) {
            Ok(text) if elapsed < INSTANT_EXIT && text == seed => Err(anyhow::anyhow!(
                "{} returned immediately without changes; GUI editors need a wait flag, e.g. VISUAL=\"{} --wait\"",
                editor[0],
                editor[0],
            )),
            Ok(text) => Ok(text),
            Err(err) => Err(anyhow::Error::from(err)),
        },
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
    use std::os::unix::fs::PermissionsExt;

    fn write_editor_script(body: &str) -> PathBuf {
        let path = temp_edit_path("sh");
        fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod script");
        path
    }

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

        // `true` exits 0 instantly without touching the file — the signature
        // of a GUI editor missing its wait flag.
        env::set_var("VISUAL", "true");
        let err = edit_text("hello editor\n").expect_err("instant no-op edit is rejected");
        assert!(err.to_string().contains("--wait"), "got: {err}");

        // An instant exit that DID change the file is a real edit.
        let modify = write_editor_script("printf 'edited by script' > \"$1\"\n");
        env::set_var("VISUAL", modify.to_str().unwrap());
        let edited = edit_text("hello editor\n").expect("edit succeeds");
        assert_eq!(edited, "edited by script");

        // A slow exit with unchanged content is a deliberate no-op edit:
        // trimmed for markdown, byte-exact for TOML.
        let noop = write_editor_script("sleep 0.4\n");
        env::set_var("VISUAL", noop.to_str().unwrap());
        let edited = edit_text("hello editor\n").expect("edit succeeds");
        assert_eq!(edited, "hello editor");
        let toml = edit_toml("# comment\nname = \"x\"\n").expect("toml edit succeeds");
        assert_eq!(toml, "# comment\nname = \"x\"\n");
        let _ = fs::remove_file(&modify);
        let _ = fs::remove_file(&noop);

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
