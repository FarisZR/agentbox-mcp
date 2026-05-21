use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::config::{SkillsConfig, expand_tilde};

#[derive(Clone)]
pub struct SkillCatalog {
    config: SkillsConfig,
}

#[derive(Debug, Deserialize)]
pub struct ListSkillsInput {
    pub query: Option<String>,
    pub include_paths: Option<bool>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LoadSkillInput {
    pub skill: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction_file: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListSkillsOutput {
    pub skill_roots: Vec<String>,
    pub skills: Vec<SkillMeta>,
}

#[derive(Debug, Serialize)]
pub struct LoadSkillOutput {
    pub name: String,
    pub title: String,
    pub path: String,
    pub instruction_file: String,
    pub content: String,
}

impl SkillCatalog {
    pub fn new(config: SkillsConfig) -> Self {
        Self { config }
    }

    pub fn list(&self, input: ListSkillsInput) -> ListSkillsOutput {
        let include_paths = input.include_paths.unwrap_or(true);
        let query = input.query.unwrap_or_default().to_lowercase();
        let max = input.max_results.unwrap_or(100);
        let roots: Vec<String> = self.config.roots.iter().map(|r| expand_tilde(r)).collect();
        let mut skills = Vec::new();
        for root in &roots {
            let root_path = Path::new(root);
            if !root_path.exists() {
                continue;
            }
            for entry in WalkDir::new(root_path)
                .min_depth(1)
                .max_depth(3)
                .into_iter()
                .flatten()
            {
                if !entry.file_type().is_dir() {
                    continue;
                }
                let Some(file) = instruction_file(entry.path()) else {
                    continue;
                };
                if let Ok(content) = fs::read_to_string(&file) {
                    let mut meta = parse_skill(entry.path(), &file, &content);
                    if !include_paths {
                        meta.path = None;
                        meta.instruction_file = None;
                    }
                    let hay =
                        format!("{} {} {}", meta.name, meta.title, meta.description).to_lowercase();
                    if query.is_empty()
                        || hay.contains(&query)
                        || meta.tags.iter().any(|t| t.to_lowercase().contains(&query))
                    {
                        skills.push(meta);
                    }
                    if skills.len() >= max {
                        return ListSkillsOutput {
                            skill_roots: roots,
                            skills,
                        };
                    }
                }
            }
        }
        ListSkillsOutput {
            skill_roots: roots,
            skills,
        }
    }

    pub fn load(&self, input: LoadSkillInput) -> anyhow::Result<LoadSkillOutput> {
        let list = self.list(ListSkillsInput {
            query: None,
            include_paths: Some(true),
            max_results: Some(10000),
        });
        let selected = list
            .skills
            .into_iter()
            .find(|s| {
                s.name == input.skill
                    || s.path.as_deref() == Some(input.skill.as_str())
                    || s.instruction_file.as_deref() == Some(input.skill.as_str())
            })
            .ok_or_else(|| anyhow::anyhow!("skill not found: {}", input.skill))?;
        let path = selected.path.clone().unwrap_or_default();
        let file = selected.instruction_file.clone().unwrap_or_default();
        let content = fs::read_to_string(&file)?;
        Ok(LoadSkillOutput {
            name: selected.name,
            title: selected.title,
            path,
            instruction_file: file,
            content,
        })
    }
}

fn instruction_file(dir: &Path) -> Option<PathBuf> {
    ["SKILL.md", "skill.md", "README.md"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|p| p.exists())
}

fn parse_skill(path: &Path, instruction_file: &Path, content: &str) -> SkillMeta {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();
    let tags = parse_front_matter_tags(content);
    let mut title = name.clone();
    let mut description = String::new();
    let body = strip_front_matter(content);
    let mut after_title = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") && title == name {
            title = trimmed.trim_start_matches("# ").trim().to_string();
            after_title = true;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if !after_title && trimmed.starts_with("# ") {
            continue;
        }
        description = trimmed.to_string();
        break;
    }
    SkillMeta {
        name,
        title,
        description,
        tags,
        path: Some(path.display().to_string()),
        instruction_file: Some(instruction_file.display().to_string()),
    }
}

fn strip_front_matter(content: &str) -> &str {
    if let Some(rest) = content.strip_prefix("---\n")
        && let Some(idx) = rest.find("\n---\n")
    {
        return &rest[idx + 5..];
    }
    content
}

fn parse_front_matter_tags(content: &str) -> Vec<String> {
    let Some(rest) = content.strip_prefix("---\n") else {
        return vec![];
    };
    let Some(idx) = rest.find("\n---\n") else {
        return vec![];
    };
    rest[..idx]
        .lines()
        .find_map(|l| l.trim().strip_prefix("tags:"))
        .map(|raw| {
            raw.trim()
                .trim_matches(['[', ']'])
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_does_not_include_full_body_but_load_does() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rust-maintainer");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "# Rust maintainer\n\nShort desc.\n\nSECRET BODY",
        )
        .unwrap();
        let cat = SkillCatalog::new(SkillsConfig {
            enabled: true,
            roots: vec![tmp.path().display().to_string()],
        });
        let list = cat.list(ListSkillsInput {
            query: None,
            include_paths: Some(true),
            max_results: None,
        });
        assert_eq!(list.skills[0].title, "Rust maintainer");
        assert!(
            !serde_json::to_string(&list)
                .unwrap()
                .contains("SECRET BODY")
        );
        let loaded = cat
            .load(LoadSkillInput {
                skill: "rust-maintainer".into(),
            })
            .unwrap();
        assert!(loaded.content.contains("SECRET BODY"));
    }
}
