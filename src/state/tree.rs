use std::collections::{BTreeMap, HashSet};

use crate::git::FileEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeKind {
    AllChanges,
    Folder {
        expanded: bool,
        total: usize,
        staged: usize,
    },
    File {
        entry_idx: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub kind: TreeKind,
    pub depth: u16,
    pub path: String,
    pub label: String,
}

#[derive(Default, Debug)]
struct DirNode {
    subdirs: BTreeMap<String, DirNode>,
    files: Vec<usize>,
}

pub fn build_tree_rows(files: &[FileEntry], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = DirNode::default();
    for (idx, file) in files.iter().enumerate() {
        let mut node = &mut root;
        let parts: Vec<&str> = file.path.split('/').collect();
        let last = parts.len().saturating_sub(1);
        for segment in &parts[..last] {
            node = node.subdirs.entry((*segment).to_string()).or_default();
        }
        node.files.push(idx);
    }

    let mut rows = vec![TreeRow {
        kind: TreeKind::AllChanges,
        depth: 0,
        path: String::new(),
        label: "(all changes)".to_string(),
    }];
    emit_rows(&root, "", 0, files, collapsed, &mut rows);
    rows
}

fn count_descendants(node: &DirNode, files: &[FileEntry]) -> (usize, usize) {
    let mut total = 0usize;
    let mut staged = 0usize;
    for &idx in &node.files {
        total += 1;
        let file = &files[idx];
        if file.x != ' ' && file.x != '?' {
            staged += 1;
        }
    }
    for child in node.subdirs.values() {
        let (child_total, child_staged) = count_descendants(child, files);
        total += child_total;
        staged += child_staged;
    }
    (total, staged)
}

fn emit_rows(
    node: &DirNode,
    prefix: &str,
    depth: u16,
    files: &[FileEntry],
    collapsed: &HashSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    enum Child<'a> {
        Dir(&'a String, &'a DirNode),
        File(usize),
    }

    let mut children: Vec<Child> = node.subdirs.iter().map(|(n, c)| Child::Dir(n, c)).collect();
    for &idx in &node.files {
        children.push(Child::File(idx));
    }
    children.sort_by_cached_key(|child| match child {
        Child::Dir(name, _) => name.to_ascii_lowercase(),
        Child::File(idx) => {
            let path = &files[*idx].path;
            path.rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(path)
                .to_ascii_lowercase()
        }
    });

    for child in children {
        match child {
            Child::Dir(name, dir) => {
                let initial_path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                let (label, path, dir) =
                    compact_single_subdir_chain(name.clone(), initial_path, dir, collapsed);
                let (total, staged) = count_descendants(dir, files);
                let expanded = !collapsed.contains(&path);
                rows.push(TreeRow {
                    kind: TreeKind::Folder {
                        expanded,
                        total,
                        staged,
                    },
                    depth,
                    path: path.clone(),
                    label,
                });
                if expanded {
                    emit_rows(dir, &path, depth + 1, files, collapsed, rows);
                }
            }
            Child::File(idx) => {
                let file = &files[idx];
                let label = file
                    .path
                    .rsplit_once('/')
                    .map(|(_, name)| name)
                    .unwrap_or(&file.path)
                    .to_string();
                rows.push(TreeRow {
                    kind: TreeKind::File { entry_idx: idx },
                    depth,
                    path: file.path.clone(),
                    label,
                });
            }
        }
    }
}

fn compact_single_subdir_chain<'a>(
    mut label: String,
    mut path: String,
    mut node: &'a DirNode,
    collapsed: &HashSet<String>,
) -> (String, String, &'a DirNode) {
    if collapsed.contains(&path) {
        return (label, path, node);
    }

    while node.files.is_empty() && node.subdirs.len() == 1 {
        let (child_name, child) = node
            .subdirs
            .iter()
            .next()
            .expect("single subdir must exist");
        label.push('/');
        label.push_str(child_name);
        path.push('/');
        path.push_str(child_name);
        node = child;
        if collapsed.contains(&path) {
            break;
        }
    }

    (label, path, node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fe(path: &str, x: char, y: char) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            x,
            y,
        }
    }

    #[test]
    fn tree_flat_files_emit_all_plus_files() {
        let files = vec![fe("a.rs", ' ', 'M'), fe("b.rs", 'A', ' ')];
        let rows = build_tree_rows(&files, &HashSet::new());
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].kind, TreeKind::AllChanges));
        assert!(matches!(rows[1].kind, TreeKind::File { entry_idx: 0 }));
        assert!(matches!(rows[2].kind, TreeKind::File { entry_idx: 1 }));
    }

    #[test]
    fn tree_groups_files_under_folders_when_expanded() {
        let files = vec![
            fe("src/lib.rs", 'M', ' '),
            fe("src/util/mod.rs", 'A', ' '),
            fe("README.md", ' ', 'M'),
        ];
        let rows = build_tree_rows(&files, &HashSet::new());
        assert!(matches!(rows[0].kind, TreeKind::AllChanges));
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
            _ => panic!("expected folder, got {:?}", rows[2].kind),
        }
        assert_eq!(rows[3].path, "src/lib.rs");
        assert_eq!(rows[4].path, "src/util");
        assert_eq!(rows[5].path, "src/util/mod.rs");
    }

    #[test]
    fn tree_compacts_single_subdir_chains() {
        let files = vec![
            fe("src/main/kotlin/org/example/inventory/App.kt", 'M', ' '),
            fe("src/main/kotlin/org/example/inventory/Service.kt", ' ', 'M'),
        ];
        let rows = build_tree_rows(&files, &HashSet::new());

        assert_eq!(rows[1].path, "src/main/kotlin/org/example/inventory");
        assert_eq!(rows[1].label, "src/main/kotlin/org/example/inventory");
        match rows[1].kind {
            TreeKind::Folder {
                expanded,
                total,
                staged,
            } => {
                assert!(expanded);
                assert_eq!(total, 2);
                assert_eq!(staged, 1);
            }
            _ => panic!("expected compacted folder"),
        }
        assert_eq!(rows[2].path, "src/main/kotlin/org/example/inventory/App.kt");
        assert_eq!(
            rows[3].path,
            "src/main/kotlin/org/example/inventory/Service.kt"
        );
    }

    #[test]
    fn tree_compaction_stops_at_collapsed_path() {
        let files = vec![fe("src/main/kotlin/org/example/inventory/App.kt", 'M', ' ')];
        let mut collapsed = HashSet::new();
        collapsed.insert("src/main".to_string());
        let rows = build_tree_rows(&files, &collapsed);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].path, "src/main");
        assert_eq!(rows[1].label, "src/main");
        match rows[1].kind {
            TreeKind::Folder { expanded, .. } => assert!(!expanded),
            _ => panic!("expected compacted collapsed folder"),
        }
    }

    #[test]
    fn tree_collapsed_folder_hides_children() {
        let files = vec![fe("src/lib.rs", 'M', ' '), fe("src/mod.rs", 'A', ' ')];
        let mut collapsed = HashSet::new();
        collapsed.insert("src".to_string());
        let rows = build_tree_rows(&files, &collapsed);
        assert_eq!(rows.len(), 2);
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
            _ => panic!("expected folder"),
        }
    }
}
