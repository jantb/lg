//! The substitution parameters an SBPL profile is rendered against.
//!
//! Tool paths are detected here rather than saved into a profile, so a profile
//! stays portable between machines.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::config::project::ProjectProfile;

/// Auto-detects common environment paths and sets PROJECT_ROOT.
fn base_env_params(project_root: &Path) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Ok(home) = std::env::var("HOME") {
        params.insert("HOME".to_string(), home.clone());
        params.insert("CARGO_HOME".to_string(), format!("{home}/.cargo"));
        params.insert("GRADLE_HOME".to_string(), format!("{home}/.gradle"));
        params.insert("MAVEN_HOME".to_string(), format!("{home}/.m2"));
        params.insert("RUSTUP_HOME".to_string(), format!("{home}/.rustup"));
    }
    if let Some(java_home) = detect_java_home() {
        params.insert("JAVA_HOME".to_string(), java_home);
    }
    params.insert(
        "PROJECT_ROOT".to_string(),
        project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf())
            .display()
            .to_string(),
    );
    params
}

/// Builds the params map with auto-detected values merged with profile params.
pub fn build_params(profile: &ProjectProfile, project_root: &Path) -> HashMap<String, String> {
    let mut params = base_env_params(project_root);
    // Profile params override auto-detected values
    for (k, v) in &profile.params {
        params.insert(k.clone(), v.clone());
    }
    params
}

/// Builds the params map for a workspace. PROJECT_ROOT points to the workspace root.
/// Sub-project roots are not added as separate params since they're subpaths of PROJECT_ROOT.
pub fn build_workspace_params(workspace_root: &Path) -> HashMap<String, String> {
    base_env_params(workspace_root)
}

/// `JAVA_HOME` if set, else whatever `/usr/libexec/java_home` reports.
///
/// Gradle runs with this in its environment, so the sandbox must also grant the
/// same path — which is why detection lives beside the profile parameters.
pub fn detect_java_home() -> Option<String> {
    if let Ok(java_home) = std::env::var("JAVA_HOME")
        && !java_home.trim().is_empty()
    {
        return Some(java_home);
    }

    let output = Command::new("/usr/libexec/java_home").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let java_home = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if java_home.is_empty() {
        None
    } else {
        Some(java_home)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_params_sets_project_root() {
        let profile = ProjectProfile::new("test", None);
        let params = build_params(&profile, Path::new("/tmp"));
        assert!(params.contains_key("PROJECT_ROOT"));
    }

    #[test]
    fn build_params_profile_overrides_defaults() {
        let mut profile = ProjectProfile::new("test", None);
        profile
            .params
            .insert("CARGO_HOME".to_string(), "/custom/.cargo".to_string());
        let params = build_params(&profile, Path::new("/tmp"));
        assert_eq!(params["CARGO_HOME"], "/custom/.cargo");
    }

    #[test]
    fn build_params_uses_java_home_from_env_when_present() {
        let original_java_home = std::env::var("JAVA_HOME").ok();
        unsafe { std::env::set_var("JAVA_HOME", "/custom/java/home") };
        let profile = ProjectProfile::new("test", None);
        let params = build_params(&profile, Path::new("/tmp"));
        assert_eq!(params["JAVA_HOME"], "/custom/java/home");
        match original_java_home {
            Some(value) => unsafe { std::env::set_var("JAVA_HOME", value) },
            None => unsafe { std::env::remove_var("JAVA_HOME") },
        }
    }

    #[test]
    fn build_params_without_home_env() {
        // Temporarily remove HOME to test the fallback
        let original_home = std::env::var("HOME").ok();
        unsafe { std::env::remove_var("HOME") };
        let profile = ProjectProfile::new("test", None);
        let params = build_params(&profile, Path::new("/tmp"));
        // Without HOME, CARGO_HOME etc should not be set
        assert!(!params.contains_key("CARGO_HOME"));
        assert!(!params.contains_key("GRADLE_HOME"));
        // Restore HOME
        if let Some(h) = original_home {
            unsafe { std::env::set_var("HOME", h) };
        }
    }

    #[test]
    fn workspace_params_include_project_root() {
        let params = build_workspace_params(Path::new("/tmp/ws"));
        assert_eq!(params["PROJECT_ROOT"], "/tmp/ws");
    }
}
