//! The project's `.mcp.json` entry pointing Claude at terrarium's MCP server.
//!
//! The server listens on a port picked per run, so the entry is rewritten on
//! every `terrarium run` and removed when the run ends. Two names appear here:
//! `terrarium-<dir>` is the stdio entry older releases wrote and is only ever
//! deleted; `terrarium-<dir>-live` is the HTTP entry in use today.

use std::path::{Path, PathBuf};

use crate::config::global::claude_dir;
use crate::error::{Result, TerrariumError};

use super::claude_settings::{ensure_claude_runtime_settings, write_json_atomically};
use super::gitignore::ensure_project_gitignore_entries;

/// Registers an HTTP MCP server for `project_root` at the given port.
/// Always overwrites the existing entry because the port changes each run.
pub fn register_mcp_server_http(project_root: &Path, port: u16) -> Result<()> {
    let server_name = project_runtime_server_name(project_root);
    let settings_path = project_mcp_path(project_root);
    ensure_project_gitignore_entries(
        project_root,
        &[
            "/.mcp.json",
            // Kept as a literal rather than derived from `settings_local_path`:
            // this is a gitignore pattern, not a filesystem path.
            "/.claude/settings.local.json",
        ],
    )?;
    ensure_claude_runtime_settings(project_root)?;

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let entry = serde_json::json!({
        "type": "http",
        "url": server_url(port),
        "timeout": crate::mcp::tools::MCP_TOOL_TIMEOUT_MS
    });

    settings
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .unwrap();
    let servers = settings["mcpServers"].as_object_mut().unwrap();
    servers.remove(&project_server_name(project_root));
    servers.insert(server_name.clone(), entry);

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_atomically(&settings_path, "json.tmp", &settings)?;

    println!(
        "terrarium: registered MCP server '{server_name}' (HTTP) in {}",
        settings_path.display()
    );

    remove_stale_global_terrarium_entries();

    Ok(())
}

/// Confirms the entry Claude will read matches the port the server is on, so a
/// stale or missing registration fails the run rather than the first tool call.
pub fn verify_registered_mcp_server_http(project_root: &Path, port: u16) -> Result<String> {
    let expected_url = server_url(port);
    let actual_url = registered_mcp_server_http_url(project_root)?.ok_or_else(|| {
        TerrariumError::SandboxExecFailed {
            reason: format!(
                "MCP server entry '{}' missing from {}",
                project_runtime_server_name(project_root),
                project_mcp_path(project_root).display()
            ),
        }
    })?;

    if actual_url != expected_url {
        return Err(TerrariumError::SandboxExecFailed {
            reason: format!(
                "registered MCP URL mismatch in {}: expected {expected_url}, found {actual_url}",
                project_mcp_path(project_root).display()
            ),
        });
    }

    Ok(actual_url)
}

pub fn unregister_mcp_server_http(project_root: &Path) -> Result<()> {
    let settings_path = project_mcp_path(project_root);
    if !settings_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&settings_path)?;
    let mut value: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    let Some(servers) = value["mcpServers"].as_object_mut() else {
        return Ok(());
    };

    if servers
        .remove(&project_runtime_server_name(project_root))
        .is_none()
    {
        return Ok(());
    }

    // A file that held only terrarium's entry was terrarium's alone.
    if servers.is_empty()
        && let Some(root) = value.as_object_mut()
    {
        root.remove("mcpServers");
        if root.is_empty() {
            std::fs::remove_file(&settings_path)?;
            return Ok(());
        }
    }

    write_json_atomically(&settings_path, "json.tmp", &value)
}

fn registered_mcp_server_http_url(project_root: &Path) -> Result<Option<String>> {
    let settings_path = project_mcp_path(project_root);
    if !settings_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&settings_path)?;
    let value: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    Ok(value["mcpServers"]
        .get(project_runtime_server_name(project_root))
        .and_then(|entry| entry["url"].as_str())
        .map(ToOwned::to_owned))
}

fn server_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// The stdio entry name older releases wrote. Only removed, never written.
fn project_server_name(project_root: &Path) -> String {
    let dir_name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    format!("terrarium-{dir_name}")
}

fn project_runtime_server_name(project_root: &Path) -> String {
    format!("{}-live", project_server_name(project_root))
}

pub(super) fn project_mcp_path(project_root: &Path) -> PathBuf {
    project_root.join(".mcp.json")
}

/// Removes any `terrarium-*` keys from `~/.claude/settings.json`.
///
/// A global entry would point Claude at this project's server from every other
/// project, so any left by an older release is dropped.
fn remove_stale_global_terrarium_entries() {
    let global_path = claude_dir().join("settings.json");
    if !global_path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(&global_path) else {
        return;
    };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let Some(servers) = v
        .as_object_mut()
        .and_then(|o| o.get_mut("mcpServers"))
        .and_then(|s| s.as_object_mut())
    else {
        return;
    };
    let stale_keys: Vec<String> = servers
        .keys()
        .filter(|k| k.starts_with("terrarium-"))
        .cloned()
        .collect();
    if stale_keys.is_empty() {
        return;
    }
    for key in &stale_keys {
        servers.remove(key);
    }
    let _ = write_json_atomically(&global_path, "json.tmp", &v);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::claude_settings::settings_local_path;
    use crate::app::configure_claude;
    use crate::app::testing::temp_project;
    use std::fs;

    /// Reads `.mcp.json` and returns `(parsed, live_entry_key)`.
    fn read_mcp_json(dir: &Path) -> (serde_json::Value, String) {
        let content = fs::read_to_string(dir.join(".mcp.json")).unwrap();
        let value = serde_json::from_str(&content).unwrap();
        let key = format!(
            "terrarium-{}-live",
            dir.file_name().unwrap().to_str().unwrap()
        );
        (value, key)
    }

    fn read_settings_local(dir: &Path) -> serde_json::Value {
        let content = fs::read_to_string(settings_local_path(dir)).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[test]
    fn register_mcp_server_http_creates_http_entry() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 12345).unwrap();

        assert!(
            dir.join(".mcp.json").exists(),
            ".mcp.json should be created"
        );
        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(gitignore.lines().any(|line| line.trim() == "/.mcp.json"));
        let (v, key) = read_mcp_json(&dir);
        let servers = v["mcpServers"].as_object().unwrap();
        assert!(
            servers.contains_key(&key),
            "mcpServers should contain {key}"
        );
        assert_eq!(servers[&key]["type"], "http");
        assert_eq!(servers[&key]["url"], "http://127.0.0.1:12345/mcp");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn register_mcp_server_http_always_overwrites_port() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 11111).unwrap();
        register_mcp_server_http(&dir, 22222).unwrap();

        let (v, key) = read_mcp_json(&dir);
        assert_eq!(
            v["mcpServers"][&key]["url"], "http://127.0.0.1:22222/mcp",
            "second registration should overwrite with new port"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn verify_registered_mcp_server_http_returns_registered_url() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 12345).unwrap();

        let url = verify_registered_mcp_server_http(&dir, 12345).unwrap();

        assert_eq!(url, "http://127.0.0.1:12345/mcp");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn verify_registered_mcp_server_http_fails_on_port_mismatch() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 12345).unwrap();

        let err = verify_registered_mcp_server_http(&dir, 54321).unwrap_err();

        assert!(
            err.to_string().contains("registered MCP URL mismatch"),
            "unexpected error: {err}"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn unregister_mcp_server_http_removes_only_live_entry() {
        let (base, dir, _guard) = temp_project();
        let existing = serde_json::json!({
            "mcpServers": {
                "other-server": { "type": "http", "url": "http://127.0.0.1:9" }
            }
        });
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();
        register_mcp_server_http(&dir, 12345).unwrap();

        unregister_mcp_server_http(&dir).unwrap();

        let (v, live_key) = read_mcp_json(&dir);
        let servers = v["mcpServers"].as_object().unwrap();
        assert!(
            servers.contains_key("other-server"),
            "other servers should be preserved"
        );
        assert!(
            !servers.contains_key(&live_key),
            "live server should be removed"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn unregister_mcp_server_http_removes_file_when_only_live_entry() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 12345).unwrap();

        unregister_mcp_server_http(&dir).unwrap();

        assert!(
            !dir.join(".mcp.json").exists(),
            ".mcp.json should be removed when terrarium owned the only entry"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn unregister_mcp_server_http_preserves_other_top_level_settings() {
        let (base, dir, _guard) = temp_project();
        let existing = serde_json::json!({ "otherSetting": true });
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();
        register_mcp_server_http(&dir, 12345).unwrap();

        unregister_mcp_server_http(&dir).unwrap();

        let (v, _) = read_mcp_json(&dir);
        assert_eq!(v["otherSetting"], true);
        assert!(v.get("mcpServers").is_none());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_ignores_register_mcp_flag() {
        let (base, dir, _guard) = temp_project();
        configure_claude(&dir, false).unwrap();
        assert!(
            !dir.join(".mcp.json").exists(),
            "terrarium init should not pre-register a stdio MCP entry"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn register_mcp_server_http_removes_legacy_stdio_entry() {
        let (base, dir, _guard) = temp_project();
        let legacy_key = format!("terrarium-{}", dir.file_name().unwrap().to_str().unwrap());
        let existing = serde_json::json!({
            "mcpServers": {
                legacy_key.clone(): {
                    "type": "stdio",
                    "command": "terrarium",
                    "args": ["mcp", "--project", dir.display().to_string()]
                }
            }
        });
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        register_mcp_server_http(&dir, 12345).unwrap();

        let (v, live_key) = read_mcp_json(&dir);
        let servers = v["mcpServers"].as_object().unwrap();
        assert!(
            !servers.contains_key(&legacy_key),
            "legacy stdio entry should be removed"
        );
        assert!(
            servers.contains_key(&live_key),
            "live HTTP entry should be added"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn register_mcp_server_http_sets_mcp_tool_timeout() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 12345).unwrap();

        let (v, key) = read_mcp_json(&dir);
        assert_eq!(
            v["mcpServers"][&key]["timeout"],
            serde_json::json!(crate::mcp::tools::MCP_TOOL_TIMEOUT_MS),
            "server entry should carry a per-server timeout"
        );

        let s = read_settings_local(&dir);
        assert_eq!(
            s["env"]["MCP_TOOL_TIMEOUT"],
            crate::mcp::tools::MCP_TOOL_TIMEOUT_MS.to_string(),
            "settings.local.json should raise Claude Code's MCP tool timeout"
        );
        assert_eq!(
            s["sandbox"]["enabled"], false,
            "settings.local.json should disable Claude Code's nested sandbox"
        );

        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(
            gitignore
                .lines()
                .any(|line| line.trim() == "/.claude/settings.local.json"),
            "settings.local.json should be gitignored"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn register_mcp_server_http_preserves_existing_settings_local_values() {
        let (base, dir, _guard) = temp_project();
        let claude_dir = dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let existing = serde_json::json!({
            "permissions": { "allow": ["Bash(ls:*)"] },
            "env": { "FOO": "bar" }
        });
        fs::write(
            claude_dir.join("settings.local.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        register_mcp_server_http(&dir, 12345).unwrap();

        let s = read_settings_local(&dir);
        assert_eq!(s["env"]["FOO"], "bar", "other env vars should be preserved");
        assert_eq!(
            s["env"]["MCP_TOOL_TIMEOUT"],
            crate::mcp::tools::MCP_TOOL_TIMEOUT_MS.to_string()
        );
        assert_eq!(
            s["permissions"]["allow"][0], "Bash(ls:*)",
            "other top-level settings should be preserved"
        );
        assert_eq!(s["sandbox"]["enabled"], false);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn register_mcp_server_http_keeps_user_mcp_tool_timeout() {
        let (base, dir, _guard) = temp_project();
        let claude_dir = dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"env":{"MCP_TOOL_TIMEOUT":"42000"}}"#,
        )
        .unwrap();

        register_mcp_server_http(&dir, 12345).unwrap();

        let s = read_settings_local(&dir);
        assert_eq!(
            s["env"]["MCP_TOOL_TIMEOUT"], "42000",
            "a user-set timeout must not be overwritten"
        );
        assert_eq!(s["sandbox"]["enabled"], false);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn register_mcp_server_http_pins_bypass_permissions_mode() {
        let (base, dir, _guard) = temp_project();
        register_mcp_server_http(&dir, 12345).unwrap();

        let s = read_settings_local(&dir);
        assert_eq!(
            s["permissions"]["defaultMode"], "bypassPermissions",
            "terrarium provides the sandbox, so Claude must not prompt for permissions"
        );
        assert!(
            s["permissions"].get("disableAutoMode").is_none(),
            "auto mode is the fallback when bypass is off, so it must stay available"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    /// Older releases wrote `disableAutoMode: "disable"`, forcing the fallback to
    /// `default`. A project they configured must pick up auto mode on the next run.
    #[test]
    fn register_mcp_server_http_reenables_auto_mode_disabled_by_an_older_release() {
        let (base, dir, _guard) = temp_project();
        let claude_dir = dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"permissions":{"defaultMode":"bypassPermissions","disableAutoMode":"disable"}}"#,
        )
        .unwrap();

        register_mcp_server_http(&dir, 12345).unwrap();

        let s = read_settings_local(&dir);
        assert!(
            s["permissions"].get("disableAutoMode").is_none(),
            "the stale disable must be dropped"
        );
        assert_eq!(s["permissions"]["defaultMode"], "bypassPermissions");
        fs::remove_dir_all(&base).unwrap();
    }

    /// A `disableAutoMode` the user chose themselves is not terrarium's to undo.
    #[test]
    fn register_mcp_server_http_keeps_a_user_chosen_auto_mode_setting() {
        let (base, dir, _guard) = temp_project();
        let claude_dir = dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"permissions":{"disableAutoMode":"enable"}}"#,
        )
        .unwrap();

        register_mcp_server_http(&dir, 12345).unwrap();

        let s = read_settings_local(&dir);
        assert_eq!(
            s["permissions"]["disableAutoMode"], "enable",
            "only the value terrarium itself wrote may be removed"
        );
        fs::remove_dir_all(&base).unwrap();
    }
}
