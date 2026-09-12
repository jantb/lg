//! The command-line surface: the argument definitions and the handler each
//! subcommand dispatches to.
//!
//! Handlers live in their own module when they do more than call one function
//! ([`run`], [`init`], [`clear`]); the rest are one-liners in [`dispatch`].

mod clear;
mod init;
mod prompts;
mod run;

use clap::{Parser, Subcommand};

use crate::profile::presets;

/// Validates and normalizes a comma-separated preset spec (e.g. `node,kotlin,rust`).
fn parse_preset_spec(value: &str) -> std::result::Result<String, String> {
    if !presets::is_known_preset(value) {
        return Err(format!(
            "unknown preset in '{value}'. Available presets: {}",
            presets::known_presets().join(", ")
        ));
    }
    Ok(presets::normalize_preset(value))
}

#[derive(Debug, Parser)]
#[command(
    name = "terrarium",
    version,
    about = "A controlled environment where your code tools run safely",
    long_about = "terrarium wraps commands (like Claude Code) in a macOS Seatbelt sandbox — a sealed,\n\
                  transparent enclosure for your dev tools. It generates SBPL profiles from simple\n\
                  TOML configs, tracks running instances, and exposes build/git tools via MCP.",
    after_help = "\x1b[1mExamples:\x1b[0m\n  \
                  terrarium init --preset rust   Initialize a Rust sandbox profile\n  \
                  terrarium init --preset node,kotlin,rust\n                                 Initialize a multi-language sandbox profile\n  \
                  terrarium run                  Launch the default command in a sandbox\n  \
                  terrarium list                 Show running sandboxed instances\n  \
                  terrarium kill <ID>            Stop a sandboxed instance\n  \
                  terrarium profile validate     Test the generated sandbox profile\n\n\
                  \x1b[1mQuick start:\x1b[0m\n  \
                  cd my-project && terrarium init --preset rust && terrarium run"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Launch a command inside a sandbox
    #[command(
        long_about = "Resolves the sandbox profile for the project, applies it via sandbox_init(3),\n\
                            and spawns the configured command (default: claude). The instance is registered\n\
                            so it can be managed via `terrarium list` and `terrarium kill`."
    )]
    Run {
        /// Project directory
        #[arg(long, default_value = ".", value_name = "DIR")]
        project: String,

        /// Command to run inside the sandbox (overrides global config default).
        /// Use `--` to separate terrarium flags from the command, e.g. `terrarium run -- bash`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// List running sandboxed instances
    List,

    /// Initialize the sandbox profile for the current directory
    #[command(
        long_about = "Creates a private profile under ~/.terrarium containing the chosen preset(s).\n\
                            Several presets can be combined with commas for multi-language projects,\n\
                            e.g. --preset node,kotlin,rust.\n\
                            If --preset is omitted, presets are detected from the project files, or\n\
                            you will be prompted to choose interactively.\n\n\
                            Available presets:\n  \
                            rust     Project workspace + Claude runtime baseline\n  \
                            kotlin   Project workspace + Maven local repository\n  \
                            swift    Project workspace + Xcode/SPM caches\n  \
                            node     Project workspace + npm/pnpm/yarn/bun caches\n  \
                            python   Project workspace + pip/uv/Poetry caches\n  \
                            none     Minimal sandbox with no extra rules\n\n\
                            Each preset also opens its language's package registry in the\n\
                            outbound proxy allowlist (crates.io, Maven Central, npm, PyPI)."
    )]
    Init {
        /// Sandbox preset(s) to use, comma-separated (rust, kotlin, swift, node, python, none).
        /// Detected or prompted if omitted.
        #[arg(long, value_parser = parse_preset_spec)]
        preset: Option<String>,
        /// Initialize as a multi-project workspace (auto-detects sub-projects)
        #[arg(long)]
        workspace: bool,
    },

    /// Remove everything terrarium configured for the current directory
    #[command(
        long_about = "Removes the private profile (or workspace config) under ~/.terrarium, the\n\
                            managed CLAUDE.local.md block, the registered MCP server entry, the\n\
                            terrarium-owned keys in .claude/settings.local.json, and the gitignore\n\
                            entries terrarium added. Unrelated settings are preserved."
    )]
    Clear {
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Manage the sandbox profile
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },

    /// Kill a sandboxed instance
    Kill {
        /// Instance ID (from `terrarium list`)
        instance_id: String,
    },

    /// Start an MCP stdio server exposing sandboxed cargo tools
    Mcp {
        #[arg(long, default_value = ".", value_name = "DIR")]
        project: String,
    },

    /// Manage domain whitelist and view blocked suggestions
    Tui {
        /// Project directory
        #[arg(long, default_value = ".", value_name = "DIR")]
        project: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProfileAction {
    /// Compile and test-run the sandbox profile against /usr/bin/true
    Validate,
}

/// Routes a parsed command to its handler.
pub async fn dispatch(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Run { project, command } => run::run(project, command).await,
        Command::Init { preset, workspace } => init::run(preset, workspace),
        Command::Clear { yes } => clear::run(yes),
        Command::List => Ok(crate::app::print_instances()?),
        Command::Profile {
            action: ProfileAction::Validate,
        } => Ok(crate::app::validate_profile(&std::env::current_dir()?)?),
        Command::Kill { instance_id } => Ok(crate::sandbox::runner::kill(&instance_id).await?),
        Command::Mcp { project } => Ok(crate::mcp::serve(resolve_project_dir(&project)).await?),
        Command::Tui { project } => Ok(crate::tui::run(&resolve_project_dir(&project))?),
    }
}

/// Canonicalizes a project directory, falling back to the path as given when it
/// cannot be resolved.
fn resolve_project_dir(project: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(project)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(project))
}
