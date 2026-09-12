//! Undoing everything terrarium configured for a project.

use std::path::Path;

use crate::error::Result;

use super::claude_md::{claude_md_path, legacy_claude_md_path, remove_managed_claude_block};
use super::claude_settings::{clear_claude_runtime_settings, settings_local_path};
use super::gitignore::remove_project_gitignore_entries;
use super::mcp_registration::{project_mcp_path, unregister_mcp_server_http};

/// Removes everything terrarium configured for a project.
///
/// This deletes the private profile/workspace directory under `~/.terrarium`,
/// the managed CLAUDE.local.md block, the registered MCP server entry, the
/// terrarium-owned keys in `.claude/settings.local.json`, and the gitignore
/// entries terrarium added. Unrelated user settings are preserved.
///
/// Returns a description of each item removed, so the caller can report what a
/// clear actually did — an unmanaged project yields an empty list.
pub fn clear_project(project_root: &Path) -> Result<Vec<String>> {
    let mut removed = Vec::new();

    let profile_dir = crate::config::project::global_profile_dir(project_root);
    if profile_dir.exists() {
        std::fs::remove_dir_all(&profile_dir)?;
        removed.push(profile_dir.display().to_string());
    }

    if claude_md_path(project_root).exists() || legacy_claude_md_path(project_root).exists() {
        remove_managed_claude_block(project_root)?;
        removed.push("managed CLAUDE.local.md block".to_string());
    }

    let mcp_path = project_mcp_path(project_root);
    if mcp_path.exists() {
        unregister_mcp_server_http(project_root)?;
        removed.push(mcp_path.display().to_string());
    }

    if clear_claude_runtime_settings(project_root)? {
        removed.push(settings_local_path(project_root).display().to_string());
    }

    if remove_project_gitignore_entries(
        project_root,
        &[
            "/CLAUDE.local.md",
            "/.mcp.json",
            "/.claude/settings.local.json",
        ],
    )? {
        removed.push(".gitignore entries".to_string());
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::temp_project;
    use crate::app::{configure_claude, init_project, register_mcp_server_http};
    use std::fs;

    #[test]
    fn clear_project_removes_all_terrarium_state() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        configure_claude(&dir, false).unwrap();
        register_mcp_server_http(&dir, 12345).unwrap();

        let removed = clear_project(&dir).unwrap();

        assert!(!removed.is_empty(), "clear should report removed items");
        assert!(
            !crate::config::project::profile_path(&dir).exists(),
            "profile should be gone"
        );
        assert!(
            !crate::config::project::global_profile_dir(&dir).exists(),
            "profile directory should be gone"
        );
        assert!(!dir.join(".mcp.json").exists(), ".mcp.json should be gone");
        assert!(
            !settings_local_path(&dir).exists(),
            "terrarium-only settings.local.json should be gone"
        );
        assert!(
            !dir.join("CLAUDE.local.md").exists(),
            "terrarium-only CLAUDE.local.md should be gone"
        );
        assert!(
            !dir.join(".gitignore").exists(),
            "terrarium-only .gitignore should be gone"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn clear_project_preserves_unrelated_user_settings() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        fs::write(dir.join(".gitignore"), "/target\n").unwrap();
        fs::write(dir.join("CLAUDE.local.md"), "# My notes\n").unwrap();
        let claude_dir = dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"env":{"FOO":"bar"},"permissions":{"allow":["Bash(ls:*)"]}}"#,
        )
        .unwrap();
        configure_claude(&dir, false).unwrap();
        register_mcp_server_http(&dir, 12345).unwrap();

        clear_project(&dir).unwrap();

        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(settings_local_path(&dir)).unwrap()).unwrap();
        assert_eq!(settings["env"]["FOO"], "bar");
        assert_eq!(settings["permissions"]["allow"][0], "Bash(ls:*)");
        assert!(settings.get("sandbox").is_none());
        assert!(settings["permissions"].get("defaultMode").is_none());
        assert!(settings["permissions"].get("disableAutoMode").is_none());

        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(gitignore.contains("/target"));
        assert!(!gitignore.contains("/.mcp.json"));

        let notes = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(notes.contains("# My notes"));
        assert!(!notes.contains("terrarium-managed:start"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn clear_project_is_a_noop_for_unmanaged_project() {
        let (base, dir, _guard) = temp_project();
        let removed = clear_project(&dir).unwrap();
        assert!(removed.is_empty());
        fs::remove_dir_all(&base).unwrap();
    }
}
