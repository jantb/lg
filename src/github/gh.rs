//! Running the gh command, and saying on one line what went wrong when it
//! fails.

use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};

/// A flag and its value as one argument. Written apart, gh takes whatever
/// follows a flag as its value, but a value that starts with a dash — a title
/// such as `--draft` — reads as a flag of its own.
pub(super) fn flag_value(flag: &str, value: &str) -> String {
    format!("{flag}={value}")
}

pub(super) fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rfind(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
}

/// A `gh` invocation in `dir`, or in the checkout this thread's git commands
/// run in.
pub(super) fn command(dir: Option<&Path>, args: &[&str]) -> Command {
    let mut command = Command::new("gh");
    command
        .args(args)
        .stdin(Stdio::null())
        // Never wait on a question nobody is there to answer, and keep the
        // output free of colour, spinners and upgrade notices so it parses.
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_SPINNER_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("GH_PAGER", "cat")
        // Clone and checkout run git underneath; a credential prompt there
        // would hang just the same.
        .env("GIT_TERMINAL_PROMPT", "0");
    let dir = dir
        .map(Path::to_path_buf)
        .or_else(crate::git::repo_dir)
        .filter(|dir| dir.is_dir());
    crate::git::batch_ssh(&mut command, dir.as_deref());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command
}

pub(super) fn spawn_failure(err: std::io::Error) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::NotFound {
        anyhow::anyhow!(
            "the GitHub CLI is not installed: get gh from https://cli.github.com, then run `gh auth login`"
        )
    } else {
        anyhow::anyhow!("failed to start gh: {err}")
    }
}

/// What a failed `gh` call said, on one line: the status bar has room for
/// that and no more, and gh spreads its errors over several.
pub(super) fn failure(args: &[&str], stdout: &str, stderr: &str) -> anyhow::Error {
    let said = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let said = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let what = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
    anyhow::anyhow!("gh {what} failed: {said}")
}

/// Run `gh` and hand back what it printed on stdout.
pub(super) fn run(args: &[&str]) -> Result<String> {
    let out = command(None, args).output().map_err(spawn_failure)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(failure(
            args,
            &stdout,
            &String::from_utf8_lossy(&out.stderr),
        ))
    }
}

/// Run `gh` and hand back everything it printed. The commands that drive git
/// report on stderr, and that report is the result.
pub(super) fn run_combined(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let out = command(dir, args).output().map_err(spawn_failure)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(failure(args, &stdout, &stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(command: &Command, name: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new(name))
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    /// `gh pr checkout` and `gh repo clone` run git over ssh behind the TUI,
    /// where a passphrase or host-key prompt would wait for good.
    #[test]
    fn ssh_under_gh_never_prompts_unless_the_user_chose_how_it_runs() {
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        if set("GIT_SSH_COMMAND") || set("GIT_SSH") {
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        if git(&["config", "--get", "core.sshCommand"])
            .status
            .success()
        {
            return;
        }

        let checkout = command(Some(repo.path()), &["pr", "checkout", "1"]);
        let ssh = env_of(&checkout, "GIT_SSH_COMMAND").unwrap_or_default();
        assert!(ssh.contains("BatchMode=yes"), "{ssh:?}");

        assert!(
            git(&["config", "core.sshCommand", "ssh -i /tmp/key"])
                .status
                .success()
        );
        let checkout = command(Some(repo.path()), &["pr", "checkout", "1"]);
        assert_eq!(env_of(&checkout, "GIT_SSH_COMMAND"), None);
    }
}
