use anyhow::Result;

use super::run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub x: char,
    pub y: char,
}

/// Parse `git status -z --porcelain=v1` output.
/// Returns `(unstaged, staged)`; a file may appear in both.
pub fn parse_porcelain(bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let mut unstaged = Vec::new();
    let mut staged = Vec::new();

    // Records are NUL-separated. For rename/copy (R/C), the record contains
    // "XY path" and the old path follows as a second NUL-terminated record.
    let mut records: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    if records.last().map(|r| r.is_empty()).unwrap_or(false) {
        records.pop();
    }

    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;

        if rec.len() < 4 {
            continue;
        }

        let x = rec[0] as char;
        let y = rec[1] as char;
        let path = String::from_utf8_lossy(&rec[3..]).into_owned();

        if x == 'R' || x == 'C' {
            i += 1;
        }

        if x != ' ' && x != '?' {
            staged.push(path.clone());
        }
        if y != ' ' && y != '.' {
            unstaged.push(path.clone());
        }
    }

    (unstaged, staged)
}

pub fn status_porcelain() -> Result<(Vec<String>, Vec<String>)> {
    let out = run(&["status", "-z", "--porcelain=v1"])?;
    Ok(parse_porcelain(&out.stdout))
}

/// Parse `git status -z --porcelain=v1` output into unified `FileEntry` vec.
/// Each entry carries the raw x and y status chars.
pub fn parse_porcelain_xy(bytes: &[u8]) -> Vec<FileEntry> {
    let mut records: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    if records.last().map(|r| r.is_empty()).unwrap_or(false) {
        records.pop();
    }

    let mut entries = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;

        if rec.len() < 4 {
            continue;
        }

        let x = rec[0] as char;
        let y = rec[1] as char;
        let path = String::from_utf8_lossy(&rec[3..]).into_owned();

        if x == 'R' || x == 'C' {
            i += 1;
        }

        entries.push(FileEntry { path, x, y });
    }

    entries
}

pub fn status_entries() -> Result<Vec<FileEntry>> {
    let out = run(&["status", "-z", "--porcelain=v1"])?;
    expand_untracked_directories(parse_porcelain_xy(&out.stdout))
}

fn expand_untracked_directories(entries: Vec<FileEntry>) -> Result<Vec<FileEntry>> {
    let mut expanded = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.x == '?' && entry.y == '?' && entry.path.ends_with('/') {
            let files = untracked_files_under(&entry.path)?;
            if files.is_empty() {
                expanded.push(entry);
            } else {
                expanded.extend(files.into_iter().map(|path| FileEntry {
                    path,
                    x: entry.x,
                    y: entry.y,
                }));
            }
        } else {
            expanded.push(entry);
        }
    }
    Ok(expanded)
}

fn untracked_files_under(path: &str) -> Result<Vec<String>> {
    let out = run(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
        path,
    ])?;
    Ok(out
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect())
}
