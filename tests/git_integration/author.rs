//! Who commits in a checkout, as the author settings show it.

use super::common::*;

/// The repository's own author is what its config file says, the last value
/// winning; a value another file brings in through an include is in effect
/// but is not the repository's own, so clearing the repository author could
/// not remove it.
#[test]
fn the_repository_author_is_what_its_own_config_file_sets() {
    let dir = init_repo();
    let included = dir.path().join("included.gitconfig");
    fs::write(&included, "[user]\n\temail = included@example.com\n").unwrap();
    git_ok(dir.path(), &["config", "user.name", "First"]);
    git_ok(dir.path(), &["config", "--add", "user.name", "Second"]);
    git_ok(
        dir.path(),
        &["config", "include.path", included.to_str().unwrap()],
    );
    git_ok(dir.path(), &["config", "--unset-all", "user.email"]);

    let author = lg::git::with_repo(dir.path(), lg::git::author_config).unwrap();

    assert_eq!(author.name.as_deref(), Some("Second"));
    assert_eq!(author.local_name.as_deref(), Some("Second"));
    assert_eq!(author.email.as_deref(), Some("included@example.com"));
    assert_eq!(author.local_email, None);
}
