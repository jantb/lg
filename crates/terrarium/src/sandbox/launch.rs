//! Deciding what to run, with which arguments, and where its binary lives.

use std::path::{Path, PathBuf};

use crate::config::global::GlobalConfig;
use crate::config::workspace::WorkspaceConfig;

use super::policy::load_or_default_profile;

/// The command to launch when the caller named none: workspace config, then
/// project profile, then global config, then `claude`.
pub(super) fn configured_default_command(
    project_root: &Path,
    ws_config: Option<&WorkspaceConfig>,
) -> String {
    ws_config
        .and_then(|ws| non_empty_command(ws.command.as_deref()))
        .or_else(|| {
            load_or_default_profile(project_root)
                .ok()
                .and_then(|p| non_empty_command(p.command.as_deref()))
        })
        .unwrap_or_else(|| {
            GlobalConfig::load()
                .ok()
                .and_then(|g| non_empty_command(Some(&g.defaults.command)))
                .unwrap_or_else(|| "claude".to_string())
        })
}

/// A blank configured command is treated as unset, so it falls through to the
/// next source rather than failing the spawn.
fn non_empty_command(command: Option<&str>) -> Option<String> {
    command
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

fn command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

/// Returns true if the command refers to Claude Code.
pub(super) fn is_claude_command(command: &str) -> bool {
    command_basename(command) == "claude"
}

/// Terrarium provides the outer sandbox, so Claude must not prompt for its own
/// permissions regardless of whether it was selected implicitly or explicitly.
pub(super) fn ensure_claude_launch_args(command: &str, args: &mut Vec<String>) {
    const SKIP_PERMISSIONS: &str = "--dangerously-skip-permissions";
    if !is_claude_command(command) {
        return;
    }
    // Only `--dangerously-skip-permissions` grants bypass mode. Passing
    // `--permission-mode bypassPermissions` alongside it does not escalate and
    // instead lands the session back on the built-in default mode, cancelling
    // the dangerous flag — so the mode is never injected here. An explicit
    // `--permission-mode` from the caller is left untouched.
    if !args.iter().any(|arg| arg == SKIP_PERMISSIONS) {
        args.insert(0, SKIP_PERMISSIONS.to_string());
    }
}

/// Resolves a command name or path to its canonical absolute path.
///
/// This runs in the **parent** process (before fork/sandbox), so PATH lookup
/// is unrestricted. Passing the resolved absolute path to the spawn means the
/// sandboxed child never needs to scan PATH directories or follow symlinks —
/// both of which could be blocked by the sandbox policy.
pub(super) fn resolve_command_path(command: &str) -> String {
    if command.starts_with('/') {
        return std::fs::canonicalize(command)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| command.to_string());
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let candidate = PathBuf::from(dir).join(command);
            if candidate.is_file() {
                if let Ok(canonical) = std::fs::canonicalize(&candidate) {
                    return canonical.display().to_string();
                }
                return candidate.display().to_string();
            }
        }
    }
    command.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::override_home_for_tests;
    use crate::config::project::ProjectProfile;
    use crate::test_support;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{}_{}", label, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_command_path_returns_absolute_for_known_binary() {
        let result = resolve_command_path("/bin/sh");
        assert!(
            result.starts_with('/'),
            "expected absolute path, got {result}"
        );
    }

    #[test]
    fn resolve_command_path_resolves_via_path_env() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("resolve_cmd");
        let bin = dir.join("dummy-terrarium-test");
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let original = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", format!("{}:{original}", dir.display())) };
        let result = resolve_command_path("dummy-terrarium-test");
        unsafe { std::env::set_var("PATH", original) };
        assert_eq!(result, bin.canonicalize().unwrap().display().to_string());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_command_path_falls_back_for_unknown() {
        let result = resolve_command_path("definitely-does-not-exist-xyz");
        assert_eq!(result, "definitely-does-not-exist-xyz");
    }

    #[test]
    fn is_claude_command_returns_true_for_claude() {
        assert!(is_claude_command("claude"));
    }

    #[test]
    fn is_claude_command_returns_true_for_full_path() {
        assert!(is_claude_command("/usr/local/bin/claude"));
    }

    #[test]
    fn is_claude_command_returns_false_for_other() {
        assert!(!is_claude_command("bash"));
        assert!(!is_claude_command("/bin/sh"));
        assert!(!is_claude_command("agent"));
    }

    #[test]
    fn explicit_claude_command_gets_permission_bypass() {
        let mut args = vec!["--model".to_string(), "opus".to_string()];
        ensure_claude_launch_args("claude", &mut args);
        assert_eq!(args, ["--dangerously-skip-permissions", "--model", "opus"]);
    }

    #[test]
    fn claude_permission_bypass_is_not_duplicated() {
        let mut args = vec!["--dangerously-skip-permissions".to_string()];
        ensure_claude_launch_args("/usr/local/bin/claude", &mut args);
        assert_eq!(args, ["--dangerously-skip-permissions"]);
    }

    #[test]
    fn claude_permission_mode_is_never_injected() {
        let mut args = vec![];
        ensure_claude_launch_args("claude", &mut args);
        assert_eq!(args, ["--dangerously-skip-permissions"]);
    }

    #[test]
    fn explicit_permission_mode_is_respected() {
        let mut args = vec!["--permission-mode".to_string(), "plan".to_string()];
        ensure_claude_launch_args("claude", &mut args);
        assert_eq!(
            args,
            [
                "--dangerously-skip-permissions",
                "--permission-mode",
                "plan"
            ]
        );
    }

    #[test]
    fn non_claude_command_is_unchanged() {
        let mut args = vec!["-l".to_string()];
        ensure_claude_launch_args("bash", &mut args);
        assert_eq!(args, ["-l"]);
    }

    #[test]
    fn configured_default_command_prefers_workspace_command() {
        let config = WorkspaceConfig {
            projects: vec![],
            network: Default::default(),
            allow_commit: false,
            command: Some("myclaude".to_string()),
        };

        assert_eq!(
            configured_default_command(Path::new("/tmp/no-profile-needed"), Some(&config)),
            "myclaude"
        );
    }

    #[test]
    fn configured_default_command_ignores_blank_workspace_command() {
        let _lock = test_support::lock();
        let root = temp_dir("blank_ws_command");
        let home = root.join("home");
        fs::create_dir_all(home.join(".terrarium")).unwrap();
        let _home = override_home_for_tests(home);

        let mut profile = ProjectProfile::new("test", Some("none".to_string()));
        profile.command = Some("myclaude".to_string());
        profile.save(&root).unwrap();
        let config = WorkspaceConfig {
            projects: vec![],
            network: Default::default(),
            allow_commit: false,
            command: Some("   ".to_string()),
        };

        assert_eq!(configured_default_command(&root, Some(&config)), "myclaude");
        fs::remove_dir_all(&root).unwrap();
    }
}
