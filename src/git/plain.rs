//! What lg can show about a directory that is no checkout.
//!
//! Started in a plain folder — one a project is about to be cloned into, or
//! one that simply was never a repository — there is no history to compare
//! anything against, so everything in it is new. That is also how git
//! describes a file it has never seen, so the panels are handed the same
//! untracked entries `git status` would give them, and a file's diff is the
//! whole of its content.

use anyhow::Result;
use std::path::Path;

use super::nested::ignored_discovery_dir;
use super::status::FileEntry;

/// Enough files to work in a project folder, few enough that opening lg on a
/// home directory by accident cannot walk for minutes.
const MAX_NEW_FILES: usize = 5_000;

/// What the pane will take for a folder of new files before the rest is left
/// out. Past this much text there is nothing more to see.
const MAX_NEW_DIFF_BYTES: usize = 256 * 1024;

/// Every file in `dir`, reported as new.
///
/// Repositories inside `dir` are left out along with the build output every
/// discovery scan skips: their files belong to the checkout that owns them,
/// which the workspace pane lists separately.
pub fn new_file_entries(dir: &Path) -> Vec<FileEntry> {
    let mut paths = Vec::new();
    collect_new_files(dir, dir, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|path| FileEntry {
            path,
            x: '?',
            y: '?',
        })
        .collect()
}

/// Make `dir` a repository, on the branch lg's flows assume a project starts
/// on. This is the way out of everything above: once it succeeds the folder is
/// a checkout, and the next refresh shows it as one.
pub fn init_repository(dir: &Path) -> Result<String> {
    let configured_base = crate::preferences::base_branch();
    let text = super::run_combined_in_dir(dir, &["init", "-b", configured_base.as_str()])?;
    Ok(text
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("initialized repository")
        .trim()
        .to_owned())
}

/// One new file's content, as the diff that adding it would be.
pub fn new_file_diff(dir: &Path, path: &str) -> Result<String> {
    super::diff::added_file_diff(dir, path)
}

/// Every new file under `prefix` — the whole directory when it is empty — one
/// after another. Cut off once the pane has more than it can show, with a line
/// saying how much was left out.
pub fn new_files_diff(dir: &Path, prefix: &str) -> Result<String> {
    let root = if prefix.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(prefix)
    };
    let mut paths = Vec::new();
    collect_new_files(dir, &root, &mut paths);
    paths.sort();

    let mut text = String::new();
    let mut shown = 0usize;
    for path in &paths {
        if text.len() >= MAX_NEW_DIFF_BYTES {
            break;
        }
        let diff = super::diff::added_file_diff(dir, path)?;
        if diff.trim().is_empty() {
            shown += 1;
            continue;
        }
        text.push_str(&diff);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        shown += 1;
    }
    if shown < paths.len() {
        text.push_str(&format!(
            "\n... {} more file(s) not shown\n",
            paths.len() - shown
        ));
    }
    Ok(text)
}

/// Paths of the files under `dir`, relative to `root`.
fn collect_new_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    if out.len() >= MAX_NEW_FILES {
        return;
    }
    // A file was asked for rather than a directory: it is its own answer.
    if dir.is_file() {
        if let Some(path) = relative_path(root, dir) {
            out.push(path);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        // Asked of the entry rather than the path, so a symlinked directory
        // counts as the file it is instead of being walked into — which is
        // also how git reports one, and what keeps a cycle of them finite.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if !kind.is_dir() {
            if let Some(path) = relative_path(root, &path) {
                out.push(path);
                if out.len() >= MAX_NEW_FILES {
                    return;
                }
            }
            continue;
        }
        if ignored_discovery_dir(&path) || path.join(".git").exists() {
            continue;
        }
        dirs.push(path);
    }
    for dir in dirs {
        collect_new_files(root, &dir, out);
        if out.len() >= MAX_NEW_FILES {
            return;
        }
    }
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    Some(
        path.strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, text).expect("write file");
    }

    #[test]
    fn every_file_in_a_plain_directory_is_reported_as_new() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(&tmp.path().join("README.md"), "hello\n");
        write(&tmp.path().join("src/main.rs"), "fn main() {}\n");

        let entries = new_file_entries(tmp.path());

        let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert!(paths.contains(&"README.md"), "got {paths:?}");
        assert!(paths.contains(&"src/main.rs"), "got {paths:?}");
        assert!(
            entries.iter().all(|entry| entry.x == '?' && entry.y == '?'),
            "a directory with no history has nothing but new files"
        );
    }

    /// The repositories inside a workspace have a pane of their own, and their
    /// files belong to whichever of them is checked out.
    #[test]
    fn files_inside_a_nested_repository_are_left_to_that_repository() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(&tmp.path().join("notes.txt"), "mine\n");
        write(
            &tmp.path().join("project/.git/HEAD"),
            "ref: refs/heads/main\n",
        );
        write(&tmp.path().join("project/src/lib.rs"), "// theirs\n");
        write(&tmp.path().join("target/debug/build.log"), "noise\n");

        let paths: Vec<String> = new_file_entries(tmp.path())
            .into_iter()
            .map(|entry| entry.path)
            .collect();

        assert!(paths.contains(&"notes.txt".to_string()), "got {paths:?}");
        assert!(
            !paths.iter().any(|path| path.starts_with("project/")),
            "got {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path.starts_with("target/")),
            "got {paths:?}"
        );
    }

    #[test]
    fn initializing_a_plain_directory_makes_it_a_checkout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(&tmp.path().join("README.md"), "hello\n");

        init_repository(tmp.path()).expect("git init");

        assert!(
            super::super::repo_root_at(tmp.path()).is_some(),
            "the folder must be a checkout afterwards"
        );
    }

    #[test]
    fn a_new_file_shows_its_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(&tmp.path().join("src/main.rs"), "fn main() {}\n");

        let diff = new_file_diff(tmp.path(), "src/main.rs").expect("diff a new file");

        assert!(diff.contains("fn main() {}"), "got {diff}");
        assert!(diff.contains("src/main.rs"), "got {diff}");
    }

    #[test]
    fn a_folder_of_new_files_shows_all_of_them() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(&tmp.path().join("src/one.rs"), "const ONE: u8 = 1;\n");
        write(&tmp.path().join("src/two.rs"), "const TWO: u8 = 2;\n");
        write(&tmp.path().join("elsewhere.txt"), "not in src\n");

        let diff = new_files_diff(tmp.path(), "src").expect("diff a folder");

        assert!(diff.contains("const ONE"), "got {diff}");
        assert!(diff.contains("const TWO"), "got {diff}");
        assert!(!diff.contains("not in src"), "got {diff}");
    }
}
