use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_LIMIT: usize = 20;
const FALLBACK_ENTRY_LIMIT: usize = 50_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtToken {
    pub query: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    pub path: String,
    pub score: i64,
}

/// Finds an active `@` token using a character-index cursor.
pub fn current_at_token(text: &str, cursor: usize, allow_empty: bool) -> Option<AtToken> {
    current_prefixed_token(text, cursor, '@', allow_empty)
}

/// Finds an active `$` token (skill mention) using a character-index cursor.
pub fn current_dollar_token(text: &str, cursor: usize, allow_empty: bool) -> Option<AtToken> {
    current_prefixed_token(text, cursor, '$', allow_empty)
}

pub fn load_file_list(cwd: &Path) -> Vec<String> {
    git_file_list(cwd).unwrap_or_else(|| fallback_file_list(cwd, FALLBACK_ENTRY_LIMIT))
}

pub fn search_files(query: &str, files: &[String]) -> Vec<FileMatch> {
    let mut matches = files
        .iter()
        .filter_map(|path| {
            score_path(query, path).map(|score| FileMatch {
                path: path.clone(),
                score,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| match b.score.cmp(&a.score) {
        Ordering::Equal => a.path.cmp(&b.path),
        other => other,
    });
    matches.truncate(DEFAULT_LIMIT);
    matches
}

pub fn quote_path_for_insert(path: &str) -> String {
    if path.chars().any(char::is_whitespace) && !path.contains('"') {
        format!("\"{path}\" ")
    } else {
        format!("{path} ")
    }
}

fn current_prefixed_token(
    text: &str,
    cursor: usize,
    prefix: char,
    allow_empty: bool,
) -> Option<AtToken> {
    let chars = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());

    let mut start = cursor;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }

    let mut end = cursor;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }

    if start == end {
        let mut right = cursor;
        while right < chars.len() && chars[right].is_whitespace() {
            right += 1;
        }
        let mut right_end = right;
        while right_end < chars.len() && !chars[right_end].is_whitespace() {
            right_end += 1;
        }
        if right < right_end && chars[right] == prefix {
            return token_from_range(&chars, right, right_end, prefix, allow_empty);
        }
        return None;
    }

    if chars[start] == prefix {
        return token_from_range(&chars, start, end, prefix, allow_empty);
    }

    if cursor < chars.len() && chars[cursor] == prefix {
        let prefix_starts_token = cursor == 0 || chars[cursor - 1].is_whitespace();
        if prefix_starts_token {
            let mut prefix_end = cursor + 1;
            while prefix_end < chars.len() && !chars[prefix_end].is_whitespace() {
                prefix_end += 1;
            }
            return token_from_range(&chars, cursor, prefix_end, prefix, allow_empty);
        }
    }

    None
}

fn token_from_range(
    chars: &[char],
    start: usize,
    end: usize,
    prefix: char,
    allow_empty: bool,
) -> Option<AtToken> {
    if start >= end || chars.get(start) != Some(&prefix) {
        return None;
    }
    let query = chars[start + 1..end].iter().collect::<String>();
    if query.is_empty() && !allow_empty {
        return None;
    }
    Some(AtToken { query, start, end })
}

fn git_file_list(cwd: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["ls-files", "-z", "-co", "--exclude-standard"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut seen = HashSet::new();
    let mut files = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| String::from_utf8_lossy(bytes).replace('\\', "/"))
        .filter(|path| seen.insert(path.clone()))
        .collect::<Vec<_>>();
    files.sort();
    Some(files)
}

fn fallback_file_list(cwd: &Path, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    walk_fallback(cwd, cwd, limit, &mut out);
    out.sort();
    out
}

fn walk_fallback(root: &Path, dir: &Path, limit: usize, out: &mut Vec<String>) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= limit {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_symlink() || should_skip_dir_name(&name) {
                continue;
            }
            walk_fallback(root, &path, limit, out);
        } else if file_type.is_file() {
            if let Some(relative) = relative_slash_path(root, &path) {
                out.push(relative);
            }
        }
    }
}

fn should_skip_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "node_modules"
            | "bower_components"
            | "jspm_packages"
            | "target"
            | "build"
            | "dist"
            | "out"
            | ".next"
            | ".nuxt"
            | ".turbo"
            | ".svelte-kit"
            | ".cache"
            | "coverage"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".mypy_cache"
            | ".pytest_cache"
            | "vendor"
    )
}

fn relative_slash_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(PathBuf::from)?
        .to_str()
        .map(|path| path.replace('\\', "/"))
}

fn score_path(query: &str, path: &str) -> Option<i64> {
    let query = query.trim();
    if query.is_empty() {
        return Some(1);
    }
    let query_lower = query.to_lowercase();
    let path_lower = path.to_lowercase();
    let matched = match_positions(&query_lower, &path_lower)?;
    let mut score = 1000i64;
    score -= matched.first().copied().unwrap_or_default() as i64;
    score -= path_lower.len() as i64 / 4;

    let name = path_lower.rsplit('/').next().unwrap_or(&path_lower);
    if name == query_lower {
        score += 500;
    } else if name.starts_with(&query_lower) {
        score += 250;
    } else if name.contains(&query_lower) {
        score += 120;
    }
    if path_lower.starts_with(&query_lower) {
        score += 160;
    } else if path_lower.contains(&query_lower) {
        score += 60;
    }
    score += contiguous_bonus(&matched);
    Some(score)
}

fn match_positions(query: &str, path: &str) -> Option<Vec<usize>> {
    let mut positions = Vec::new();
    let mut path_chars = path.chars().enumerate();
    for query_char in query.chars() {
        let (position, _) = path_chars.find(|(_, path_char)| *path_char == query_char)?;
        positions.push(position);
    }
    Some(positions)
}

fn contiguous_bonus(positions: &[usize]) -> i64 {
    if positions.len() < 2 {
        return 0;
    }
    let mut bonus = 0;
    let mut run = 1;
    for pair in positions.windows(2) {
        if pair[1] == pair[0] + 1 {
            run += 1;
            bonus += 15 * run;
        } else {
            run = 1;
        }
    }
    bonus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_at_token_matches_codex_cases() {
        let cases = vec![
            ("@hello", 3, Some("hello"), "basic"),
            ("@file.txt", 4, Some("file.txt"), "extension"),
            ("hello @world test", 8, Some("world"), "middle"),
            ("@test123", 5, Some("test123"), "numbers"),
            ("@İstanbul", 3, Some("İstanbul"), "turkish"),
            ("@testЙЦУ.rs", 8, Some("testЙЦУ.rs"), "mixed cyrillic"),
            ("@诶", 2, Some("诶"), "chinese"),
            ("@👍", 2, Some("👍"), "emoji"),
            ("hello", 2, None, "no at"),
            ("@", 1, None, "empty disallowed"),
            ("@ hello", 2, None, "space after at"),
            ("test @ world", 6, None, "space after token start"),
            ("aaa@aaa", 4, None, "mid-word at"),
            ("aaa @aaa", 5, Some("aaa"), "after space"),
            ("test @file.txt", 7, Some("file.txt"), "file after space"),
            ("test　@İstanbul", 8, Some("İstanbul"), "full-width space"),
            ("@ЙЦУ　@诶", 10, Some("诶"), "full-width between tokens"),
            ("test\t@file", 6, Some("file"), "tab boundary"),
            (
                "npx -y @kaeawc/auto-mobile@latest",
                20,
                Some("kaeawc/auto-mobile@latest"),
                "nested at package",
            ),
            (
                "@icons/icon@2x.png",
                15,
                Some("icons/icon@2x.png"),
                "nested at file",
            ),
            ("foo@bar", 7, None, "email-like mid-word"),
        ];
        for (text, cursor, expected, label) in cases {
            assert_eq!(
                current_at_token(text, cursor, false).map(|token| token.query),
                expected.map(str::to_string),
                "{label}"
            );
        }
    }

    #[test]
    fn current_dollar_token_finds_skill_mentions() {
        assert_eq!(
            current_dollar_token("say $fus", 8, false),
            Some(AtToken {
                query: "fus".to_string(),
                start: 4,
                end: 8,
            })
        );
        // `$` mid-word (prices, shell vars) is not a token.
        assert_eq!(current_dollar_token("cost 100$", 9, true), None);
        // `@` tokens are untouched by the dollar scanner and vice versa.
        assert_eq!(current_dollar_token("see @file", 9, true), None);
        assert_eq!(current_at_token("run $fus", 8, true), None);
    }

    #[test]
    fn current_at_token_allows_empty_when_requested() {
        assert_eq!(
            current_at_token("@", 1, true),
            Some(AtToken {
                query: String::new(),
                start: 0,
                end: 1
            })
        );
    }

    #[test]
    fn current_at_token_uses_character_cursor_after_non_ascii() {
        let text = "诶 @main";
        assert_eq!(
            current_at_token(text, 4, false),
            Some(AtToken {
                query: "main".to_string(),
                start: 2,
                end: 7,
            })
        );
    }

    #[test]
    fn search_prefers_basename_prefix_then_path_order() {
        let files = vec![
            "src/main.rs".to_string(),
            "README.md".to_string(),
            "src/domain/read_model.rs".to_string(),
        ];
        let matches = search_files("ma", &files);
        assert_eq!(matches[0].path, "src/main.rs");
    }

    #[test]
    fn quote_path_for_insert_quotes_whitespace_only_when_simple() {
        assert_eq!(quote_path_for_insert("src/main.rs"), "src/main.rs ");
        assert_eq!(
            quote_path_for_insert("docs/my file.md"),
            "\"docs/my file.md\" "
        );
        assert_eq!(
            quote_path_for_insert("docs/\"x\" file.md"),
            "docs/\"x\" file.md "
        );
    }

    #[test]
    fn fallback_walk_skips_generated_dirs_but_not_same_named_files() {
        let root =
            std::env::temp_dir().join(format!("prompt-builder-file-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("dist")).expect("create skipped dir");
        fs::create_dir_all(root.join("src")).expect("create src dir");
        fs::write(root.join("dist").join("ignored.rs"), "").expect("write ignored file");
        fs::write(root.join("src").join("main.rs"), "").expect("write src file");
        fs::write(root.join("target"), "").expect("write file with skipped dir name");

        let files = fallback_file_list(&root, FALLBACK_ENTRY_LIMIT);

        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(files.contains(&"target".to_string()));
        assert!(!files.contains(&"dist/ignored.rs".to_string()));
        let _ = fs::remove_dir_all(root);
    }
}
