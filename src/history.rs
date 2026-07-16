use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Cross-session prompt history.
///
/// Reads Codex's `~/.codex/history.jsonl` (entries Codex records for every
/// submitted message) merged with prompt-builder's own
/// `~/.prompt-builder/history.jsonl`, and appends new submissions to the
/// latter. Loading is deferred until the first navigation keypress so startup
/// stays instant.
pub struct History {
    own_path: Option<PathBuf>,
    extra_paths: Vec<PathBuf>,
    entries: Option<Vec<String>>,
    cursor: Option<usize>,
    draft: Option<String>,
}

impl History {
    pub fn new(own_path: Option<PathBuf>, extra_paths: Vec<PathBuf>) -> Self {
        Self {
            own_path,
            extra_paths,
            entries: None,
            cursor: None,
            draft: None,
        }
    }

    pub fn default_paths() -> Self {
        let home = dirs::home_dir();
        let own_path = home
            .as_deref()
            .map(|home| home.join(".prompt-builder").join("history.jsonl"));
        let extra_paths = home
            .as_deref()
            .map(|home| vec![home.join(".codex").join("history.jsonl")])
            .unwrap_or_default();
        Self::new(own_path, extra_paths)
    }

    pub fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }

    /// Steps to the previous (older) entry. `current_text` is preserved as the
    /// draft when browsing starts. Returns the text to show, or `None` when
    /// there is nothing older.
    pub fn navigate_up(&mut self, current_text: &str) -> Option<String> {
        self.ensure_loaded();
        let entries = self.entries.as_ref()?;
        if entries.is_empty() {
            return None;
        }

        let next_index = match self.cursor {
            None => {
                self.draft = Some(current_text.to_string());
                entries.len().checked_sub(1)?
            }
            Some(0) => return None,
            Some(index) => index - 1,
        };

        self.cursor = Some(next_index);
        entries.get(next_index).cloned()
    }

    /// Steps to the next (newer) entry, restoring the draft when stepping past
    /// the newest entry. Returns `None` when not browsing.
    pub fn navigate_down(&mut self) -> Option<String> {
        let entries = self.entries.as_ref()?;
        let index = self.cursor?;
        if index + 1 < entries.len() {
            self.cursor = Some(index + 1);
            entries.get(index + 1).cloned()
        } else {
            self.cursor = None;
            Some(self.draft.take().unwrap_or_default())
        }
    }

    /// Leaves browsing mode without restoring the draft; the currently shown
    /// text becomes the new draft (matches Codex: editing a recalled entry
    /// detaches it from history).
    pub fn stop_browsing(&mut self) {
        self.cursor = None;
        self.draft = None;
    }

    /// Records a submitted prompt to the persistent history file and the
    /// in-memory list.
    pub fn record(&mut self, text: &str) {
        let text = text.trim_end_matches('\n');
        if text.trim().is_empty() {
            return;
        }
        if let Some(entries) = self.entries.as_mut() {
            if entries.last().map(String::as_str) != Some(text) {
                entries.push(text.to_string());
            }
        }
        self.cursor = None;
        self.draft = None;

        let Some(path) = &self.own_path else {
            return;
        };
        let _ = append_entry(path, text);
    }

    /// Finds the newest entry strictly older than `before` whose text
    /// contains `query` case-insensitively. `before = None` starts from the
    /// newest entry. An empty query matches everything.
    pub fn search_older(&mut self, query: &str, before: Option<usize>) -> Option<(usize, String)> {
        self.ensure_loaded();
        let entries = self.entries.as_ref()?;
        let query = query.to_lowercase();
        let end = before.unwrap_or(entries.len()).min(entries.len());
        entries[..end]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, entry)| entry.to_lowercase().contains(&query))
            .map(|(index, entry)| (index, entry.clone()))
    }

    fn ensure_loaded(&mut self) {
        if self.entries.is_some() {
            return;
        }
        let mut stamped = Vec::new();
        for path in self
            .extra_paths
            .iter()
            .chain(self.own_path.iter())
            .cloned()
            .collect::<Vec<_>>()
        {
            stamped.extend(read_entries(&path));
        }
        stamped.sort_by_key(|(ts, _)| *ts);
        let mut entries: Vec<String> = Vec::with_capacity(stamped.len());
        for (_, text) in stamped {
            if entries.last() != Some(&text) {
                entries.push(text);
            }
        }
        self.entries = Some(entries);
    }
}

fn read_entries(path: &Path) -> Vec<(u64, String)> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            let text = value.get("text")?.as_str()?;
            if text.trim().is_empty() {
                return None;
            }
            let ts = value.get("ts").and_then(serde_json::Value::as_u64)?;
            Some((ts, text.to_string()))
        })
        .collect()
}

fn append_entry(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::json!({ "ts": ts, "text": text }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "prompt-builder-history-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_history(path: &Path, entries: &[(u64, &str)]) {
        let lines = entries
            .iter()
            .map(|(ts, text)| serde_json::json!({ "ts": ts, "text": text }).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, lines).expect("write history fixture");
    }

    #[test]
    fn up_recalls_newest_entry_first_and_preserves_draft() {
        let dir = temp_dir("up");
        let codex = dir.join("codex.jsonl");
        write_history(&codex, &[(1, "first"), (2, "second")]);
        let mut history = History::new(None, vec![codex]);

        assert_eq!(history.navigate_up("draft"), Some("second".to_string()));
        assert_eq!(history.navigate_up("ignored"), Some("first".to_string()));
        assert_eq!(history.navigate_up("ignored"), None);
        assert_eq!(history.navigate_down(), Some("second".to_string()));
        assert_eq!(history.navigate_down(), Some("draft".to_string()));
        assert!(!history.is_browsing());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merges_sources_by_timestamp_and_dedupes_neighbors() {
        let dir = temp_dir("merge");
        let codex = dir.join("codex.jsonl");
        let own = dir.join("own.jsonl");
        write_history(&codex, &[(1, "a"), (3, "b")]);
        write_history(&own, &[(2, "b"), (4, "c")]);
        let mut history = History::new(Some(own), vec![codex]);

        assert_eq!(history.navigate_up(""), Some("c".to_string()));
        assert_eq!(history.navigate_up(""), Some("b".to_string()));
        // "b" at ts=2 and ts=3 collapse into one entry.
        assert_eq!(history.navigate_up(""), Some("a".to_string()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn record_appends_jsonl_and_resets_browsing() {
        let dir = temp_dir("record");
        let own = dir.join("nested").join("own.jsonl");
        let mut history = History::new(Some(own.clone()), Vec::new());

        history.record("hello world");

        let raw = fs::read_to_string(&own).expect("history file written");
        let value: serde_json::Value = serde_json::from_str(raw.trim()).expect("valid json line");
        assert_eq!(value["text"], "hello world");
        assert!(value["ts"].as_u64().is_some());

        let mut fresh = History::new(Some(own), Vec::new());
        assert_eq!(fresh.navigate_up(""), Some("hello world".to_string()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn blank_and_duplicate_records_are_skipped() {
        let dir = temp_dir("dedupe");
        let own = dir.join("own.jsonl");
        let mut history = History::new(Some(own.clone()), Vec::new());

        history.record("  \n");
        history.record("same");
        assert_eq!(history.navigate_up(""), Some("same".to_string()));
        history.stop_browsing();
        history.record("same");

        let raw = fs::read_to_string(&own).expect("history file written");
        // The blank entry is dropped; the duplicate is still persisted to disk
        // but collapses during load.
        assert_eq!(raw.lines().count(), 2);
        let mut fresh = History::new(Some(own), Vec::new());
        assert_eq!(fresh.navigate_up(""), Some("same".to_string()));
        assert_eq!(fresh.navigate_up(""), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        let dir = temp_dir("malformed");
        let codex = dir.join("codex.jsonl");
        fs::write(&codex, "not json\n{\"ts\":1,\"text\":\"ok\"}\n{\"ts\":2}\n")
            .expect("write fixture");
        let mut history = History::new(None, vec![codex]);

        assert_eq!(history.navigate_up(""), Some("ok".to_string()));
        assert_eq!(history.navigate_up(""), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn search_older_matches_case_insensitively_from_newest() {
        let dir = temp_dir("search");
        let codex = dir.join("codex.jsonl");
        write_history(
            &codex,
            &[
                (1, "Fix the parser"),
                (2, "add tests"),
                (3, "fix focus bug"),
            ],
        );
        let mut history = History::new(None, vec![codex]);

        assert_eq!(
            history.search_older("FIX", None),
            Some((2, "fix focus bug".to_string()))
        );
        assert_eq!(
            history.search_older("fix", Some(2)),
            Some((0, "Fix the parser".to_string()))
        );
        assert_eq!(history.search_older("fix", Some(0)), None);
        assert_eq!(history.search_older("missing", None), None);
        // Empty query matches the newest entry (bash-style).
        assert_eq!(
            history.search_older("", None),
            Some((2, "fix focus bug".to_string()))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn navigate_down_without_browsing_is_none() {
        let mut history = History::new(None, Vec::new());
        assert_eq!(history.navigate_down(), None);
        assert_eq!(history.navigate_up("draft"), None);
    }
}
