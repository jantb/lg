//! Launching a command inside the Seatbelt sandbox and tracking it while it runs.
//!
//! The sandbox is applied by calling `sandbox_init(3)` from a `pre_exec` hook
//! between fork and exec — no `sandbox-exec` subprocess is involved, so the
//! sandboxed process is the command itself.

use chrono::Utc;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use uuid::Uuid;

use crate::error::{Result, TerrariumError};
use crate::sandbox::registry::{self, InstanceRecord};
use crate::sandbox::seatbelt;

use super::banner::print_banner;
use super::launch::{
    configured_default_command, ensure_claude_launch_args, is_claude_command, resolve_command_path,
};
use super::mcp_session;
use super::policy::profile_name;
use super::proxy_env;

pub(crate) use super::policy::build_sbpl;

pub struct RunOptions {
    pub project_root: PathBuf,
    pub project_root_aliases: Vec<PathBuf>,
    /// Optional command + args override. If None, uses global config default.
    pub command: Option<Vec<String>>,
}

/// Resolves the sandbox profile, then launches the configured command (default: `claude`)
/// inside a sandbox applied via `sandbox_init(3)` in a `pre_exec` hook — no `sandbox-exec`
/// subprocess is spawned.
///
/// Returns the instance ID.
pub async fn run(opts: RunOptions) -> Result<String> {
    let project_root = opts
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| opts.project_root.clone());
    let project_root_aliases = &opts.project_root_aliases;

    let ws_config = crate::config::workspace::WorkspaceConfig::load(&project_root).ok();
    let (sbpl, proxy_enabled) =
        build_sbpl(&project_root, ws_config.as_ref(), project_root_aliases)?;
    let preset = profile_name(&project_root, ws_config.as_ref());
    print_banner(&preset, proxy_enabled);

    let (command, mut extra_args) = resolve_command(&project_root, ws_config.as_ref(), &opts)?;
    ensure_claude_launch_args(&command, &mut extra_args);

    // Only Claude reads `.mcp.json`, so only a Claude session gets a server.
    let claude_session = is_claude_command(&command);
    if claude_session {
        mcp_session::start_and_register(&project_root, ws_config.as_ref()).await?;
    }

    let instance_id = short_id();
    let sb_profile_path = crate::config::project::active_sb_path(&project_root)
        .display()
        .to_string();
    let args: Vec<&str> = extra_args.iter().map(|s| s.as_str()).collect();

    let proxy_env =
        proxy_env::start_if_enabled(&project_root, ws_config.as_ref(), proxy_enabled).await?;
    let proxy_env_refs: Vec<(&str, &str)> = proxy_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let spawn_cmd = resolve_command_path(&command);
    let mut child = match spawn_with_sandbox(&spawn_cmd, &args, &sbpl, &proxy_env_refs) {
        Ok(child) => child,
        Err(err) => {
            if claude_session {
                mcp_session::unregister(&project_root);
            }
            return Err(err);
        }
    };

    let pid = child
        .id()
        .ok_or_else(|| TerrariumError::SandboxExecFailed {
            reason: "could not get child PID".to_string(),
        })?;

    registry::register(InstanceRecord {
        id: instance_id.clone(),
        pid,
        project_root: project_root.display().to_string(),
        profile_name: preset,
        started_at: Utc::now(),
        sb_profile_path,
    })?;

    println!(
        "terrarium: started instance {instance_id} (pid {pid}) — sandbox applied via sandbox_init"
    );

    let _ = child.wait().await;
    if claude_session {
        mcp_session::unregister(&project_root);
    }
    let _ = registry::unregister(&instance_id);

    Ok(instance_id)
}

/// Splits the caller's command override into `(program, args)`, or falls back to
/// the configured default.
fn resolve_command(
    project_root: &Path,
    ws_config: Option<&crate::config::workspace::WorkspaceConfig>,
    opts: &RunOptions,
) -> Result<(String, Vec<String>)> {
    match opts.command.as_ref() {
        Some(parts) => {
            let first = parts
                .first()
                .ok_or_else(|| TerrariumError::SandboxExecFailed {
                    reason: "command list must not be empty".to_string(),
                })?;
            Ok((first.clone(), parts[1..].to_vec()))
        }
        None => Ok((configured_default_command(project_root, ws_config), vec![])),
    }
}

/// Kills a sandboxed instance by ID.
pub async fn kill(instance_id: &str) -> Result<()> {
    let instance =
        registry::find(instance_id)?.ok_or_else(|| TerrariumError::InstanceNotFound {
            id: instance_id.to_string(),
        })?;

    let pid = nix::unistd::Pid::from_raw(instance.pid as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).map_err(|_| {
        TerrariumError::SandboxExecFailed {
            reason: format!("failed to kill pid {}", instance.pid),
        }
    })?;

    if let Err(err) = crate::app::unregister_mcp_server_http(Path::new(&instance.project_root)) {
        eprintln!("terrarium: warning: failed to unregister MCP server: {err}");
    }
    registry::unregister(instance_id)?;
    println!("terrarium: killed instance {instance_id}");
    Ok(())
}

/// Spawns `command` with the SBPL sandbox applied in a `pre_exec` hook.
///
/// The flow: Rust forks → child calls `sandbox_init(sbpl)` → child execs `command`.
/// No `sandbox-exec` binary is involved.
fn spawn_with_sandbox(
    command: &str,
    args: &[&str],
    sbpl: &str,
    extra_env: &[(&str, &str)],
) -> Result<tokio::process::Child> {
    let sbpl_owned = sbpl.to_string();

    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.envs(extra_env.iter().copied());
    // SAFETY: pre_exec runs after fork, before exec, in the single-threaded child.
    // `sandbox_init` is safe to call in that context.
    unsafe {
        cmd.as_std_mut()
            .pre_exec(move || seatbelt::apply(&sbpl_owned));
    }

    cmd.spawn().map_err(|e| TerrariumError::SandboxExecFailed {
        reason: format!("failed to spawn '{command}': {e}"),
    })
}

/// The first segment of a UUID: short enough to type, unique enough for the
/// handful of instances alive at once.
fn short_id() -> String {
    Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_is_8_chars() {
        let id = short_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
