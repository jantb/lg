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
        state.conflict_preview = None;
        return;
    }
    let Some(path) = state.conflicts.get(state.conflict_idx) else {
        return;
    };
    let Some(root) = &state.repo_root else {
        return;
    };
    if state
        .conflict_preview
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
        state.conflict_resolved.remove(path);
    }
    state.conflict_preview = Some(ConflictPreview {
        root: root.clone(),
        path: path.clone(),
        editor,
    });
}

pub(crate) fn save_conflict_editor(state: &mut AppState) {
    if state.conflict_resolve_job.is_some() {
        return;
    }
    let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor) else {
        return;
    };
    match editor.save() {
        Ok(()) => {
            let path = editor.snapshot.path.clone();
            state.conflict_resolved.insert(path.clone());
            state.conflict_log = format!(
                "Saved {path}. Review the other files, then v stages resolutions and continues."
            );
            state.set_status("merged file saved; v validates and continues", false);
        }
        Err(err) => {
            state.conflict_log = err.to_string();
            state.set_status(err.to_string(), true);
        }
    }
}
