//! Following symlinks out of the project and granting their targets.
//!
//! An allow rule on the project root does not reach a directory symlinked into
//! it: Seatbelt checks the resolved path. Two places need the extra rules — a
//! linked repository inside the project (granted read-write, matching the rule it
//! came from) and a skill linked into `~/.claude/skills` (granted read-only).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::project::{PathType, Rule, RuleAction, RuleSource};
use crate::profile::sbpl;

/// Directories not worth descending into when hunting for symlinks.
const SKIP_SCAN_DIRS: &[&str] = &[".git", ".terrarium", "target", "node_modules"];

/// For each allow rule on a subpath, adds the same grant to every directory
/// symlinked out of that subpath.
pub(super) fn expand_rules_for_symlink_targets(
    rules: &[Rule],
    params: &HashMap<String, String>,
) -> Vec<Rule> {
    let mut expanded = rules.to_vec();
    let canonical_project_root = params
        .get("PROJECT_ROOT")
        .and_then(|root| PathBuf::from(root).canonicalize().ok());
    let mut seen: HashSet<(String, Option<PathType>, String)> = rules
        .iter()
        .map(|rule| {
            (
                rule.operation.clone(),
                rule.path_type.clone(),
                rule.path_value.clone().unwrap_or_default(),
            )
        })
        .collect();

    for rule in rules {
        if !matches!(rule.action, RuleAction::Allow)
            || !matches!(rule.path_type, Some(PathType::Subpath))
        {
            continue;
        }
        let Some(path_value) = &rule.path_value else {
            continue;
        };
        let root = PathBuf::from(sbpl::substitute(path_value, params));
        let Some(project_root) = canonical_project_root.as_deref() else {
            continue;
        };
        for (link_path, target_path) in symlinked_directory_targets(&root, project_root) {
            let target_value = target_path.display().to_string();
            let target_key = (
                rule.operation.clone(),
                Some(PathType::Subpath),
                target_value.clone(),
            );
            if seen.insert(target_key) {
                expanded.push(Rule {
                    action: RuleAction::Allow,
                    operation: rule.operation.clone(),
                    path_type: Some(PathType::Subpath),
                    path_value: Some(target_value.clone()),
                    comment: Some(format!("Symlink target for {}", comment_path(&link_path))),
                    source: RuleSource::Preset,
                });
            }

            let metadata_key = (
                "file-read-metadata file-test-existence".to_string(),
                Some(PathType::Ancestors),
                target_value.clone(),
            );
            if seen.insert(metadata_key) {
                expanded.push(Rule {
                    action: RuleAction::Allow,
                    operation: "file-read-metadata file-test-existence".to_string(),
                    path_type: Some(PathType::Ancestors),
                    path_value: Some(target_value),
                    comment: Some(format!(
                        "Resolve symlink target ancestors for {}",
                        comment_path(&link_path)
                    )),
                    source: RuleSource::Preset,
                });
            }
        }
    }

    expanded
}

/// Read-only rules for skill symlink targets outside HOME/.claude.
pub(super) fn claude_skills_symlink_rules(params: &HashMap<String, String>) -> Vec<Rule> {
    let Some(home) = params.get("HOME") else {
        return Vec::new();
    };
    let claude_dir = Path::new(home).join(".claude");
    let canonical_claude = claude_dir
        .canonicalize()
        .unwrap_or_else(|_| claude_dir.clone());
    let skills_dir = claude_dir.join("skills");

    // Either the whole skills directory or an individual skill can be a link.
    let mut candidates = vec![skills_dir.clone()];
    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        candidates.extend(
            entries
                .filter_map(std::result::Result::ok)
                .map(|e| e.path()),
        );
    }

    let mut targets = Vec::new();
    for path in candidates {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(target) = path.canonicalize() else {
            continue;
        };
        // A link that stays inside `.claude` is already covered by its rules.
        if !target.is_dir() || target.starts_with(&canonical_claude) {
            continue;
        }
        targets.push((path, target));
    }
    targets.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    targets.dedup_by(|a, b| a.1 == b.1);

    let mut rules = Vec::new();
    for (link_path, target) in targets {
        let target_value = target.display().to_string();
        rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read*".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some(target_value.clone()),
            comment: Some(format!(
                "Symlinked skill target for {}",
                comment_path(&link_path)
            )),
            source: RuleSource::Preset,
        });
        rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read-metadata file-test-existence".to_string(),
            path_type: Some(PathType::Ancestors),
            path_value: Some(target_value),
            comment: Some(format!(
                "Resolve symlinked skill ancestors for {}",
                comment_path(&link_path)
            )),
            source: RuleSource::Preset,
        });
    }
    rules
}

/// Walks `root` for directory symlinks pointing outside it, returning
/// `(link, target)` pairs sorted and deduplicated by target.
fn symlinked_directory_targets(root: &Path, project_root: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(canonical_root) = root.canonicalize() else {
        return Vec::new();
    };
    // Only scan inside the project: a rule granting, say, `~/.cargo` should not
    // pull that tree's links into the profile.
    if !canonical_root.is_dir() || !canonical_root.starts_with(project_root) {
        return Vec::new();
    }

    let mut targets = Vec::new();
    let mut stack = vec![canonical_root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                let Ok(target) = path.canonicalize() else {
                    continue;
                };
                // Skip links that stay inside the scanned tree, and links to an
                // ancestor of the project — granting those would widen the
                // sandbox to everything above the project.
                if !target.is_dir()
                    || target.starts_with(&canonical_root)
                    || project_root.starts_with(&target)
                {
                    continue;
                }
                targets.push((path.to_path_buf(), target));
            } else if metadata.is_dir() {
                if should_skip_symlink_scan_dir(&path) {
                    continue;
                }
                stack.push(path);
            }
        }
    }

    targets.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    targets.dedup_by(|a, b| a.1 == b.1);
    targets
}

fn should_skip_symlink_scan_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| SKIP_SCAN_DIRS.contains(&name))
}

/// Comments become SBPL line comments, so an embedded newline would split one.
fn comment_path(path: &Path) -> String {
    path.display().to_string().replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::super::{RenderRequest, render};
    use crate::config::project::{PathType, ProjectProfile, Rule, RuleAction, RuleSource};
    use std::fs;
    use std::path::PathBuf;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    fn temp_dir(label: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{label}_{}", uuid::Uuid::new_v4()))
    }

    /// A rust-preset profile rooted at `project`.
    fn profile_rooted_at(project: &std::path::Path) -> ProjectProfile {
        let mut profile = ProjectProfile::new("test", Some("rust".to_string()));
        profile.params.insert(
            "PROJECT_ROOT".to_string(),
            project.canonicalize().unwrap().display().to_string(),
        );
        profile
    }

    #[cfg(unix)]
    #[test]
    fn resolve_adds_rules_for_symlinked_reference_directory() {
        let base = temp_dir("symlink_ref");
        let project = base.join("project");
        let reference = base.join("other-repo");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&reference).unwrap();
        symlink(&reference, project.join("reference")).unwrap();

        let profile = profile_rooted_at(&project);
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        let reference_path = reference.canonicalize().unwrap().display().to_string();

        assert!(output.contains(&format!("(subpath \"{reference_path}\")")));
        assert!(output.contains(&format!("(path-ancestors \"{reference_path}\")")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_skips_symlinks_to_allowed_root_ancestors() {
        let base = temp_dir("symlink_ancestor");
        let project = base.join("project");
        fs::create_dir_all(&project).unwrap();
        symlink(&base, project.join("parent")).unwrap();

        let profile = profile_rooted_at(&project);
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        let base_path = base.canonicalize().unwrap().display().to_string();

        assert!(!output.contains(&format!("(subpath \"{base_path}\")")));
        assert!(!output.contains(&format!("(path-ancestors \"{base_path}\")")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_only_expands_symlinks_from_project_root_rules() {
        let base = temp_dir("symlink_non_project");
        let project = base.join("project");
        let cache = base.join("cache");
        let reference = base.join("other-repo");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&reference).unwrap();
        symlink(&reference, cache.join("reference")).unwrap();

        let mut profile = ProjectProfile::new("test", Some("none".to_string()));
        profile.params.insert(
            "PROJECT_ROOT".to_string(),
            project.canonicalize().unwrap().display().to_string(),
        );
        profile.rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read* file-write*".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some(cache.canonicalize().unwrap().display().to_string()),
            comment: None,
            source: RuleSource::Manual,
        });

        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        let reference_path = reference.canonicalize().unwrap().display().to_string();

        assert!(!output.contains(&format!("(subpath \"{reference_path}\")")));
        assert!(!output.contains(&format!("(path-ancestors \"{reference_path}\")")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_does_not_scan_terrarium_state_for_symlinks() {
        let base = temp_dir("symlink_state");
        let project = base.join("project");
        let state = project.join(".terrarium");
        let reference = base.join("other-repo");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&reference).unwrap();
        symlink(&reference, state.join("reference")).unwrap();

        let profile = profile_rooted_at(&project);
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        let reference_path = reference.canonicalize().unwrap().display().to_string();

        assert!(!output.contains(&format!("(subpath \"{reference_path}\")")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_adds_ro_rules_for_symlinked_claude_skills() {
        let base = temp_dir("skill_symlink");
        let home = base.join("home");
        let project = base.join("project");
        let skill_target = base.join("skills-repo").join("my-skill");
        fs::create_dir_all(home.join(".claude/skills")).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&skill_target).unwrap();
        symlink(&skill_target, home.join(".claude/skills/my-skill")).unwrap();

        let mut profile = profile_rooted_at(&project);
        profile.params.insert(
            "HOME".to_string(),
            home.canonicalize().unwrap().display().to_string(),
        );
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        let target_path = skill_target.canonicalize().unwrap().display().to_string();

        assert!(output.contains(&format!(
            "(allow file-read*\n  (subpath \"{target_path}\"))"
        )));
        assert!(output.contains(&format!("(path-ancestors \"{target_path}\")")));
        assert!(!output.contains(&format!(
            "(allow file-read* file-write*\n  (subpath \"{target_path}\"))"
        )));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_skips_skill_symlinks_targeting_claude_dir() {
        let base = temp_dir("skill_symlink_internal");
        let home = base.join("home");
        let project = base.join("project");
        let internal_target = home.join(".claude").join("shared-skill");
        fs::create_dir_all(home.join(".claude/skills")).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&internal_target).unwrap();
        symlink(&internal_target, home.join(".claude/skills/shared")).unwrap();

        let mut profile = profile_rooted_at(&project);
        profile.params.insert(
            "HOME".to_string(),
            home.canonicalize().unwrap().display().to_string(),
        );
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        let target_path = internal_target
            .canonicalize()
            .unwrap()
            .display()
            .to_string();

        assert!(!output.contains(&format!("(subpath \"{target_path}\")")));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_adds_ro_rules_for_symlinked_claude_skills() {
        let base = temp_dir("skill_symlink_ws");
        let home = base.join("home");
        let workspace = base.join("workspace");
        let skill_target = base.join("skills-repo").join("ws-skill");
        fs::create_dir_all(home.join(".claude/skills")).unwrap();
        fs::create_dir_all(workspace.join("svc")).unwrap();
        fs::create_dir_all(&skill_target).unwrap();
        symlink(&skill_target, home.join(".claude/skills/ws-skill")).unwrap();

        // A workspace derives HOME from the environment rather than a profile.
        let original_home = std::env::var("HOME").unwrap();
        unsafe { std::env::set_var("HOME", home.canonicalize().unwrap().display().to_string()) };
        let config = crate::config::workspace::WorkspaceConfig {
            projects: vec![crate::config::workspace::WorkspaceProject {
                path: "svc".to_string(),
                preset: "rust".to_string(),
            }],
            network: Default::default(),
            allow_commit: false,
            command: None,
        };
        let output = render(&RenderRequest::workspace(&workspace, &config));
        unsafe { std::env::set_var("HOME", original_home) };

        let target_path = skill_target.canonicalize().unwrap().display().to_string();
        assert!(output.unwrap().contains(&format!(
            "(allow file-read*\n  (subpath \"{target_path}\"))"
        )));
        fs::remove_dir_all(&base).unwrap();
    }
}
