//! Scratch directories shared by the `app` module's tests.
//!
//! Anything that reads or writes `~` must run against an overridden home, or the
//! tests would rewrite the developer's real `~/.claude` and `~/.terrarium`.

use std::path::PathBuf;

use crate::config::global::{TestHomeGuard, override_home_for_tests};

/// A fresh scratch directory under `temp/`. The caller removes it.
pub(super) fn temp_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("temp")
        .join(format!("test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Returns `(base, project_dir, home_guard)`. `base` contains both `home/` and
/// `project/`. Drop the guard when done; remove `base` for cleanup.
pub(super) fn temp_project() -> (PathBuf, PathBuf, TestHomeGuard) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("temp")
        .join(format!("test_{}", uuid::Uuid::new_v4()));
    let home = base.join("home");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let guard = override_home_for_tests(home);
    (base, project, guard)
}
