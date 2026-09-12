//! Agent profiles and executable discovery. Commands are never interpreted by a shell.
use crate::{preferences::Agent, session::SessionKind, term::Spawn};
use std::path::{Path, PathBuf};
pub fn kind(agent: &Agent) -> SessionKind {
    match agent.adapter.as_str() {
        "claude" => SessionKind::Claude,
        "codex" => SessionKind::Codex,
        "pi" => SessionKind::Pi,
        _ => SessionKind::Terminal,
    }
}
pub fn resolve(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let p = PathBuf::from(program);
        return executable(&p).then_some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join(program))
        .find(|p| executable(p))
}
/// The confinement mode in the words the picker and Settings show.
pub fn confinement_label(confinement: &str) -> &'static str {
    match confinement {
        "terrarium" => "Terrarium sandbox",
        "agent" => "agent's own permission sandbox",
        _ => "no sandbox",
    }
}
/// What lg can do with a session of this agent, in plain words.
pub fn capabilities(agent: &Agent) -> &'static str {
    match agent.adapter.as_str() {
        "claude" => "prompt, live activity, resume",
        "terminal" => "plain terminal, no agent integration",
        _ => "prompt only; activity and resume not tracked",
    }
}
pub fn describe(agent: &Agent) -> String {
    format!(
        "{} · {} · {} · {}",
        agent.name,
        resolve(&agent.executable)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| format!("{} not installed", agent.executable)),
        if agent.adapter == "terminal" {
            confinement_label("direct")
        } else {
            confinement_label(&agent.confinement)
        },
        capabilities(agent)
    )
}
pub fn spawn(
    agent: &Agent,
    cwd: &Path,
    hooks: Option<&Path>,
    prompt: Option<&str>,
) -> anyhow::Result<Spawn> {
    let executable = resolve(&agent.executable).ok_or_else(|| {
        anyhow::anyhow!(
            "{} is unavailable: {} was not found or is not executable",
            agent.name,
            agent.executable
        )
    })?;
    let sandboxed = agent.sandboxed();
    let mut spawn = match kind(agent) {
        SessionKind::Claude => crate::session::claude_spawn(cwd, sandboxed, hooks, None),
        SessionKind::Codex => crate::session::codex_spawn(cwd, sandboxed, None),
        SessionKind::Pi => crate::session::pi_spawn(cwd, sandboxed, None),
        SessionKind::Terminal => crate::session::shell_spawn(cwd),
    };
    if sandboxed {
        let index = spawn
            .args
            .iter()
            .position(|s| s == "--")
            .context("sandbox command separator")?
            + 1;
        spawn.args[index] = executable.to_string_lossy().into_owned();
    } else {
        spawn.program = executable.to_string_lossy().into_owned();
    }
    if agent.confinement == "direct" {
        spawn.args.clear();
    }
    spawn.args.extend(agent.args.clone());
    if !agent.model.is_empty() && agent.adapter != "terminal" {
        spawn.args.extend(["--model".into(), agent.model.clone()]);
    }
    if let Some(prompt) = prompt.filter(|p| !p.trim().is_empty()) {
        if agent.adapter == "pi" {
            spawn.args.push("--".into());
        }
        spawn.args.push(prompt.into());
    }
    Ok(spawn)
}
use anyhow::Context;

/// Version checks run off the UI thread with a deadline and file-backed output
/// so a noisy or stuck executable cannot fill a pipe or stall navigation.
pub fn diagnose(agent: &Agent) -> String {
    use std::{
        io::{Read, Seek},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let Some(program) = resolve(&agent.executable) else {
        return describe(agent);
    };
    let result = (|| -> anyhow::Result<String> {
        let mut output = tempfile::tempfile()?;
        let mut child = Command::new(program)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = child.try_wait()? {
                output.rewind()?;
                let mut text = String::new();
                output.take(2048).read_to_string(&mut text)?;
                return Ok(format!(
                    "{} · {}",
                    status,
                    text.lines().next().unwrap_or("no version reported")
                ));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Ok("version check timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    format!(
        "{} · {} · authentication unverified",
        describe(agent),
        result.unwrap_or_else(|e| e.to_string())
    )
}
