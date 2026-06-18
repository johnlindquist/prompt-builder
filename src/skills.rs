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

pub fn load_skills(dirs: &[PathBuf]) -> Vec<Skill> {
    let mut seen = HashSet::new();
    let mut skills = Vec::new();
    for dir in dirs {
        walk(dir, 0, &mut |path| {
            let Ok(real) = path.canonicalize() else {
                return;
            };
            if !seen.insert(real) {
                return;
            }
            if let Ok(text) = fs::read_to_string(path) {
                skills.push(parse_skill(path, &text));
            }
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
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
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
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
    use super::*;

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
}
