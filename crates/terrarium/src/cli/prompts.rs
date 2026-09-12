//! The interactive questions `init` and `run` ask when something was not passed
//! on the command line.
//!
//! Every prompt has a default, and a failed prompt falls back to it rather than
//! erroring: terrarium must still start when stdin is not a terminal.

use std::path::Path;

use crate::profile::presets;

/// Asks a yes/no question, falling back to `default` if the prompt cannot run.
pub(super) fn confirm(question: &str, default: bool) -> bool {
    inquire::Confirm::new(question)
        .with_default(default)
        .prompt()
        .unwrap_or(default)
}

pub(super) fn confirm_proxy() -> bool {
    confirm("Enable network proxy? (filters outbound domains)", true)
}

pub(super) fn confirm_allow_commit() -> bool {
    confirm("Allow git commit? (enables git_commit MCP tool)", false)
}

/// Prompts for one or more sandbox presets and returns the comma-separated spec.
pub(super) fn select_presets() -> anyhow::Result<String> {
    let choices: Vec<&str> = presets::known_presets().to_vec();
    let selected = inquire::MultiSelect::new("Select sandbox preset(s):", choices)
        .with_help_message("Space to toggle (several languages allowed), Enter to confirm")
        .prompt()?;
    if selected.is_empty() {
        return Ok("none".to_string());
    }
    Ok(selected.join(","))
}

/// The preset for `project_root`: detected from its files, or chosen interactively.
pub(super) fn detect_or_select_presets(project_root: &Path) -> anyhow::Result<String> {
    match crate::app::detect_project_preset_spec(project_root) {
        Some(detected) => {
            println!("terrarium: detected preset '{detected}'");
            Ok(detected)
        }
        None => select_presets(),
    }
}

/// Asks whether to install the project's skill files, if any are missing or stale.
pub(super) fn prompt_install_skills(project_root: &Path) -> bool {
    let names = crate::app::skill_prompt_names(project_root);
    if names.is_empty() || crate::app::skill_is_current(project_root) {
        return false;
    }
    confirm(
        &format!(
            "Install {} skill(s) to ./.claude/skills/?",
            names.join(", ")
        ),
        true,
    )
}
