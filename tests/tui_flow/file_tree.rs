//! The files panel as a tree: folders, compacted chains and collapsing.

use super::common::*;

#[test]
fn tree_flat_files_emit_all_changes_plus_each_file() {
    let files = vec![
        FileEntry {
            path: "a.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "b.rs".into(),
            x: 'A',
            y: ' ',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());
    assert_eq!(rows.len(), 3);
    assert!(matches!(rows[0].kind, TreeKind::AllChanges));
    assert!(matches!(rows[1].kind, TreeKind::File { entry_idx: 0 }));
    assert!(matches!(rows[2].kind, TreeKind::File { entry_idx: 1 }));
}

#[test]
fn tree_groups_nested_files_under_folders() {
    let files = vec![
        FileEntry {
            path: "src/lib.rs".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/util/mod.rs".into(),
            x: 'A',
            y: ' ',
        },
        FileEntry {
            path: "README.md".into(),
            x: ' ',
            y: 'M',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());
    // Interleaved alphabetical at each depth:
    //   root: README.md ('r') < src/ ('s')
    //   src/: lib.rs ('l') < util/ ('u')
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[1].path, "README.md");
    assert_eq!(rows[2].path, "src");
    match rows[2].kind {
        TreeKind::Folder {
            expanded,
            total,
            staged,
        } => {
            assert!(expanded);
            assert_eq!(total, 2);
            assert_eq!(staged, 2);
        }
        _ => panic!("expected folder row"),
    }
    assert_eq!(rows[3].path, "src/lib.rs");
    assert_eq!(rows[4].path, "src/util");
    assert_eq!(rows[5].path, "src/util/mod.rs");
}

#[test]
fn tree_compacts_single_subdir_chains() {
    let files = vec![
        FileEntry {
            path: "src/main/kotlin/org/example/inventory/App.kt".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/main/kotlin/org/example/inventory/Service.kt".into(),
            x: ' ',
            y: 'M',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());

    assert_eq!(rows[1].path, "src/main/kotlin/org/example/inventory");
    assert_eq!(rows[1].label, "src/main/kotlin/org/example/inventory");
    assert_eq!(rows[2].path, "src/main/kotlin/org/example/inventory/App.kt");
    assert_eq!(
        rows[3].path,
        "src/main/kotlin/org/example/inventory/Service.kt"
    );
}

#[test]
fn tree_renders_untracked_directory_with_name() {
    let files = vec![FileEntry {
        path: "src/main/kotlin/no/alv/exchange_rate/model/response/".into(),
        x: '?',
        y: '?',
    }];

    let rows = build_tree_rows(&files, &HashSet::new());

    assert!(
        rows.iter().any(|row| row.label == "response/"),
        "untracked directory row should keep its display name: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.label.is_empty()),
        "tree should not emit an empty file label: {rows:?}"
    );
}

#[test]
fn tree_collapsed_folder_hides_children() {
    let files = vec![
        FileEntry {
            path: "src/a.rs".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/b.rs".into(),
            x: 'A',
            y: ' ',
        },
    ];
    let mut collapsed = HashSet::new();
    collapsed.insert("src".to_string());
    let rows = build_tree_rows(&files, &collapsed);
    assert_eq!(rows.len(), 2); // AllChanges + collapsed src folder only
    match rows[1].kind {
        TreeKind::Folder {
            expanded,
            total,
            staged,
        } => {
            assert!(!expanded);
            assert_eq!(total, 2);
            assert_eq!(staged, 2);
        }
        _ => panic!("expected folder row"),
    }
}

#[test]
fn files_panel_enter_toggles_folder_collapse() {
    use ratatui::crossterm::event::KeyCode;
    let mut state = AppState::new();
    state.files = vec![FileEntry {
        path: "src/lib.rs".into(),
        x: 'M',
        y: ' ',
    }];
    // Initial rows: AllChanges, src/, src/lib.rs. Move cursor to folder row.
    state.files_list.idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(
        state.collapsed_dirs.contains("src"),
        "Enter on expanded folder should collapse it"
    );
    // Now only AllChanges + collapsed src remain
    assert_eq!(state.tree_rows().len(), 2);

    // Enter again re-expands.
    panel::files::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(!state.collapsed_dirs.contains("src"));
    assert_eq!(state.tree_rows().len(), 3);
}
