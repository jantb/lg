//! Rules for the alternate paths a project root is reachable through.
//!
//! Seatbelt matches paths lexically, so a project entered through a symlink
//! (`terrarium run ~/dev/link` where `link` points elsewhere) needs rules for the
//! link's own path as well as its target — otherwise every access through the
//! link name is denied.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::project::{PathType, Rule, RuleAction, RuleSource};

pub(super) fn project_root_alias_rules(
    aliases: &[PathBuf],
    params: &HashMap<String, String>,
) -> Vec<Rule> {
    let Some(project_root) = params.get("PROJECT_ROOT") else {
        return Vec::new();
    };
    let Ok(canonical_project_root) = PathBuf::from(project_root).canonicalize() else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    let mut seen = HashSet::new();
    for alias in aliases {
        let Some(alias) = clean_absolute_path(alias) else {
            continue;
        };
        let Ok(canonical_alias) = alias.canonicalize() else {
            continue;
        };
        // Only aliases that really point at this root, and only when they differ
        // from it — the root itself already has rules.
        if canonical_alias != canonical_project_root || alias == canonical_project_root {
            continue;
        }
        let alias_value = alias.display().to_string();
        if !seen.insert(alias_value.clone()) {
            continue;
        }
        rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read* file-write*".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some(alias_value.clone()),
            comment: Some("Project root symlink alias".to_string()),
            source: RuleSource::Preset,
        });
        rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read-metadata file-test-existence".to_string(),
            path_type: Some(PathType::Ancestors),
            path_value: Some(alias_value),
            comment: Some("Resolve project root symlink alias ancestors".to_string()),
            source: RuleSource::Preset,
        });
    }
    rules
}

/// The `.terrarium` deny rule, repeated for each alias path: hiding it under the
/// canonical root does not hide it under the link name.
pub(super) fn hidden_project_state_alias_rules(
    aliases: &[PathBuf],
    params: &HashMap<String, String>,
) -> Vec<Rule> {
    project_root_alias_rules(aliases, params)
        .into_iter()
        .filter(|rule| {
            rule.operation == "file-read* file-write*"
                && matches!(rule.path_type, Some(PathType::Subpath))
        })
        .filter_map(|rule| {
            let alias = PathBuf::from(rule.path_value?);
            Some(Rule {
                action: RuleAction::Deny,
                operation: "file-read* file-write* file-test-existence".to_string(),
                path_type: Some(PathType::Subpath),
                path_value: Some(alias.join(".terrarium").display().to_string()),
                comment: Some(
                    "Hide project-local terrarium state through symlink alias".to_string(),
                ),
                source: RuleSource::Preset,
            })
        })
        .collect()
}

/// Resolves `.` and `..` lexically, without touching the filesystem: the alias
/// must keep its own (possibly symlinked) name, which canonicalizing would lose.
fn clean_absolute_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }

    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::RootDir => cleaned.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => cleaned.push(part),
            std::path::Component::ParentDir => {
                cleaned.pop();
            }
            std::path::Component::Prefix(_) => return None,
        }
    }
    Some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::super::{RenderRequest, render};
    use crate::config::project::ProjectProfile;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[cfg(unix)]
    #[test]
    fn resolve_adds_project_root_symlink_alias_rules() {
        let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("symlink_root_alias_{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        let alias = base.join("project-link");
        fs::create_dir_all(&project).unwrap();
        symlink(&project, &alias).unwrap();

        let mut profile = ProjectProfile::new("test", Some("rust".to_string()));
        profile.params.insert(
            "PROJECT_ROOT".to_string(),
            project.canonicalize().unwrap().display().to_string(),
        );
        let output = render(
            &RenderRequest::project(&profile, false).with_aliases(std::slice::from_ref(&alias)),
        )
        .unwrap();
        let alias_path = alias.display().to_string();
        let alias_state = alias.join(".terrarium").display().to_string();

        assert!(output.contains(&format!("(subpath \"{alias_path}\")")));
        assert!(output.contains(&format!("(path-ancestors \"{alias_path}\")")));
        assert!(output.contains("Hide project-local terrarium state through symlink alias"));
        assert!(output.contains(&format!("(subpath \"{alias_state}\")")));
        fs::remove_dir_all(&base).unwrap();
    }
}
