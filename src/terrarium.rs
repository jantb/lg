//! Sandbox profiles for sessions.
//!
//! terrarium confines a command to one project directory. That is exactly what
//! a worktree session wants: several claude sessions on the same repository,
//! each able to touch only its own checkout. A worktree needs one rule beyond
//! the usual profile — its `.git` is a pointer into the main repository, and
//! commits write objects and refs there — so lg derives the worktree's profile
//! from the repository's own and adds that rule.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Directory terrarium keeps a project's profile in: the resolved project path
/// with the separators folded into dashes. Resolving matters — terrarium looks
/// the profile up by the real path, and on macOS `/tmp` and `/var` are symlinks,
/// so an unresolved path would name a profile that is never found.
pub fn profile_slug(project: &Path) -> String {
    resolve(project)
        .to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-")
}

/// The path with symlinks resolved, or the path itself when it does not exist
/// yet and so cannot be resolved.
pub fn resolve(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Where terrarium looks for a project's profile, if this machine has a home.
pub fn profile_path(project: &Path) -> Option<PathBuf> {
    Some(profile_path_in(&home_dir()?, project))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn profile_path_in(home: &Path, project: &Path) -> PathBuf {
    home.join(".terrarium/projects")
        .join(profile_slug(project))
        .join("profile.toml")
}

pub fn has_profile(project: &Path) -> bool {
    profile_path(project).is_some_and(|path| path.is_file())
}

/// Make sure `worktree` can be run sandboxed, deriving its profile from
/// `main_worktree`'s when it has none yet. Returns what was done, or `None` when
/// the profile was already in place.
pub fn ensure_profile(
    worktree: &Path,
    main_worktree: &Path,
    shared_git_dir: &Path,
) -> Result<Option<String>> {
    let home = home_dir().context("HOME is not set, so terrarium has no profiles")?;
    ensure_profile_in(&home, worktree, main_worktree, shared_git_dir)
}

/// The body of `ensure_profile`, with the home directory passed in so it can be
/// exercised without touching the environment.
fn ensure_profile_in(
    home: &Path,
    worktree: &Path,
    main_worktree: &Path,
    shared_git_dir: &Path,
) -> Result<Option<String>> {
    let path = profile_path_in(home, worktree);

    // A checkout that contains its own git directory needs no extra rule.
    let resolved = resolve(worktree);
    let git_rule = (!resolve(shared_git_dir).starts_with(&resolved)).then_some(shared_git_dir);

    if path.is_file() {
        let existing =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let Some(git_dir) = git_rule else {
            return Ok(None);
        };
        let Some(updated) = with_git_rule(&existing, git_dir) else {
            return Ok(None);
        };
        std::fs::write(&path, updated).with_context(|| format!("write {}", path.display()))?;
        return Ok(Some(format!(
            "added the shared git directory to the sandbox profile for {}",
            name_of(worktree)
        )));
    }

    let template_path = Some(profile_path_in(home, main_worktree))
        .filter(|path| path.is_file())
        .with_context(|| {
            format!(
                "{} has no terrarium profile — run `terrarium init` there, or start the session unsandboxed",
                main_worktree.display()
            )
        })?;
    let template = std::fs::read_to_string(&template_path)
        .with_context(|| format!("read {}", template_path.display()))?;

    let mut profile = retarget(
        &template,
        &profile_name(main_worktree, worktree),
        &resolve(worktree),
    );
    if let Some(git_dir) = git_rule
        && let Some(updated) = with_git_rule(&profile, git_dir)
    {
        profile = updated;
    }

    let dir = path.parent().context("profile path has no directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    std::fs::write(&path, profile).with_context(|| format!("write {}", path.display()))?;
    Ok(Some(format!(
        "wrote a sandbox profile for {}",
        name_of(worktree)
    )))
}

/// Point a copy of the repository's profile at the worktree instead.
fn retarget(template: &str, name: &str, project_root: &Path) -> String {
    let root = project_root.to_string_lossy();
    let mut out = String::with_capacity(template.len() + 64);
    for line in template.lines() {
        if line.starts_with("name = ") {
            out.push_str(&format!("name = \"{name}\"\n"));
        } else if line.starts_with("PROJECT_ROOT = ") {
            out.push_str(&format!("PROJECT_ROOT = \"{root}\"\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Add the rule that lets a worktree reach the repository's git directory.
/// Returns `None` when the profile already allows it.
fn with_git_rule(profile: &str, git_dir: &Path) -> Option<String> {
    let git_dir = resolve(git_dir);
    let git_dir = git_dir.to_string_lossy();
    let value = format!("path_value = \"{git_dir}\"");
    if profile.contains(&value) {
        return None;
    }

    let rule = format!(
        "\n[[rules]]\n\
         action = \"allow\"\n\
         operation = \"file-read* file-write*\"\n\
         path_type = \"subpath\"\n\
         path_value = \"{git_dir}\"\n\
         comment = \"worktree shares the repository's git directory\"\n\
         source = \"manual\"\n"
    );

    // An empty `rules` key and an appended array-of-tables cannot both be
    // present, so replace the empty one where it exists.
    if let Some(rest) = profile.strip_prefix("rules = []\n") {
        return Some(format!("{rest}{rule}"));
    }
    if let Some(at) = profile.find("\nrules = []\n") {
        let (before, after) = profile.split_at(at);
        let after = after.trim_start_matches("\nrules = []\n");
        return Some(format!("{before}\n{after}{rule}"));
    }
    Some(format!("{profile}{rule}"))
}

/// `repo@worktree`, so `terrarium list` says which checkout an instance is in.
fn profile_name(main_worktree: &Path, worktree: &Path) -> String {
    format!("{}@{}", name_of(main_worktree), name_of(worktree))
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "name = \"lg\"\n\
        preset = \"rust\"\n\
        rules = []\n\
        allow_commit = false\n\
        \n\
        [params]\n\
        PROJECT_ROOT = \"/dev/lg\"\n\
        \n\
        [network]\n\
        proxy_enabled = false\n\
        allowed_domains = []\n";

    #[test]
    fn the_slug_matches_terrariums_own_naming() {
        assert_eq!(
            profile_slug(Path::new("/Users/jantb/dev/priv/lg")),
            "Users-jantb-dev-priv-lg"
        );
        assert_eq!(
            profile_slug(Path::new("/private/tmp/shg")),
            "private-tmp-shg"
        );
    }

    #[test]
    fn a_retargeted_profile_keeps_everything_but_the_project() {
        let profile = retarget(TEMPLATE, "lg@feat-x", Path::new("/dev/lg.worktrees/feat-x"));
        assert!(profile.contains("name = \"lg@feat-x\""));
        assert!(profile.contains("PROJECT_ROOT = \"/dev/lg.worktrees/feat-x\""));
        assert!(profile.contains("preset = \"rust\""), "{profile}");
        assert!(profile.contains("proxy_enabled = false"), "{profile}");
        assert!(!profile.contains("/dev/lg\""), "the old root is gone");
    }

    #[test]
    fn the_git_rule_replaces_an_empty_rules_list() {
        let profile = with_git_rule(TEMPLATE, Path::new("/dev/lg/.git")).expect("rule added");
        assert!(
            !profile.contains("rules = []"),
            "an empty list and a rule table cannot coexist: {profile}"
        );
        assert_eq!(profile.matches("[[rules]]").count(), 1);
        assert!(profile.contains("path_value = \"/dev/lg/.git\""));
        assert!(profile.contains("preset = \"rust\""), "{profile}");
    }

    #[test]
    fn a_rule_is_appended_when_others_are_already_there() {
        let existing = format!(
            "{}\n[[rules]]\naction = \"allow\"\npath_value = \"/elsewhere\"\n",
            TEMPLATE.replace("rules = []\n", "")
        );
        let profile = with_git_rule(&existing, Path::new("/dev/lg/.git")).expect("rule added");
        assert_eq!(profile.matches("[[rules]]").count(), 2);
        assert!(profile.contains("path_value = \"/elsewhere\""));
        assert!(profile.contains("path_value = \"/dev/lg/.git\""));
    }

    #[test]
    fn a_profile_that_already_allows_the_git_directory_is_left_alone() {
        let profile = with_git_rule(TEMPLATE, Path::new("/dev/lg/.git")).expect("rule added");
        assert!(with_git_rule(&profile, Path::new("/dev/lg/.git")).is_none());
    }

    #[test]
    fn the_profile_name_says_which_checkout_it_is() {
        assert_eq!(
            profile_name(Path::new("/dev/lg"), Path::new("/dev/lg.worktrees/feat-x")),
            "lg@feat-x"
        );
    }

    #[test]
    fn a_worktree_profile_is_derived_from_the_repositorys_own() {
        let home = tempfile::tempdir().expect("tempdir");
        let main = Path::new("/dev/lg");
        let worktree = Path::new("/dev/lg.worktrees/feat-x");

        let main_profile = profile_path_in(home.path(), main);
        std::fs::create_dir_all(main_profile.parent().unwrap()).unwrap();
        std::fs::write(&main_profile, TEMPLATE).unwrap();

        let note = ensure_profile_in(home.path(), worktree, main, Path::new("/dev/lg/.git"))
            .expect("derive profile")
            .expect("something was written");
        assert!(note.contains("feat-x"), "{note}");

        let written =
            std::fs::read_to_string(profile_path_in(home.path(), worktree)).expect("read");
        assert!(written.contains("PROJECT_ROOT = \"/dev/lg.worktrees/feat-x\""));
        assert!(written.contains("path_value = \"/dev/lg/.git\""));
        assert!(written.contains("preset = \"rust\""));

        // Running again changes nothing.
        assert!(
            ensure_profile_in(home.path(), worktree, main, Path::new("/dev/lg/.git"))
                .expect("second run")
                .is_none()
        );
    }

    #[test]
    fn a_repository_with_no_profile_says_so_instead_of_guessing() {
        let home = tempfile::tempdir().expect("tempdir");
        let err = ensure_profile_in(
            home.path(),
            Path::new("/dev/lg.worktrees/feat-x"),
            Path::new("/dev/lg"),
            Path::new("/dev/lg/.git"),
        )
        .expect_err("there is nothing to derive from");
        let message = format!("{err:#}");
        assert!(message.contains("terrarium init"), "{message}");
    }

    #[test]
    fn a_checkout_holding_its_own_git_directory_needs_no_extra_rule() {
        let home = tempfile::tempdir().expect("tempdir");
        let main = Path::new("/dev/lg");
        let profile = profile_path_in(home.path(), main);
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(&profile, TEMPLATE).unwrap();

        assert!(
            ensure_profile_in(home.path(), main, main, Path::new("/dev/lg/.git"))
                .expect("already sandboxed")
                .is_none()
        );
        let written = std::fs::read_to_string(&profile).expect("read");
        assert_eq!(written, TEMPLATE, "the repository's profile is untouched");
    }
}
