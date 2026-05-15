use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ApplyPatchInput {
    pub patch: String,
    pub workdir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApplyPatchOutput {
    pub status: String,
    pub output: String,
}

pub fn apply(input: ApplyPatchInput, default_workdir: &str) -> ApplyPatchOutput {
    match apply_inner(
        &input.patch,
        input.workdir.as_deref().unwrap_or(default_workdir),
    ) {
        Ok(out) => ApplyPatchOutput {
            status: "completed".into(),
            output: out,
        },
        Err(err) => ApplyPatchOutput {
            status: "failed".into(),
            output: err,
        },
    }
}

fn apply_inner(patch: &str, workdir: &str) -> Result<String, String> {
    let mut lines: Vec<&str> = patch.lines().collect();
    if patch.ends_with('\n') {
        lines.push("");
    }
    if lines.first() != Some(&"*** Begin Patch") || !lines.iter().any(|l| *l == "*** End Patch") {
        return Err("patch must start with *** Begin Patch and end with *** End Patch".into());
    }
    let root = Path::new(workdir);
    let mut i = 1;
    let mut changed = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        if line == "*** End Patch" {
            return Ok(format!("Applied patch to {} file(s)", changed.len()));
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut content = String::new();
            while i < lines.len() && !lines[i].starts_with("*** ") {
                let Some(rest) = lines[i].strip_prefix('+') else {
                    return Err(format!("Invalid add-file line {}: expected +", i + 1));
                };
                content.push_str(rest);
                content.push('\n');
                i += 1;
            }
            let path = safe_join(root, path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, content).map_err(|e| e.to_string())?;
            changed.push(path);
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            let path = safe_join(root, path);
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            changed.push(path);
            i += 1;
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = None;
            if i < lines.len() {
                if let Some(dest) = lines[i].strip_prefix("*** Move to: ") {
                    move_to = Some(dest.to_string());
                    i += 1;
                }
            }
            if i < lines.len() && lines[i].starts_with("@@") {
                i += 1;
            }
            let path = safe_join(root, path);
            let old =
                fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let mut out = String::new();
            let old_lines: Vec<&str> = old.lines().collect();
            let mut pos = 0usize;
            while i < lines.len() && !lines[i].starts_with("*** ") {
                let marker = lines[i]
                    .chars()
                    .next()
                    .ok_or_else(|| format!("empty patch line {}", i + 1))?;
                let body = &lines[i][1..];
                match marker {
                    ' ' => {
                        while pos < old_lines.len() && old_lines[pos] != body {
                            out.push_str(old_lines[pos]);
                            out.push('\n');
                            pos += 1;
                        }
                        if pos >= old_lines.len() {
                            return Err(format!("missing context line: {body}"));
                        }
                        out.push_str(body);
                        out.push('\n');
                        pos += 1;
                    }
                    '-' => {
                        while pos < old_lines.len() && old_lines[pos] != body {
                            out.push_str(old_lines[pos]);
                            out.push('\n');
                            pos += 1;
                        }
                        if pos >= old_lines.len() {
                            return Err(format!("missing removal line: {body}"));
                        }
                        pos += 1;
                    }
                    '+' => {
                        out.push_str(body);
                        out.push('\n');
                    }
                    _ => return Err(format!("invalid update line {}", i + 1)),
                }
                i += 1;
            }
            while pos < old_lines.len() {
                out.push_str(old_lines[pos]);
                out.push('\n');
                pos += 1;
            }
            let target = move_to.map_or_else(|| path.clone(), |p| safe_join(root, &p));
            if target != path {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&target, out).map_err(|e| e.to_string())?;
            changed.push(target);
            continue;
        }
        return Err(format!("invalid patch header on line {}", i + 1));
    }
    Err("missing *** End Patch".into())
}

fn safe_join(root: &Path, path: &str) -> PathBuf {
    root.join(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_update() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello\n").unwrap();
        let out = apply(
            ApplyPatchInput {
                patch:
                    "*** Begin Patch\n*** Update File: a.txt\n@@\n-hello\n+world\n*** End Patch\n"
                        .into(),
                workdir: Some(tmp.path().display().to_string()),
            },
            "/tmp",
        );
        assert_eq!(out.status, "completed");
        assert_eq!(
            fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "world\n"
        );
    }
}
