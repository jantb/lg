//! Creating a project's or workspace's private terrarium profile.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::project::ProjectProfile;
use crate::config::workspace::WorkspaceProject;
use crate::error::{Result, TerrariumError};
use crate::profile::presets;

use super::detect::detect_project_preset;

/// Initializes a new private profile for the given project root.
pub fn init_project(
    project_root: &Path,
    preset: &str,
    proxy_enabled: bool,
    allow_commit: bool,
) -> Result<()> {
    if !presets::is_known_preset(preset) {
        return Err(TerrariumError::InvalidPreset {
            name: preset.to_string(),
        });
    }
    let preset = presets::normalize_preset(preset);

    let dir_name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");

    let mut profile = ProjectProfile::new(dir_name, Some(preset.clone()));
    profile.network.proxy_enabled = proxy_enabled;
    profile.allow_commit = allow_commit;

    profile.params.insert(
        "PROJECT_ROOT".to_string(),
        project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf())
            .display()
            .to_string(),
    );

    profile.save(project_root)?;

    println!(
        "terrarium: initialized {} for '{dir_name}' with preset '{preset}'",
        crate::config::project::profile_path(project_root).display()
    );

    Ok(())
}

/// Initializes a workspace configuration for a multi-project directory.
pub fn init_workspace(
    workspace_root: &Path,
    projects: &[WorkspaceProject],
    proxy_enabled: bool,
    allow_commit: bool,
) -> Result<()> {
    use crate::config::workspace::WorkspaceConfig;

    let config = WorkspaceConfig {
        projects: projects.to_vec(),
        network: crate::config::project::NetworkConfig {
            proxy_enabled,
            allowed_domains: Vec::new(),
        },
        allow_commit,
        command: None,
    };

    config.save(workspace_root)?;

    // Also init each sub-project with its own profile, but never overwrite one
    // that already exists. A workspace is commonly assembled by symlinking
    // repositories that are terrarium projects in their own right; rewriting
    // their profiles here would discard the manual rules and allowlists they
    // were set up with. The workspace config above is what drives this
    // workspace's sandbox, proxy and tools either way.
    for project in projects {
        let sub_root = workspace_root.join(&project.path);
        if !sub_root.exists() {
            continue;
        }
        if ProjectProfile::load(&sub_root).is_ok() {
            println!(
                "terrarium: kept existing profile for '{}' — not overwritten",
                project.path
            );
            continue;
        }
        init_project(&sub_root, &project.preset, proxy_enabled, allow_commit)?;
    }

    println!(
        "terrarium: initialized workspace with {} projects",
        projects.len()
    );

    Ok(())
}

/// Finds workspace project candidates. Top-level visible directories are kept for
/// backward compatibility; nested directories are added when they look like
/// project roots.
///
/// Symlinked directories count as directories: a workspace assembled by linking
/// sibling repositories into one folder is the common terrarium layout, and
/// skipping links would leave such a workspace with nothing to discover.
pub fn discover_workspace_project_dirs(workspace_root: &Path) -> Result<Vec<String>> {
    let mut projects = Vec::new();
    let mut visited = HashSet::new();
    collect_workspace_project_dirs(workspace_root, workspace_root, &mut projects, &mut visited)?;
    projects.sort();
    projects.dedup();
    Ok(projects)
}

fn collect_workspace_project_dirs(
    workspace_root: &Path,
    dir: &Path,
    projects: &mut Vec<String>,
    visited: &mut HashSet<PathBuf>,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(std::result::Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        // `path.is_dir()` follows symlinks, unlike the entry's own file type.
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if should_skip_workspace_discovery_dir(&name) {
            continue;
        }

        let rel = path.strip_prefix(workspace_root).unwrap_or(&path);
        let is_top_level = rel.components().count() == 1;
        let is_project_root = detect_project_preset(&path).is_some();

        if is_top_level || is_project_root {
            projects.push(relative_path_string(rel));
        }

        if !is_project_root {
            // Following links can loop; descend into each real directory once.
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if visited.insert(canonical) {
                collect_workspace_project_dirs(workspace_root, &path, projects, visited)?;
            }
        }
    }

    Ok(())
}

fn should_skip_workspace_discovery_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "build" | "dist" | "out" | "DerivedData"
        )
}

/// Joins with `/` rather than the platform separator: the result is written into
/// config files and compared against configured project paths.
fn relative_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{temp_dir, temp_project};
    use crate::config::workspace::WorkspaceConfig;
    use std::fs;

    #[test]
    fn init_creates_profile_toml() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        assert!(crate::config::project::profile_path(&dir).exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_with_proxy_disabled() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", false, false).unwrap();
        let profile = ProjectProfile::load(&dir).unwrap();
        assert!(!profile.network.proxy_enabled);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_with_proxy_enabled() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        let profile = ProjectProfile::load(&dir).unwrap();
        assert!(profile.network.proxy_enabled);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_workspace_persists_allow_commit_without_command() {
        let (base, dir, _guard) = temp_project();
        let projects = vec![WorkspaceProject {
            path: "svc-a".to_string(),
            preset: "rust".to_string(),
        }];

        init_workspace(&dir, &projects, true, true).unwrap();

        let config = WorkspaceConfig::load(&dir).unwrap();
        assert!(config.network.proxy_enabled);
        assert!(config.allow_commit);
        assert!(config.command.is_none());
        fs::remove_dir_all(&base).unwrap();
    }

    /// A workspace assembled from symlinked repositories must not rewrite those
    /// repositories' own profiles — they carry manual rules and allowlists the
    /// workspace knows nothing about.
    #[test]
    fn init_workspace_keeps_an_existing_sub_project_profile() {
        let (base, dir, _guard) = temp_project();
        let sub = dir.join("svc-a");
        fs::create_dir_all(&sub).unwrap();

        let mut existing = ProjectProfile::new("svc-a", Some("kotlin".to_string()));
        existing.allow_commit = true;
        existing
            .network
            .allowed_domains
            .push("internal.example".to_string());
        existing.save(&sub).unwrap();

        init_workspace(
            &dir,
            &[WorkspaceProject {
                path: "svc-a".to_string(),
                preset: "rust".to_string(),
            }],
            true,
            false,
        )
        .unwrap();

        let after = ProjectProfile::load(&sub).unwrap();
        assert_eq!(after.preset, Some("kotlin".to_string()));
        assert!(after.allow_commit);
        assert_eq!(after.network.allowed_domains, vec!["internal.example"]);
        // The workspace config itself still records what was selected.
        assert_eq!(
            WorkspaceConfig::load(&dir).unwrap().projects[0].preset,
            "rust"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    /// A sub-project with no profile of its own is still initialized.
    #[test]
    fn init_workspace_creates_a_missing_sub_project_profile() {
        let (base, dir, _guard) = temp_project();
        let sub = dir.join("svc-b");
        fs::create_dir_all(&sub).unwrap();

        init_workspace(
            &dir,
            &[WorkspaceProject {
                path: "svc-b".to_string(),
                preset: "node".to_string(),
            }],
            true,
            false,
        )
        .unwrap();

        assert_eq!(
            ProjectProfile::load(&sub).unwrap().preset,
            Some("node".to_string())
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn discover_workspace_project_dirs_includes_nested_project_roots() {
        let root = temp_dir();
        fs::create_dir_all(root.join("apps/api/src")).unwrap();
        fs::create_dir_all(root.join("backend/src")).unwrap();
        fs::create_dir_all(root.join("libs/shared/Sources")).unwrap();
        fs::create_dir_all(root.join("node_modules/ignored")).unwrap();
        fs::write(root.join("apps/api/Cargo.toml"), "[package]\n").unwrap();
        fs::write(root.join("backend/Cargo.toml"), "[package]\n").unwrap();
        fs::write(root.join("libs/shared/Package.swift"), "// swift\n").unwrap();
        fs::write(root.join("node_modules/ignored/Cargo.toml"), "[package]\n").unwrap();

        let projects = discover_workspace_project_dirs(&root).unwrap();

        assert!(projects.contains(&"apps".to_string()));
        assert!(projects.contains(&"apps/api".to_string()));
        assert!(projects.contains(&"backend".to_string()));
        assert!(projects.contains(&"libs".to_string()));
        assert!(projects.contains(&"libs/shared".to_string()));
        assert!(!projects.contains(&"backend/src".to_string()));
        assert!(!projects.contains(&"node_modules".to_string()));
        assert!(!projects.contains(&"node_modules/ignored".to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    /// The benk layout: sibling repositories symlinked into one workspace
    /// directory. `read_dir` reports those entries as symlinks rather than
    /// directories, which used to make discovery return nothing at all.
    #[test]
    fn discover_workspace_project_dirs_follows_symlinked_projects() {
        let root = temp_dir();
        let workspace = root.join("workspace");
        let linked = root.join("elsewhere/web");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(linked.join("src")).unwrap();
        fs::write(linked.join("package.json"), "{}\n").unwrap();
        std::os::unix::fs::symlink(&linked, workspace.join("web")).unwrap();

        let projects = discover_workspace_project_dirs(&workspace).unwrap();

        assert_eq!(projects, vec!["web".to_string()]);
        assert_eq!(
            crate::app::detect_project_preset_spec(&workspace.join("web")),
            Some("node".to_string()),
            "a symlinked project's preset must resolve through the link"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// A link pointing back at an ancestor must not send discovery into a loop.
    #[test]
    fn discover_workspace_project_dirs_survives_symlink_cycles() {
        let root = temp_dir();
        let workspace = root.join("workspace");
        fs::create_dir_all(workspace.join("nested")).unwrap();
        std::os::unix::fs::symlink(&workspace, workspace.join("nested/loop")).unwrap();

        let projects = discover_workspace_project_dirs(&workspace).unwrap();

        assert!(projects.contains(&"nested".to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn init_project_accepts_multi_language_preset() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, " node , Kotlin ,rust", true, false).unwrap();
        let profile = ProjectProfile::load(&dir).unwrap();
        assert_eq!(profile.preset, Some("node,kotlin,rust".to_string()));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_project_rejects_multi_preset_with_unknown_member() {
        let (base, dir, _guard) = temp_project();
        let result = init_project(&dir, "rust,bogus", true, false);
        let _ = fs::remove_dir_all(&base);
        assert!(result.is_err());
    }

    #[test]
    fn init_invalid_preset_returns_error() {
        let (base, dir, _guard) = temp_project();
        let result = init_project(&dir, "unknown_preset", true, false);
        let _ = fs::remove_dir_all(&base);
        assert!(result.is_err());
    }

    #[test]
    fn init_project_with_kotlin_preset() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "kotlin", true, false).unwrap();
        let profile = ProjectProfile::load(&dir).unwrap();
        assert_eq!(profile.preset, Some("kotlin".to_string()));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_project_with_swift_preset() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "swift", true, false).unwrap();
        let profile = ProjectProfile::load(&dir).unwrap();
        assert_eq!(profile.preset, Some("swift".to_string()));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_project_with_none_preset() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "none", true, false).unwrap();
        let profile = ProjectProfile::load(&dir).unwrap();
        assert_eq!(profile.preset, Some("none".to_string()));
        fs::remove_dir_all(&base).unwrap();
    }

    /// The sandbox profile derives tool paths itself, so baking `CARGO_HOME` and
    /// friends into the saved params would pin them to the initializing machine.
    #[test]
    fn init_project_sets_project_root_param_only() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        assert_only_project_root_param(&dir);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn init_kotlin_preset_sets_project_root_param_only() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "kotlin", true, false).unwrap();
        assert_only_project_root_param(&dir);
        fs::remove_dir_all(&base).unwrap();
    }

    fn assert_only_project_root_param(dir: &Path) {
        let profile = ProjectProfile::load(dir).unwrap();
        assert!(profile.params.contains_key("PROJECT_ROOT"));
        for absent in ["GRADLE_HOME", "MAVEN_HOME", "CARGO_HOME", "RUSTUP_HOME"] {
            assert!(
                !profile.params.contains_key(absent),
                "{absent} must not be baked into the profile"
            );
        }
    }
}
