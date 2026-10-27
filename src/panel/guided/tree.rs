//! The walk's files as the directory tree they sit in, for the step list:
//! fully expanded, every directory on a row of its own above its files.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Dir {
        name: String,
        depth: usize,
    },
    /// `step` counts from one, as [`super::Guided::current`] does.
    File {
        step: usize,
        name: String,
        depth: usize,
    },
}

#[derive(Default)]
struct Node {
    children: Vec<Child>,
}

enum Child {
    Dir(String, Node),
    File(usize, String),
}

/// The tree of `files`, each a step number and its path, which come in path
/// order so every directory's files are next to each other.
pub fn entries(files: &[(usize, &str)]) -> Vec<Entry> {
    let mut root = Node::default();
    for &(step, path) in files {
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().unwrap_or_default().to_string();
        let mut node = &mut root;
        for part in parts {
            let at = node
                .children
                .iter()
                .position(|child| matches!(child, Child::Dir(dir, _) if dir == part));
            let at = at.unwrap_or_else(|| {
                node.children
                    .push(Child::Dir(part.to_string(), Node::default()));
                node.children.len() - 1
            });
            let Child::Dir(_, next) = &mut node.children[at] else {
                unreachable!("the position found is a directory");
            };
            node = next;
        }
        node.children.push(Child::File(step, name));
    }
    let mut out = Vec::new();
    flatten(&root, 0, &mut out);
    out
}

fn flatten(node: &Node, depth: usize, out: &mut Vec<Entry>) {
    for child in &node.children {
        match child {
            Child::File(step, name) => out.push(Entry::File {
                step: *step,
                name: name.clone(),
                depth,
            }),
            Child::Dir(name, sub) => {
                out.push(Entry::Dir {
                    name: format!("{name}/"),
                    depth,
                });
                flatten(sub, depth + 1, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files<'a>(paths: &[&'a str]) -> Vec<(usize, &'a str)> {
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| (index + 1, *path))
            .collect()
    }

    fn shown(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Dir { name, depth } => format!("{}{name}", "  ".repeat(*depth)),
                Entry::File { name, depth, .. } => format!("{}{name}", "  ".repeat(*depth)),
            })
            .collect()
    }

    #[test]
    fn every_directory_has_a_row_with_its_files_under_it() {
        let steps = files(&[
            "README.md",
            "src/a.rs",
            "src/panel/guided/draw.rs",
            "src/panel/guided/keys.rs",
            "src/panel/mod.rs",
            "tests/deep/only/one.rs",
        ]);

        assert_eq!(
            shown(&entries(&steps)),
            [
                "README.md",
                "src/",
                "  a.rs",
                "  panel/",
                "    guided/",
                "      draw.rs",
                "      keys.rs",
                "    mod.rs",
                "tests/",
                "  deep/",
                "    only/",
                "      one.rs",
            ]
        );
    }

    #[test]
    fn each_file_row_opens_its_own_step() {
        let files = [(7, "src/a/x.rs"), (3, "src/b.rs")];

        for entry in entries(&files) {
            if let Entry::File { step, name, .. } = entry {
                let path = files.iter().find(|(n, _)| *n == step).unwrap().1;
                assert!(path.ends_with(&name), "{name}");
            }
        }
    }
}
