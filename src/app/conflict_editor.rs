//! Load previews on selection changes, and coordinate manual saves with jobs.

use crate::{
    git::MergeSnapshot,
    state::{AppState, ConflictPreview, MergeEditor, Modal},
};

pub(crate) fn prepare_conflict_editor(state: &mut AppState) {
    if state.modal != Modal::Conflict {
        return;
    }
    if state.conflict_resolve_job.is_some() {
        state.conflict.preview = None;
        return;
    }
    let Some(path) = state.conflict.files.get(state.conflict.idx) else {
        return;
    };
    let Some(root) = &state.repo_root else {
        return;
    };
    if state
        .conflict
        .preview
        .as_ref()
        .is_some_and(|p| p.path == *path && p.root == *root)
    {
        return;
    }
    let editor = MergeSnapshot::load(std::path::Path::new(root), path)
        .and_then(MergeEditor::new)
        .map_err(|err| format!("{err:#}"));
    if editor
        .as_ref()
        .is_ok_and(|editor| crate::git::holds_conflict_marker(&editor.snapshot.original))
    {
        state.conflict.resolved.remove(path);
    }
    state.conflict.preview = Some(ConflictPreview {
        root: root.clone(),
        path: path.clone(),
        editor,
    });
}

/// Bring the conflict dialog back up, `select`ing one file if asked. The
/// dialog closes without the conflict going anywhere, and lg may have been
/// started on a checkout already in the middle of one, so git's list of
/// unmerged files is asked for first; what the dialog last showed stands in
/// when git has none to give. `false` when there is nothing to resolve.
pub(crate) fn reopen_conflicts(state: &mut AppState, select: Option<&str>) -> bool {
    let conflicts = crate::git::conflicted_files().unwrap_or_default();
    if !conflicts.is_empty() && state.conflict.files != conflicts {
        state.set_conflicts(conflicts);
    }
    if state.conflict.files.is_empty() {
        return false;
    }
    if let Some(index) = select.and_then(|path| state.conflict.files.iter().position(|c| c == path))
    {
        state.conflict.idx = index;
    }
    if state.repo_root.is_none() {
        state.repo_root = crate::git::repo_root().ok();
    }
    state.modal = Modal::Conflict;
    true
}

pub(crate) fn save_conflict_editor(state: &mut AppState) {
    if state.conflict_resolve_job.is_some() {
        return;
    }
    let Some(Ok(editor)) = state.conflict.preview.as_mut().map(|p| &mut p.editor) else {
        return;
    };
    match editor.save() {
        Ok(()) => {
            let path = editor.snapshot.path.clone();
            state.conflict.resolved.insert(path.clone());
            state.conflict.log = format!(
                "Saved {path}. Review the other files, then v stages resolutions and continues."
            );
            state.set_status("merged file saved; v validates and continues", false);
        }
        Err(err) => {
            state.conflict.log = err.to_string();
            state.set_status(err.to_string(), true);
        }
    }
}
