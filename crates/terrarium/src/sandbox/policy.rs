//! Resolving the SBPL policy text a run will be sandboxed with.

use std::path::{Path, PathBuf};

use crate::config::project::ProjectProfile;
use crate::config::workspace::WorkspaceConfig;
use crate::error::{Result, TerrariumError};
use crate::profile::resolver;

/// Resolves and renders the SBPL profile for the given project root.
/// Returns `(sbpl_string, proxy_enabled)`.
pub(crate) fn build_sbpl(
    project_root: &Path,
    ws_config: Option<&WorkspaceConfig>,
    project_root_aliases: &[PathBuf],
) -> Result<(String, bool)> {
    if let Some(ws_config) = ws_config {
        let request = resolver::RenderRequest::workspace(project_root, ws_config)
            .with_aliases(project_root_aliases);
        return Ok((resolver::render(&request)?, ws_config.network.proxy_enabled));
    }

    let mut profile = load_or_default_profile(project_root)?;
    let proxy_enabled = profile.network.proxy_enabled;
    let params = resolver::build_params(&profile, project_root);
    for (k, v) in &params {
        profile.params.entry(k.clone()).or_insert_with(|| v.clone());
    }

    let request = resolver::RenderRequest::project(&profile, proxy_enabled)
        .with_aliases(project_root_aliases);
    // Write active.sb for reference/debugging even though execution no longer reads it.
    // Non-fatal: terrarium mcp may run inside an inherited sandbox where ~/.terrarium/ is not writable.
    let _ = resolver::write_active(&request, project_root);
    Ok((resolver::render(&request)?, proxy_enabled))
}

/// The project's saved profile, or a `none`-preset default when it has none —
/// terrarium still sandboxes an uninitialized project.
pub(crate) fn load_or_default_profile(project_root: &Path) -> Result<ProjectProfile> {
    match ProjectProfile::load(project_root) {
        Ok(p) => Ok(p),
        Err(TerrariumError::ProfileNotFound { .. }) => {
            Ok(ProjectProfile::new("default", Some("none".to_string())))
        }
        Err(e) => Err(e),
    }
}

/// The preset label shown in the banner and the instance registry. A workspace
/// reports its sub-projects' presets joined with `+`.
pub(crate) fn profile_name(project_root: &Path, ws_config: Option<&WorkspaceConfig>) -> String {
    if let Some(ws) = ws_config {
        let mut unique: Vec<&str> = ws.projects.iter().map(|p| p.preset.as_str()).collect();
        unique.sort();
        unique.dedup();
        return unique.join("+");
    }
    match ProjectProfile::load(project_root) {
        Ok(p) => p.preset.unwrap_or_else(|| "custom".to_string()),
        Err(_) => "none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::override_home_for_tests;
    use crate::test_support;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{}_{}", label, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A scratch project root with an overridden home, so nothing touches the
    /// developer's real `~/.terrarium`.
    fn temp_project(label: &str) -> (PathBuf, crate::config::global::TestHomeGuard) {
        let root = temp_dir(label);
        let home = root.join("home");
        fs::create_dir_all(home.join(".terrarium")).unwrap();
        let guard = override_home_for_tests(home);
        (root, guard)
    }

    #[test]
    fn load_or_default_profile_falls_back_to_none() {
        let dir = temp_dir("default_profile");
        // No .terrarium/profile.toml exists
        let profile = load_or_default_profile(&dir).unwrap();
        assert_eq!(profile.preset, Some("none".to_string()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn build_sbpl_with_none_preset() {
        let _lock = test_support::lock();
        let (root, _home) = temp_project("sbpl_none");
        // No profile.toml -> defaults to "none"
        let (sbpl, _proxy_enabled) = build_sbpl(&root, None, &[]).unwrap();
        assert!(sbpl.contains("(deny default)"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn build_sbpl_writes_active_sb() {
        let _lock = test_support::lock();
        let (root, _home) = temp_project("sbpl_active");
        let _ = build_sbpl(&root, None, &[]).unwrap();
        assert!(crate::config::project::active_sb_path(&root).exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn build_sbpl_honors_proxy_enabled_profile_setting() {
        let _lock = test_support::lock();
        let (root, _home) = temp_project("sbpl_proxy_enabled");

        let mut profile = ProjectProfile::new("test", Some("none".to_string()));
        profile.network.proxy_enabled = true;
        profile.save(&root).unwrap();

        let (sbpl, proxy_enabled) = build_sbpl(&root, None, &[]).unwrap();

        assert!(proxy_enabled);
        assert!(sbpl.contains("(allow network* (remote ip \"localhost:*\"))"));
        assert!(!sbpl.contains("(allow network*)\n"));
        fs::remove_dir_all(&root).unwrap();
    }
}
