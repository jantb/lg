//! `terrarium init` — creating a project or workspace profile.

use crate::config::workspace::WorkspaceProject;

use super::prompts::{
    confirm, confirm_allow_commit, confirm_proxy, detect_or_select_presets, prompt_install_skills,
};

pub(super) fn run(preset: Option<String>, workspace: bool) -> anyhow::Result<()> {
    let use_workspace =
        workspace || confirm("Initialize as a workspace? (multiple sub-projects)", false);
    if use_workspace {
        return init_workspace();
    }
    init_single_project(preset)
}

fn init_single_project(preset: Option<String>) -> anyhow::Result<()> {
    let project_root = std::env::current_dir()?;
    let preset = match preset {
        Some(p) => p,
        None => detect_or_select_presets(&project_root)?,
    };
    let proxy_enabled = confirm_proxy();
    let allow_commit = confirm_allow_commit();

    crate::app::init_project(&project_root, &preset, proxy_enabled, allow_commit)?;
    println!("terrarium: built-in MCP server will be registered over HTTP during `terrarium run`");

    let install_skill = prompt_install_skills(&project_root);
    crate::app::configure_claude(&project_root, install_skill)?;
    Ok(())
}

fn init_workspace() -> anyhow::Result<()> {
    let project_root = std::env::current_dir()?;
    let proxy_enabled = confirm_proxy();
    let allow_commit = confirm_allow_commit();

    let projects = select_workspace_projects()?;
    if projects.is_empty() {
        println!("terrarium: no projects selected — workspace not created");
        return Ok(());
    }

    crate::app::init_workspace(&project_root, &projects, proxy_enabled, allow_commit)?;
    Ok(())
}

/// Discovers sub-project candidates, asks which to include, and infers each one's
/// preset from its project files.
fn select_workspace_projects() -> anyhow::Result<Vec<WorkspaceProject>> {
    let project_root = std::env::current_dir()?;
    let subdirs = crate::app::discover_workspace_project_dirs(&project_root)?;

    // No sub-dirs: treat the whole root as a single project.
    if subdirs.is_empty() {
        return Ok(vec![WorkspaceProject {
            path: ".".to_string(),
            preset: detected_preset(&project_root, "."),
        }]);
    }

    let choices: Vec<&str> = subdirs.iter().map(|s| s.as_str()).collect();
    let selected_dirs = inquire::MultiSelect::new("Select sub-projects to include:", choices)
        .with_help_message("Space to toggle, Enter to confirm")
        .prompt()?;

    Ok(selected_dirs
        .into_iter()
        .map(|dir| WorkspaceProject {
            preset: detected_preset(&project_root.join(dir), dir),
            path: dir.to_string(),
        })
        .collect())
}

/// A sub-project's preset is never prompted for — with many of them that would be
/// a wall of questions — so an unrecognized directory gets `none`.
///
/// Detection reaches into the sub-project's module directories: a workspace
/// entry stands for a whole repository, and a monorepo declares its languages
/// below its root rather than at it.
fn detected_preset(dir: &std::path::Path, label: &str) -> String {
    let selected =
        crate::app::detect_project_preset_spec_deep(dir).unwrap_or_else(|| "none".to_string());
    println!("terrarium: detected preset '{selected}' for '{label}'");
    selected
}
