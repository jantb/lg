//! Read-only commands: checking a profile compiles, and listing live instances.

use std::path::Path;

use crate::error::Result;
use crate::sandbox::registry;

/// Validates the sandbox profile by spawning `/usr/bin/true` under the sandbox
/// (via `sandbox_init` in `pre_exec`) and checking the exit status.
pub fn validate_profile(project_root: &Path) -> Result<()> {
    use crate::sandbox::seatbelt;
    use std::os::unix::process::CommandExt;

    let ws_config = crate::config::workspace::WorkspaceConfig::load(project_root).ok();
    let (sbpl, _proxy_enabled) =
        crate::sandbox::runner::build_sbpl(project_root, ws_config.as_ref(), &[])?;

    let sbpl_clone = sbpl.clone();
    let mut cmd = std::process::Command::new("/usr/bin/true");
    // SAFETY: pre_exec runs after fork, before exec, in the single-threaded child.
    unsafe {
        cmd.pre_exec(move || seatbelt::apply(&sbpl_clone));
    }

    let output = cmd
        .output()
        .map_err(|e| crate::error::TerrariumError::SandboxExecFailed {
            reason: e.to_string(),
        })?;

    let active_sb = crate::config::project::active_sb_path(project_root);
    if output.status.success() {
        println!(
            "terrarium: profile validation passed — {}",
            active_sb.display()
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("terrarium: profile validation FAILED\n{stderr}");
    }

    Ok(())
}

/// Prints all live instances to stdout.
pub fn print_instances() -> Result<()> {
    let instances = registry::list_live()?;
    if instances.is_empty() {
        println!("No running terrarium instances.");
        return Ok(());
    }
    println!("{:<12} {:<8} {:<12} ROOT", "ID", "PID", "PROFILE");
    for i in instances {
        println!(
            "{:<12} {:<8} {:<12} {}",
            i.id, i.pid, i.profile_name, i.project_root
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{temp_dir, temp_project};
    use crate::config::global::override_home_for_tests;
    use crate::config::workspace::{WorkspaceConfig, WorkspaceProject};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn validate_profile_uses_workspace_config() {
        let (base, dir, _guard) = temp_project();
        let config = WorkspaceConfig {
            projects: vec![WorkspaceProject {
                path: "svc-a".to_string(),
                preset: "bogus".to_string(),
            }],
            network: Default::default(),
            allow_commit: false,
            command: None,
        };
        config.save(&dir).unwrap();

        let err = validate_profile(&dir).unwrap_err();

        assert!(matches!(
            err,
            crate::error::TerrariumError::InvalidPreset { .. }
        ));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn print_instances_does_not_panic() {
        let home = temp_dir();
        let _home = override_home_for_tests(home.clone());
        let result = print_instances();
        fs::remove_dir_all(&home).unwrap();
        assert!(result.is_ok());
    }

    /// Listing must not fail just because the registry cannot be created — a
    /// read-only home is a reason to report nothing running, not to error.
    #[test]
    fn print_instances_succeeds_when_home_is_not_writable_and_registry_is_missing() {
        let home = temp_dir();
        let mut perms = fs::metadata(&home).unwrap().permissions();
        perms.set_mode(0o500);
        fs::set_permissions(&home, perms).unwrap();

        let _home = override_home_for_tests(home.clone());
        let result = print_instances();

        let mut cleanup_perms = fs::metadata(&home).unwrap().permissions();
        cleanup_perms.set_mode(0o700);
        fs::set_permissions(&home, cleanup_perms).unwrap();
        fs::remove_dir_all(&home).unwrap();

        assert!(result.is_ok());
    }
}
