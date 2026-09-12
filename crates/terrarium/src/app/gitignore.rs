//! Adding and removing the `.gitignore` lines terrarium owns.
//!
//! Every file terrarium writes into a project is machine-local, so each writer
//! ignores what it wrote. Entries are matched anchored (`/x`) or not (`x`), since
//! a user may have added the same path in either form.

use std::path::Path;

use crate::error::Result;

pub(super) fn ensure_project_gitignore_entries(
    project_root: &Path,
    entries: &[&str],
) -> Result<()> {
    let path = project_root.join(".gitignore");
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    let mut updated = existing.clone();
    for entry in entries {
        if gitignore_contains_entry(&existing, entry) || gitignore_contains_entry(&updated, entry) {
            continue;
        }
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(entry);
        updated.push('\n');
    }

    if updated != existing {
        std::fs::write(path, updated)?;
    }

    Ok(())
}

/// Removes gitignore lines terrarium added. Returns true when the file changed.
pub(super) fn remove_project_gitignore_entries(
    project_root: &Path,
    entries: &[&str],
) -> Result<bool> {
    let path = project_root.join(".gitignore");
    if !path.exists() {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(&path)?;
    let kept: Vec<&str> = existing
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !entries.iter().any(|entry| {
                let unanchored = entry.strip_prefix('/').unwrap_or(entry);
                trimmed == *entry || trimmed == unanchored
            })
        })
        .collect();
    if kept.len() == existing.lines().count() {
        return Ok(false);
    }
    // A file left holding nothing but blank lines was terrarium's alone.
    if kept.iter().all(|line| line.trim().is_empty()) {
        std::fs::remove_file(&path)?;
        return Ok(true);
    }
    let mut updated = kept.join("\n");
    updated.push('\n');
    std::fs::write(&path, updated)?;
    Ok(true)
}

fn gitignore_contains_entry(content: &str, entry: &str) -> bool {
    let unanchored = entry.strip_prefix('/').unwrap_or(entry);
    content.lines().any(|line| {
        let line = line.trim();
        line == entry || line == unanchored
    })
}
