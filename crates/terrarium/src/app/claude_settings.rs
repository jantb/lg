//! Claude Code's own settings, adjusted for running inside terrarium's sandbox.
//!
//! Terrarium applies the stricter Seatbelt profile before Claude starts, so
//! Claude's nested sandbox and permission prompts are turned off here. Only the
//! keys terrarium owns are written, and [`clear_claude_runtime_settings`] removes
//! exactly those again — a user's unrelated settings survive both directions.

use std::path::Path;

use crate::error::Result;

const TIMEOUT_KEY: &str = "MCP_TOOL_TIMEOUT";

/// The permission mode terrarium runs Claude in, matching the
/// `--dangerously-skip-permissions` flag it launches with.
const BYPASS_MODE: &str = "bypassPermissions";

/// Auto mode is Claude's classifier-driven permission mode. Terrarium leaves it
/// alone, so it stays the fallback whenever bypass mode is not in effect.
const AUTO_MODE_KEY: &str = "disableAutoMode";
const AUTO_MODE_DISABLED: &str = "disable";

/// Records that Claude Code's bypass-permissions warning has been accepted.
///
/// Without this flag Claude shows an interactive confirmation screen on every
/// start, so `--dangerously-skip-permissions` never takes effect unattended even
/// though Terrarium already applies the stricter Seatbelt sandbox.
pub fn ensure_bypass_permissions_accepted() -> Result<()> {
    const FLAG: &str = "bypassPermissionsModeAccepted";
    let path = crate::config::global::home_dir().join(".claude.json");

    let mut config = read_json_object(&path)?;

    let Some(root) = config.as_object_mut() else {
        return Ok(());
    };
    if root.get(FLAG) == Some(&serde_json::Value::Bool(true)) {
        return Ok(());
    }
    root.insert(FLAG.to_string(), serde_json::Value::Bool(true));

    write_json_atomically(&path, "json.terrarium.tmp", &config)
}

/// Configures Claude for execution inside Terrarium's outer sandbox while
/// preserving unrelated project-local settings.
pub(super) fn ensure_claude_runtime_settings(project_root: &Path) -> Result<()> {
    let settings_path = settings_local_path(project_root);
    let mut settings = read_json_object(&settings_path)?;

    let Some(root) = settings.as_object_mut() else {
        return Ok(());
    };
    let Some(env) = root
        .entry("env")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return Ok(());
    };
    // A user-set timeout wins: terrarium only raises Claude's ~120s default.
    if !env.contains_key(TIMEOUT_KEY) {
        env.insert(
            TIMEOUT_KEY.to_string(),
            serde_json::Value::String(crate::mcp::tools::MCP_TOOL_TIMEOUT_MS.to_string()),
        );
    }

    // Terrarium provides the outer sandbox, so Claude runs without its own
    // permission prompts. Pinning the default mode here keeps bypass mode active
    // for sessions that Claude restores or resumes from this project.
    let permissions = object_entry(root, "permissions");
    permissions.insert(
        "defaultMode".to_string(),
        serde_json::Value::String(BYPASS_MODE.to_string()),
    );
    // Bypass mode is what terrarium launches with, so auto mode is only reached
    // when bypass is off — a mode switch inside the session. Auto is the better
    // fallback there than `default`, so terrarium must not disable it. Earlier
    // releases wrote `disableAutoMode: "disable"`, which forced that fallback to
    // `default`; drop the value we wrote so existing projects pick this up.
    if permissions.get(AUTO_MODE_KEY) == Some(&serde_json::json!(AUTO_MODE_DISABLED)) {
        permissions.remove(AUTO_MODE_KEY);
    }

    // Claude's built-in sandbox must be disabled because Terrarium already
    // applies the stricter Seatbelt profile before Claude starts.
    object_entry(root, "sandbox").insert("enabled".to_string(), serde_json::Value::Bool(false));

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_atomically(&settings_path, "json.tmp", &settings)
}

/// Removes the `settings.local.json` keys terrarium writes, keeping user values.
/// Returns true when the file changed or was deleted.
pub(super) fn clear_claude_runtime_settings(project_root: &Path) -> Result<bool> {
    let settings_path = settings_local_path(project_root);
    if !settings_path.exists() {
        return Ok(false);
    }
    let mut settings = read_json_object(&settings_path)?;
    let Some(root) = settings.as_object_mut() else {
        return Ok(false);
    };

    let mut changed = false;
    if let Some(env) = root.get_mut("env").and_then(|v| v.as_object_mut()) {
        changed |= env.remove(TIMEOUT_KEY).is_some();
        if env.is_empty() {
            root.remove("env");
        }
    }
    if let Some(permissions) = root.get_mut("permissions").and_then(|v| v.as_object_mut()) {
        // Only remove values terrarium itself would have written.
        if permissions.get("defaultMode") == Some(&serde_json::json!(BYPASS_MODE)) {
            permissions.remove("defaultMode");
            changed = true;
        }
        // No longer written, but still cleaned up for projects an older release
        // configured.
        if permissions.get(AUTO_MODE_KEY) == Some(&serde_json::json!(AUTO_MODE_DISABLED)) {
            permissions.remove(AUTO_MODE_KEY);
            changed = true;
        }
        if permissions.is_empty() {
            root.remove("permissions");
        }
    }
    if root.get("sandbox") == Some(&serde_json::json!({"enabled": false})) {
        root.remove("sandbox");
        changed = true;
    }

    if !changed {
        return Ok(false);
    }
    if root.is_empty() {
        std::fs::remove_file(&settings_path)?;
        return Ok(true);
    }
    write_json_atomically(&settings_path, "json.tmp", &settings)?;
    Ok(true)
}

pub(super) fn settings_local_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".claude").join("settings.local.json")
}

/// Reads a JSON file, treating a missing or unparseable file as an empty object:
/// terrarium's keys are still worth writing when the rest is unreadable.
fn read_json_object(path: &Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content).unwrap_or(serde_json::json!({})))
}

/// Returns `root[key]` as an object, replacing a non-object value.
fn object_entry<'a>(
    root: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    let value = root.entry(key).or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    value.as_object_mut().unwrap()
}

/// Writes via a temporary file and rename, so a crash mid-write cannot leave
/// Claude with a truncated settings file.
pub(super) fn write_json_atomically(
    path: &Path,
    tmp_extension: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let tmp_path = path.with_extension(tmp_extension);
    let content = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::temp_dir;
    use crate::config::global::override_home_for_tests;
    use std::fs;

    #[test]
    fn ensure_bypass_permissions_accepted_sets_flag_and_preserves_config() {
        let home = temp_dir();
        let _guard = override_home_for_tests(home.clone());
        fs::write(
            home.join(".claude.json"),
            r#"{"numStartups": 7, "projects": {"/tmp/p": {"hasTrustDialogAccepted": true}}}"#,
        )
        .unwrap();

        ensure_bypass_permissions_accepted().unwrap();

        let content = fs::read_to_string(home.join(".claude.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["bypassPermissionsModeAccepted"], true);
        assert_eq!(v["numStartups"], 7, "unrelated config must be preserved");
        assert_eq!(v["projects"]["/tmp/p"]["hasTrustDialogAccepted"], true);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn ensure_bypass_permissions_accepted_creates_missing_config() {
        let home = temp_dir();
        let _guard = override_home_for_tests(home.clone());

        ensure_bypass_permissions_accepted().unwrap();

        let content = fs::read_to_string(home.join(".claude.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["bypassPermissionsModeAccepted"], true);
        fs::remove_dir_all(&home).unwrap();
    }
}
