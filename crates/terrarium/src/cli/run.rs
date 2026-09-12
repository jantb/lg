//! `terrarium run` — launching a command in the sandbox, initializing first if
//! the project has never been set up.

use std::path::PathBuf;

use crate::config::project::ProjectProfile;
use crate::config::workspace::WorkspaceConfig;
use crate::error::TerrariumError;
use crate::sandbox::runner;

use super::prompts::{
    confirm_allow_commit, confirm_proxy, detect_or_select_presets, prompt_install_skills,
};

pub(super) async fn run(project: String, command: Vec<String>) -> anyhow::Result<()> {
    // The path the user named and the path it resolves to are both needed: the
    // sandbox matches paths lexically, so a symlinked root needs rules for both.
    let requested_project_root = lexical_absolute_path(&project)?;
    let project_root = requested_project_root
        .canonicalize()
        .unwrap_or_else(|_| requested_project_root.clone());
    let project_root_aliases = if requested_project_root != project_root {
        vec![requested_project_root]
    } else {
        vec![]
    };

    if !is_initialized(&project_root) {
        initialize_before_run(&project_root)?;
    }

    runner::run(runner::RunOptions {
        project_root,
        project_root_aliases,
        command: if command.is_empty() {
            None
        } else {
            Some(command)
        },
    })
    .await?;

    Ok(())
}

/// True when the root has either a project profile or a workspace config.
fn is_initialized(project_root: &std::path::Path) -> bool {
    let no_profile = matches!(
        ProjectProfile::load(project_root),
        Err(TerrariumError::ProfileNotFound { .. })
    );
    let no_workspace = matches!(
        WorkspaceConfig::load(project_root),
        Err(TerrariumError::WorkspaceNotFound { .. })
    );
    !(no_profile && no_workspace)
}

/// Runs the `init` flow inline, so `terrarium run` works in a fresh project.
fn initialize_before_run(project_root: &std::path::Path) -> anyhow::Result<()> {
    eprintln!("terrarium: no profile found for this project — running init first");
    let preset = detect_or_select_presets(project_root)?;
    let proxy_enabled = confirm_proxy();
    let allow_commit = confirm_allow_commit();
    crate::app::init_project(project_root, &preset, proxy_enabled, allow_commit)?;
    let install_skill = prompt_install_skills(project_root);
    crate::app::configure_claude(project_root, install_skill)?;
    Ok(())
}

/// Makes `path` absolute without resolving symlinks.
///
/// `PWD` is preferred over `current_dir()` because the shell sets it to the path
/// the user actually typed, symlinks intact — which is the path the sandbox has to
/// allow.
fn lexical_absolute_path(path: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return Ok(path);
    }

    if let Some(pwd) = std::env::var_os("PWD") {
        let pwd = PathBuf::from(pwd);
        if pwd.is_absolute() {
            return Ok(pwd.join(path));
        }
    }

    Ok(std::env::current_dir()?.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_are_returned_unchanged() {
        assert_eq!(
            lexical_absolute_path("/tmp/project").unwrap(),
            PathBuf::from("/tmp/project")
        );
    }

    /// A relative path is joined onto `PWD`, keeping any symlink the user typed.
    #[test]
    fn relative_paths_are_joined_onto_pwd() {
        let original = std::env::var_os("PWD");
        unsafe { std::env::set_var("PWD", "/tmp/link-to-project") };

        let resolved = lexical_absolute_path("sub").unwrap();

        match original {
            Some(value) => unsafe { std::env::set_var("PWD", value) },
            None => unsafe { std::env::remove_var("PWD") },
        }
        assert_eq!(resolved, PathBuf::from("/tmp/link-to-project/sub"));
    }
}
