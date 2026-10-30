//! Cloning a repository, and opening GitHub pages in the browser.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::gh::run_combined;

/// Clone `repo` — `owner/name` or a URL — into `dir`.
pub fn clone(repo: &str, dir: &Path) -> Result<String> {
    // gh would read a leading dash as an option, and no repository has one.
    if repo.starts_with('-') {
        anyhow::bail!("not a repository: {repo}");
    }
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    let target = dir.to_string_lossy().into_owned();
    run_combined(dir.parent(), &["repo", "clone", repo, &target])?;
    Ok(format!("cloned {repo} into {}", dir.display()))
}

/// Open `url` in the browser. Not waited on beyond starting it.
pub fn open_in_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start the browser")?;
    // Reaped off the UI thread so the opener does not linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The directory a clone of `repo` gets under `parent`: the repository's own
/// name, as `gh repo clone` would pick it.
pub fn clone_dir(parent: &Path, repo: &str) -> Option<PathBuf> {
    let name = repo
        .trim()
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()?
        .trim_end_matches(".git");
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(parent.join(name))
}
