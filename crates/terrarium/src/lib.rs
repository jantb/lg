//! `terrarium` — runs your dev tools inside a macOS Seatbelt sandbox.
//!
//! The modules mirror what the tool does: [`cli`] parses and dispatches commands,
//! [`app`] configures a project, [`profile`] turns that configuration into a
//! sandbox policy, [`sandbox`] runs a command under it, [`mcp`] serves the tools
//! the sandboxed session calls back into, and [`proxy`] filters its network.

mod app;
mod cli;
mod config;
mod error;
mod mcp;
mod profile;
mod proxy;
mod sandbox;
#[cfg(test)]
mod test_support;
mod tui;

/// Run the bundled CLI before the host starts its terminal UI.
pub fn run_cli(args: impl IntoIterator<Item = std::ffi::OsString>) -> anyhow::Result<()> {
    use clap::Parser;
    let cli = cli::Cli::parse_from(args);
    tokio::runtime::Runtime::new()?.block_on(cli::dispatch(cli.command))
}

/// Create only the private profile; leave repository guidance and agent files intact.
pub fn initialize(project: &std::path::Path, preset: &str) -> anyhow::Result<()> {
    if config::project::profile_path(project).exists() {
        anyhow::bail!("a sandbox profile already exists; edit it instead");
    }
    if !profile::presets::is_known_preset(preset) {
        anyhow::bail!("unknown sandbox preset: {preset}");
    }
    let mut profile = config::project::ProjectProfile::new(
        project.file_name().unwrap_or_default().to_string_lossy(),
        Some(preset.into()),
    );
    profile.allow_commit = true;
    profile.params.insert(
        "PROJECT_ROOT".into(),
        project.canonicalize()?.display().to_string(),
    );
    profile.save(project)?;
    Ok(())
}

pub fn validate(project: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let workspace = config::workspace::WorkspaceConfig::load(project).ok();
    let (policy, _) = sandbox::runner::build_sbpl(project, workspace.as_ref(), &[])?;
    let mut command = std::process::Command::new("/usr/bin/true");
    // SAFETY: only the sandbox setup runs in the forked child before exec.
    unsafe {
        command.pre_exec(move || sandbox::seatbelt::apply(&policy));
    }
    let result = command.output()?;
    if !result.status.success() {
        anyhow::bail!(
            "sandbox validation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(())
}
pub fn check_profile(text: &str) -> anyhow::Result<()> {
    let _: config::project::ProjectProfile = toml::from_str(text)?;
    Ok(())
}
