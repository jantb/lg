//! The index and the working tree: staging, discarding, committing.

use anyhow::{Context, Result};
use std::path::{Component, Path};

use super::{flow, repo_root, run};

pub fn stage(path: &str) -> Result<()> {
    run(&["add", "--", path]).map(|_| ())
}

pub fn unstage(path: &str) -> Result<()> {
    // `git reset -q HEAD -- <path>` works even pre-initial-commit (falls back
    // to `git rm --cached` semantics when there is no HEAD).
    let result = run(&["reset", "-q", "HEAD", "--", path]);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            // Pre-initial-commit: "HEAD" doesn't exist yet; use rm --cached.
            let msg = e.to_string();
            if msg.contains("unknown revision") || msg.contains("Failed to resolve") {
                run(&["rm", "--cached", "--", path]).map(|_| ())
            } else {
                Err(e)
            }
        }
    }
}

pub fn stage_all() -> Result<()> {
    run(&["add", "-A"]).map(|_| ())
}

pub fn unstage_all() -> Result<()> {
    let result = run(&["reset", "-q", "HEAD"]);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("unknown revision") || msg.contains("Failed to resolve") {
                // Nothing staged pre-initial-commit; treat as success.
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

fn ensure_safe_relative_path(path: &str) -> Result<&Path> {
    let rel = Path::new(path);
    if path.trim().is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        anyhow::bail!("refusing unsafe path: {path}");
    }
    Ok(rel)
}

fn restore_path_missing_from_head(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("did not match any file(s) known to git")
        || msg.contains("could not resolve HEAD")
        || msg.contains("unable to resolve HEAD")
        || msg.contains("Failed to resolve 'HEAD'")
}

pub fn rollback_worktree_path(path: &str) -> Result<()> {
    ensure_safe_relative_path(path)?;

    let restore_result = run(&["restore", "--staged", "--worktree", "--", path]);
    if let Err(err) = &restore_result {
        if restore_path_missing_from_head(err) {
            let _ = run(&["rm", "-r", "--cached", "--", path]);
        } else {
            return Err(anyhow::anyhow!("restore {path} failed: {err}"));
        }
    }

    run(&["clean", "-fd", "--", path]).map(|_| ())
}

pub fn delete_worktree_path(path: &str, is_dir: bool) -> Result<()> {
    let rel = ensure_safe_relative_path(path)?;

    let root = repo_root()?;
    let target = Path::new(&root).join(rel);
    if is_dir {
        std::fs::remove_dir_all(&target)
            .with_context(|| format!("delete directory {}", target.display()))?;
    } else {
        std::fs::remove_file(&target)
            .with_context(|| format!("delete file {}", target.display()))?;
    }
    Ok(())
}

pub fn commit(msg: &str) -> Result<String> {
    if msg.trim().is_empty() {
        anyhow::bail!("commit message must not be empty");
    }
    let release_update = flow::update_release_branch_from_main_before_commit()?;
    let out = run(&["commit", "-m", msg])?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if let Some(update) = release_update {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&update);
    }
    Ok(text)
}
