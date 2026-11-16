//! What `.gitattributes` says about the files in a diff.

use super::common::*;
use std::sync::mpsc;
use std::time::Duration;

/// A vendoring commit touches thousands of files; asking git about all of
/// them at once used to wait forever on a full pipe, and the diff pane with it.
#[test]
fn attributes_of_thousands_of_paths_come_back() {
    let dir = init_repo();
    fs::write(
        dir.path().join(".gitattributes"),
        "vendor/** linguist-vendored\n",
    )
    .unwrap();
    let paths: Vec<String> = (0..5000)
        .map(|n| format!("vendor/some/deeply/nested/package/file-{n}.js"))
        .collect();

    let _cwd = CwdGuard::new(dir.path());
    let (sender, receiver) = mpsc::channel();
    let asked = paths.clone();
    std::thread::spawn(move || {
        let _ = sender.send(lg::git::file_attrs(&asked));
    });
    let attrs = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("check-attr answered")
        .expect("check-attr succeeded");

    assert!(
        paths
            .iter()
            .all(|path| attrs.get(path).is_some_and(|attrs| attrs.vendored))
    );
}
