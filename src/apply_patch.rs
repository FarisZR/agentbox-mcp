use std::path::Path;

use codex_apply_patch::{self as codex_patch, AppliedPatchFileChange};
use codex_exec_server::LOCAL_FS;
use codex_utils_absolute_path::AbsolutePathBuf;
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

pub async fn apply(input: ApplyPatchInput, default_workdir: &str) -> ApplyPatchOutput {
    match apply_inner(
        &input.patch,
        input.workdir.as_deref().unwrap_or(default_workdir),
    )
    .await
    {
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

async fn apply_inner(patch: &str, workdir: &str) -> Result<String, String> {
    let cwd = absolute_workdir(workdir)?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    match codex_patch::apply_patch(
        patch,
        &cwd,
        &mut stdout,
        &mut stderr,
        LOCAL_FS.as_ref(),
        None,
    )
    .await
    {
        Ok(delta) => {
            let mut output = String::new();
            output.push_str(&String::from_utf8_lossy(&stdout));
            output.push_str(&String::from_utf8_lossy(&stderr));
            if !delta.is_empty() {
                output.push_str(&format_delta(&delta));
            }
            Ok(output)
        }
        Err(err) => {
            let mut output = String::new();
            output.push_str(&String::from_utf8_lossy(&stdout));
            output.push_str(&String::from_utf8_lossy(&stderr));
            if output.trim().is_empty() {
                output = err.to_string();
            }
            Err(output)
        }
    }
}

fn absolute_workdir(workdir: &str) -> Result<AbsolutePathBuf, String> {
    let path = Path::new(workdir);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("resolve current directory: {e}"))?
            .join(path)
    };
    AbsolutePathBuf::from_absolute_path(&absolute)
        .map_err(|e| format!("invalid absolute workdir {}: {e}", absolute.display()))
}

fn format_delta(delta: &codex_patch::AppliedPatchDelta) -> String {
    let mut out = String::new();
    for change in delta.changes() {
        let kind = match &change.change {
            AppliedPatchFileChange::Add { .. } => "added",
            AppliedPatchFileChange::Delete { .. } => "deleted",
            AppliedPatchFileChange::Update {
                move_path: Some(_), ..
            } => "moved",
            AppliedPatchFileChange::Update { .. } => "updated",
        };
        out.push_str(&format!("{kind}: {}\n", change.path.display()));
    }
    if !delta.is_exact() {
        out.push_str(
            "warning: patch delta may be inexact due to filesystem errors or special files\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn applies_update_with_codex_patcher() {
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
        )
        .await;
        assert_eq!(out.status, "completed");
        assert_eq!(
            fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "world\n"
        );
    }

    #[tokio::test]
    async fn codex_patcher_supports_multiple_chunks_and_move() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let out = apply(
            ApplyPatchInput {
                patch: "*** Begin Patch\n*** Update File: a.txt\n*** Move to: b.txt\n@@\n-one\n+ONE\n@@\n-three\n+THREE\n*** End Patch\n"
                    .into(),
                workdir: Some(tmp.path().display().to_string()),
            },
            "/tmp",
        )
        .await;
        assert_eq!(out.status, "completed", "{}", out.output);
        assert!(!tmp.path().join("a.txt").exists());
        assert_eq!(
            fs::read_to_string(tmp.path().join("b.txt")).unwrap(),
            "ONE\ntwo\nTHREE\n"
        );
    }
}
