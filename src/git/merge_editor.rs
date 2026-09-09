//! Snapshots and guarded writes for the manual conflict editor.

use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::{ConflictedFile, git_command_in_dir, run_in_dir};

#[derive(Debug, Clone)]
pub struct MergeSnapshot {
    pub root: PathBuf,
    pub path: String,
    pub original: String,
    pub base: String,
    pub ours: String,
    pub theirs: String,
    pub diff3: Option<ConflictedFile>,
    index: Vec<u8>,
}

const MAX_BYTES: usize = 2 * 1024 * 1024;

fn regular_file(root: &Path, path: &str) -> Result<PathBuf> {
    if path.is_empty()
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("invalid repository-relative path");
    }
    let full = root.join(path);
    if !std::fs::symlink_metadata(&full)?.file_type().is_file() {
        bail!("use your external editor for symlink or deleted-file conflicts");
    }
    if !full.canonicalize()?.starts_with(root.canonicalize()?) {
        bail!("the file is outside this repository");
    }
    Ok(full)
}

fn index_entries(root: &Path, path: &str) -> Result<Vec<u8>> {
    Ok(run_in_dir(
        root,
        &["--literal-pathspecs", "ls-files", "-u", "-z", "--", path],
    )?
    .stdout)
}

impl MergeSnapshot {
    pub fn load(root: &Path, path: &str) -> Result<Self> {
        let full = regular_file(root, path)?;
        if std::fs::metadata(&full)?.len() > MAX_BYTES as u64 {
            bail!("file is too large for the inline editor; press o to open externally");
        }
        let original =
            std::fs::read_to_string(full).context("the inline editor requires UTF-8 text")?;
        if original.contains('\0') {
            bail!("binary conflict; press o to open externally");
        }
        let index = index_entries(root, path)?;
        if index.is_empty() {
            bail!("this file is already staged; press v to validate and continue");
        }
        let mut stages = [String::new(), String::new(), String::new()];
        let mut present = [false; 3];
        for entry in index
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let header = entry
                .split(|byte| *byte == b'\t')
                .next()
                .unwrap_or_default();
            let header = std::str::from_utf8(header)?;
            let fields: Vec<_> = header.split_whitespace().collect();
            if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") {
                bail!("this conflict needs an external editor (unsupported file type)");
            }
            let stage: usize = fields[2].parse()?;
            if !(1..=3).contains(&stage) {
                bail!("invalid index stage");
            }
            let size = run_in_dir(root, &["cat-file", "-s", fields[1]])?;
            if std::str::from_utf8(&size.stdout)?.trim().parse::<usize>()? > MAX_BYTES {
                bail!("a conflict side is too large for the inline editor");
            }
            let blob = run_in_dir(root, &["cat-file", "blob", fields[1]])?.stdout;
            let text = String::from_utf8(blob).context("a conflict side is not UTF-8 text")?;
            if text.contains('\0') {
                bail!("binary conflict; press o to open externally");
            }
            stages[stage - 1] = text;
            present[stage - 1] = true;
        }
        if !present[1] || !present[2] {
            bail!(
                "delete/modify conflict; resolve the file's deletion or retention externally with o"
            );
        }
        // Reconstruct ancestor context in temporary files only. The working
        // file remains the authority, including any resolutions already made.
        let scratch = tempfile::tempdir()?;
        for (name, text) in ["base", "ours", "theirs"].into_iter().zip(&stages) {
            std::fs::write(scratch.path().join(name), text)?;
        }
        let output = git_command_in_dir(
            scratch.path(),
            &["merge-file", "--diff3", "-p", "ours", "base", "theirs"],
        )
        .output()?;
        let diff3 = if output
            .status
            .code()
            .is_some_and(|code| (0..=127).contains(&code))
        {
            String::from_utf8(output.stdout)
                .ok()
                .and_then(|text| ConflictedFile::parse(&text))
        } else {
            None
        };
        Ok(Self {
            root: root.to_path_buf(),
            path: path.into(),
            original,
            base: stages[0].clone(),
            ours: stages[1].clone(),
            theirs: stages[2].clone(),
            diff3,
            index,
        })
    }

    /// Save only against the checkout and unmerged index we originally read.
    /// Staging and continuing remain the conflict dialog's explicit v action.
    pub fn save(&mut self, result: &str) -> Result<()> {
        if super::holds_conflict_marker(result) {
            bail!("the result still contains conflict markers");
        }
        let full = regular_file(&self.root, &self.path)?;
        if std::fs::read(&full)? != self.original.as_bytes()
            || index_entries(&self.root, &self.path)? != self.index
        {
            bail!("the file or Git index changed outside this editor; reload before saving");
        }
        let permissions = std::fs::metadata(&full)?.permissions();
        let mut temp =
            tempfile::NamedTempFile::new_in(full.parent().context("file has no parent")?)?;
        temp.as_file().set_permissions(permissions)?;
        temp.write_all(result.as_bytes())?;
        temp.as_file().sync_all()?;
        // Check again after preparing the replacement, before the atomic rename.
        regular_file(&self.root, &self.path)?;
        if std::fs::read(&full)? != self.original.as_bytes()
            || index_entries(&self.root, &self.path)? != self.index
        {
            bail!("the file or Git index changed outside this editor; reload before saving");
        }
        temp.persist(full).context("save merged file")?;
        self.original = result.into();
        Ok(())
    }
}
