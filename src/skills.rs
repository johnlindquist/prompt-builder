use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

impl Skill {
    pub fn mention(&self) -> String {
        format!("${}", self.name)
    }
}

pub fn load_skills(user_dirs: &[PathBuf], cwd: &Path) -> Vec<Skill> {
    let mut roots = project_skill_dirs(cwd);
    roots.extend(user_dirs.iter().cloned());
    load_skills_from_roots(&roots)
}

fn load_skills_from_roots(roots: &[PathBuf]) -> Vec<Skill> {
    let mut seen_paths = HashSet::new();
    let mut skills_by_name = BTreeMap::new();

    for root in roots {
        let mut candidates = Vec::new();
        walk(root, 0, &mut |path| candidates.push(path.to_path_buf()));
        candidates.sort();

        for path in candidates {
            let Ok(real_path) = path.canonicalize() else {
                continue;
            };
            if !seen_paths.insert(real_path) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let skill = parse_skill(&path, &text);
            skills_by_name.entry(skill.name.clone()).or_insert(skill);
        }
    }

    skills_by_name.into_values().collect()
}

fn project_skill_dirs(cwd: &Path) -> Vec<PathBuf> {
    let Some(cwd) = canonical_directory(cwd) else {
        return Vec::new();
    };
    let project_root = cwd
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(cwd.as_path());
    let mut dirs = Vec::new();

    for ancestor in cwd.ancestors() {
        dirs.push(ancestor.join(".agents").join("skills"));
        if ancestor == project_root {
            break;
        }
    }

    dirs
}

fn canonical_directory(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let canonical = absolute.canonicalize().ok()?;
    canonical.is_dir().then_some(canonical)
}

fn walk(dir: &Path, depth: usize, on_skill: &mut impl FnMut(&Path)) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            walk(&path, depth + 1, on_skill);
        } else if file_type.is_file()
            && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        {
            on_skill(&path);
        }
    }
}

fn parse_skill(path: &Path, text: &str) -> Skill {
    let folder_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let mut name = String::new();
    let mut description = String::new();

    if let Some(frontmatter) = frontmatter(text) {
        for line in frontmatter.lines() {
            if let Some(value) = line.strip_prefix("name:") {
                name = clean_yaml_scalar(value);
            } else if let Some(value) = line.strip_prefix("description:") {
                description = clean_yaml_scalar(value);
            }
        }
    }

    if name.is_empty() {
        name = folder_name.to_string();
    }
    if description.is_empty() {
        if let Some(heading) = text.lines().find_map(|line| line.strip_prefix("# ")) {
            description = heading.trim().to_string();
        }
    }

    Skill {
        name,
        description,
        path: path.to_path_buf(),
    }
}

fn frontmatter(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let end = rest.find("\n---").or_else(|| rest.find("\r\n---"))?;
    Some(&rest[..end])
}

fn clean_yaml_scalar(value: &str) -> String {
    value
        .trim()
        .trim_start_matches(">-")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use super::*;

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("prompt-builder-skills-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).expect("create temp directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_skill(root: &Path, folder: &str, name: &str, description: &str) -> PathBuf {
        let path = root.join(folder).join("SKILL.md");
        fs::create_dir_all(path.parent().expect("skill parent")).expect("create skill directory");
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n"),
        )
        .expect("write skill");
        path
    }

    #[test]
    fn parses_frontmatter_name_and_description() {
        let skill = parse_skill(
            Path::new("/tmp/fusion/SKILL.md"),
            "---\nname: fusion\ndescription: Run Fusion\n---\n# Fallback\n",
        );

        assert_eq!(
            skill,
            Skill {
                name: "fusion".to_string(),
                description: "Run Fusion".to_string(),
                path: PathBuf::from("/tmp/fusion/SKILL.md"),
            }
        );
    }

    #[test]
    fn parses_crlf_frontmatter_name_and_description() {
        let skill = parse_skill(
            Path::new("/tmp/fusion/SKILL.md"),
            "---\r\nname: fusion\r\ndescription: Run Fusion\r\n---\r\n# Fallback\r\n",
        );

        assert_eq!(skill.name, "fusion");
        assert_eq!(skill.description, "Run Fusion");
    }

    #[test]
    fn loads_user_and_project_skills_in_name_order() {
        let temp = TestDir::new();
        let repo = temp.path().join("repo");
        let cwd = repo.join("src");
        let user_root = temp.path().join("user-skills");
        fs::create_dir_all(repo.join(".git")).expect("create git marker");
        fs::create_dir_all(&cwd).expect("create cwd");
        write_skill(&user_root, "zebra", "zebra", "User skill");
        write_skill(
            &repo.join(".agents/skills"),
            "alpha",
            "alpha",
            "Project skill",
        );

        let skills = load_skills(&[user_root], &cwd);

        assert_eq!(
            skills
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zebra"]
        );
    }

    #[test]
    fn nearest_project_skill_wins_over_parent_and_user_duplicate() {
        let temp = TestDir::new();
        let repo = temp.path().join("repo");
        let service = repo.join("service");
        let cwd = service.join("src");
        let user_root = temp.path().join("user-skills");
        fs::create_dir_all(repo.join(".git")).expect("create git marker");
        fs::create_dir_all(&cwd).expect("create cwd");
        write_skill(&user_root, "shared", "shared", "user");
        write_skill(&repo.join(".agents/skills"), "shared", "shared", "repo");
        let service_path = write_skill(
            &service.join(".agents/skills"),
            "shared",
            "shared",
            "service",
        );

        let skills = load_skills(&[user_root], &cwd);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "service");
        assert_eq!(
            skills[0].path,
            service_path.canonicalize().expect("canonical skill path")
        );
    }

    #[test]
    fn same_name_within_one_root_uses_lexicographically_first_path() {
        let temp = TestDir::new();
        let cwd = temp.path().join("cwd");
        let user_root = temp.path().join("user-skills");
        fs::create_dir_all(&cwd).expect("create cwd");
        let first = write_skill(&user_root, "a", "duplicate", "first");
        write_skill(&user_root, "b", "duplicate", "second");

        let skills = load_skills(&[user_root], &cwd);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].path, first);
    }

    #[test]
    fn overlapping_roots_do_not_load_the_same_file_twice() {
        let temp = TestDir::new();
        let cwd = temp.path().join("cwd");
        let user_root = temp.path().join("user-skills");
        fs::create_dir_all(&cwd).expect("create cwd");
        write_skill(&user_root, "global", "global", "Global skill");

        let skills = load_skills(&[user_root.clone(), user_root.join(".")], &cwd);

        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn project_discovery_stops_at_nearest_git_root() {
        let temp = TestDir::new();
        let outer = temp.path().join("outer");
        let repo = outer.join("repo");
        let cwd = repo.join("sub");
        fs::create_dir_all(&cwd).expect("create cwd");
        fs::write(repo.join(".git"), "gitdir: elsewhere").expect("create git file");
        write_skill(
            &outer.join(".agents/skills"),
            "outside",
            "outside",
            "Outside repo",
        );
        write_skill(
            &repo.join(".agents/skills"),
            "inside",
            "inside",
            "Inside repo",
        );

        let skills = load_skills(&[], &cwd);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "inside");
    }

    #[test]
    fn non_git_cwd_does_not_scan_parent_skill_dirs() {
        let temp = TestDir::new();
        let workspace = temp.path().join("workspace");
        let cwd = workspace.join("project");
        fs::create_dir_all(&cwd).expect("create cwd");
        write_skill(
            &workspace.join(".agents/skills"),
            "parent",
            "parent",
            "Parent skill",
        );
        write_skill(&cwd.join(".agents/skills"), "local", "local", "Local skill");

        let skills = load_skills(&[], &cwd);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "local");
    }

    #[test]
    fn invalid_cwd_still_loads_user_skills() {
        let temp = TestDir::new();
        let user_root = temp.path().join("user-skills");
        write_skill(&user_root, "global", "global", "Global skill");

        let skills = load_skills(&[user_root], &temp.path().join("missing"));

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "global");
    }

    #[test]
    fn unreadable_utf8_skill_is_skipped_without_hiding_valid_skills() {
        let temp = TestDir::new();
        let cwd = temp.path().join("cwd");
        let user_root = temp.path().join("user-skills");
        fs::create_dir_all(&cwd).expect("create cwd");
        write_skill(&user_root, "valid", "valid", "Valid skill");
        let invalid = user_root.join("invalid/SKILL.md");
        fs::create_dir_all(invalid.parent().expect("invalid skill parent"))
            .expect("create invalid skill directory");
        fs::write(invalid, [0xff, 0xfe]).expect("write invalid UTF-8");

        let skills = load_skills(&[user_root], &cwd);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "valid");
    }
}
