//! Everything terrarium writes into a project, and how it is taken back out.
//!
//! A terrarium project is configured in five places, one module each: the
//! private profile under `~/.terrarium` ([`init`]), the managed block in
//! `CLAUDE.local.md` ([`claude_md`]), the skill files under `.claude/skills`
//! ([`skills`]), the MCP server entry in `.mcp.json` ([`mcp_registration`]), and
//! Claude's own runtime settings ([`claude_settings`]). Each of those adds
//! [`gitignore`] entries for what it wrote, and [`clear`] removes the lot.

mod claude_md;
mod claude_settings;
mod clear;
mod detect;
mod gitignore;
mod init;
mod mcp_registration;
mod skills;
mod status;
#[cfg(test)]
mod testing;

pub use claude_md::configure_workspace;
pub use claude_settings::ensure_bypass_permissions_accepted;
pub use clear::clear_project;
pub use detect::{detect_project_preset_spec, detect_project_preset_spec_deep};
pub use init::{discover_workspace_project_dirs, init_project, init_workspace};
pub use mcp_registration::{
    register_mcp_server_http, unregister_mcp_server_http, verify_registered_mcp_server_http,
};
pub use skills::{skill_is_current, skill_prompt_names};
pub use status::{print_instances, validate_profile};

use std::path::Path;

use crate::error::Result;

/// Markers delimiting the block terrarium owns inside a user's `CLAUDE.local.md`.
/// Everything outside them belongs to the user and is preserved.
const TERRARIUM_CLAUDE_BLOCK_START: &str = "<!-- terrarium-managed:start -->";
const TERRARIUM_CLAUDE_BLOCK_END: &str = "<!-- terrarium-managed:end -->";

/// Version-independent prefix of `TERRARIUM_VERSION_TAG`, used to recognize files
/// terrarium wrote regardless of which release wrote them.
const TERRARIUM_SKILL_MARKER: &str = "<!-- terrarium-version: ";
const TERRARIUM_VERSION_TAG: &str = concat!(
    "<!-- terrarium-version: ",
    env!("CARGO_PKG_VERSION"),
    " -->"
);

/// Brings a project's Claude Code configuration up to date: the managed
/// `CLAUDE.local.md` block always, the skill files only when asked.
pub fn configure_claude(project_root: &Path, install_skill: bool) -> Result<()> {
    claude_md::ensure_project_claude_md(project_root)?;
    if install_skill {
        skills::ensure_skill_file(project_root)?;
    }
    Ok(())
}
