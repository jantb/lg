//! Turning a caller-supplied relative path into a directory a tool may run in.
//!
//! Two rules are enforced here, and nowhere else: a path may not leave the
//! project root, and it may not reach into terrarium's own `.terrarium` state.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::types::ToolCallError;

/// Resolves the working directory a tool's optional `path` argument selects.
pub(crate) fn resolve_cwd(
    project_root: &Path,
    path: Option<&str>,
) -> Result<PathBuf, ToolCallError> {
    let Some(rel) = path else {
        return Ok(project_root.to_path_buf());
    };
    resolve_existing_path(project_root, rel)
}

/// Like [`resolve_cwd`] for a path that must already exist.
///
/// The lexical path is returned rather than its canonicalization, so a
/// symlinked repository inside the project stays reachable under its link name.
pub(crate) fn resolve_existing_path(
    project_root: &Path,
    rel: &str,
) -> Result<PathBuf, ToolCallError> {
    let candidate = resolve_lexical_project_path(project_root, rel)?;
    let resolved = candidate
        .canonicalize()
        .map_err(|e| ToolCallError::InvalidParams(format!("path '{}' is not valid: {}", rel, e)))?;
    let root_canon = project_root.canonicalize().map_err(|e| {
        ToolCallError::Execution(anyhow::anyhow!("cannot resolve project root: {}", e))
    })?;
    if resolved.starts_with(&root_canon) {
        let relative = resolved.strip_prefix(&root_canon).unwrap_or(&resolved);
        if contains_hidden_terrarium_component(relative) {
            return Err(hidden_terrarium_path_error(rel));
        }
    }
    Ok(candidate)
}

fn resolve_lexical_project_path(project_root: &Path, rel: &str) -> Result<PathBuf, ToolCallError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(ToolCallError::InvalidParams(format!(
            "path '{}' must be relative to the project root",
            rel
        )));
    }
    if contains_hidden_terrarium_component(rel_path) {
        return Err(hidden_terrarium_path_error(rel));
    }

    let mut normalized = PathBuf::new();
    for component in rel_path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(ToolCallError::InvalidParams(format!(
                        "path '{}' escapes the project root",
                        rel
                    )));
                }
            }
            _ => {
                return Err(ToolCallError::InvalidParams(format!(
                    "path '{}' must be relative to the project root",
                    rel
                )));
            }
        }
    }

    Ok(project_root.join(normalized))
}

pub(crate) fn contains_hidden_terrarium_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(name) if name == ".terrarium"
        )
    })
}

pub(crate) fn hidden_terrarium_path_error(path: &str) -> ToolCallError {
    ToolCallError::InvalidParams(format!("path '{path}' is hidden"))
}

/// Drops the `project` argument, which selects a sub-project above this layer
/// and is an unknown field to the tool implementations below it.
pub(crate) fn strip_project(arguments: &Option<Value>) -> Option<Value> {
    arguments.as_ref().map(|args| {
        let mut a = args.clone();
        if let Some(obj) = a.as_object_mut() {
            obj.remove("project");
        }
        a
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(name)
    }

    #[test]
    fn resolve_cwd_rejects_hidden_terrarium_path() {
        let base = temp_dir("resolve_cwd_hidden_terrarium");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(".terrarium")).unwrap();

        let err = resolve_cwd(&base, Some(".terrarium")).unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("hidden")),
            "expected hidden-path InvalidParams, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_cwd_allows_symlinked_repo_under_project() {
        let base = temp_dir(&format!("resolve_cwd_linked_repo_{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        let linked_repo = base.join("linked-repo");
        let link = project.join("linked");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&linked_repo).unwrap();
        std::os::unix::fs::symlink(&linked_repo, &link).unwrap();

        let resolved = resolve_cwd(&project, Some("linked")).unwrap();

        assert_eq!(resolved, link);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_cwd_does_not_let_parent_dir_escape_through_symlink() {
        let base = temp_dir(&format!(
            "resolve_cwd_symlink_parent_{}",
            uuid::Uuid::new_v4()
        ));
        let project = base.join("project");
        let linked_repo = base.join("linked-repo");
        let sibling = base.join("sibling");
        let link = project.join("linked");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&linked_repo).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::os::unix::fs::symlink(&linked_repo, &link).unwrap();

        let err = resolve_cwd(&project, Some("linked/../sibling")).unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("not valid")),
            "expected lexical project path lookup to fail, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_existing_path_rejects_traversal() {
        let base = temp_dir("resolve_subpath_traversal");
        std::fs::create_dir_all(&base).unwrap();
        let err = resolve_existing_path(&base, "../../nonexistent_escape").unwrap_err();
        assert!(
            matches!(err, ToolCallError::InvalidParams(_)),
            "expected InvalidParams, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn strip_project_removes_field() {
        let args = Some(json!({"project": "foo", "path": "bar"}));
        let stripped = strip_project(&args);
        let obj = stripped.unwrap();
        assert_eq!(obj.get("path").unwrap(), "bar");
        assert!(obj.get("project").is_none());
    }

    #[test]
    fn strip_project_preserves_none() {
        assert!(strip_project(&None).is_none());
    }

    #[test]
    fn strip_project_handles_no_project_field() {
        let args = Some(json!({"path": "bar"}));
        let stripped = strip_project(&args);
        let obj = stripped.unwrap();
        assert_eq!(obj.get("path").unwrap(), "bar");
    }
}
