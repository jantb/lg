//! What the model is shown of the change for a file, a concern or the whole
//! of it.

use super::*;

/// Everything the model is shown about step `index`: the change's overview,
/// the file's diff with every line numbered, and the source around its hunks
/// when the files on disk are the ones being reviewed.
pub(super) fn step_context(guided: &Guided, index: usize) -> String {
    format!(
        "Change overview:\n{}{}",
        guided.overview,
        file_context(guided, index)
    )
}

pub(super) fn file_context(guided: &Guided, index: usize) -> String {
    let step = &guided.steps[index - 1];
    let mut out = String::new();
    out.push_str(&format!(
        "\nFile on screen: {} ({} hunk(s)){}\n",
        step.path,
        step.hunks(),
        step.file_note
            .as_deref()
            .map(|note| format!(", {note}"))
            .unwrap_or_default()
    ));
    out.push_str(&numbered_diff(step));
    out.push_str(&concern_context(guided, step));
    if guided.source.editable()
        && let Some(source) = source_around(step)
    {
        out.push_str("\nThe file around its hunks, as it is now:\n");
        out.push_str(&source);
    }
    if guided
        .shared_brief
        .as_ref()
        .is_some_and(|shared| *shared != guided.brief)
    {
        out.push_str(
            "\nThe change has moved on since the overview you were given; it now reads:\n",
        );
        out.push_str(&guided.brief);
    }
    if let Some(earlier) = guided.earlier.get(&step.path) {
        out.push_str(
            "\nYour earlier read of this file, before the reviewer changed it. Give each finding \
             the code now fixes as a `Fixed:` line, and repeat only what still stands:\n",
        );
        out.push_str(earlier);
    }
    out
}

/// The concern the plan puts `step` in, and the diffs of its other files, so
/// the model reads a call site with the definition it calls.
fn concern_context(guided: &Guided, step: &GuidedFile) -> String {
    let Some(plan) = guided.plan() else {
        return String::new();
    };
    let Some(concern) = plan.concerns.iter().find(|concern| {
        concern.title != plan::EVERYTHING_ELSE && concern.paths.contains(&step.path)
    }) else {
        return String::new();
    };
    let why = concern
        .why
        .as_deref()
        .map(|why| format!(": {why}"))
        .unwrap_or_default();
    let mut out = format!(
        "\nThis file is part of the concern \"{}\"{why}\n",
        concern.title
    );
    let others: Vec<&GuidedFile> = guided
        .steps
        .iter()
        .filter(|other| other.path != step.path && concern.paths.contains(&other.path))
        .collect();
    if others.is_empty() {
        return out;
    }
    out.push_str(
        "The concern's other files, for context only: review and cite only the file on screen, \
         but flag where it and they disagree.\n",
    );
    let mut left = CONCERN_CONTEXT_BYTES;
    for other in others {
        let diff = numbered_diff(other);
        if diff.len() > left {
            out.push_str(&format!("\n=== {}: left out, too long\n", other.path));
            continue;
        }
        left -= diff.len();
        out.push_str(&format!("\n=== {}\n{diff}", other.path));
    }
    out
}

/// Every file's numbered diff, as much as fits.
pub(super) fn whole_diff(guided: &Guided) -> String {
    let mut out = String::new();
    let mut left = WHOLE_DIFF_BYTES;
    for step in &guided.steps {
        let note = step
            .file_note
            .as_deref()
            .map(|note| format!(" ({note})"))
            .unwrap_or_default();
        let diff = numbered_diff(step);
        if diff.len() > left {
            out.push_str(&format!(
                "\n=== {}{note}: left out, the diff is too long to show whole\n",
                step.path
            ));
            continue;
        }
        left -= diff.len();
        out.push_str(&format!("\n=== {}{note}\n{diff}", step.path));
    }
    out
}

/// The file's diff with each line numbered as a finding cites it.
fn numbered_diff(step: &GuidedFile) -> String {
    let mut out = String::new();
    for line in &step.lines {
        let number = line
            .new
            .or(line.old)
            .map(|n| format!("L{n}"))
            .unwrap_or_default();
        out.push_str(&format!("{number:>6} {}\n", line.text));
    }
    out
}

/// The whole changeset for the final review: every file's diff, as much as
/// fits, and what the file-by-file reads have found and not yet had ticked off.
pub(super) fn final_context(guided: &Guided) -> String {
    let mut out = format!(
        "Change overview:\n{}\nThe whole diff, file by file:\n{}",
        guided.overview,
        whole_diff(guided)
    );
    out.push_str("\nWhat the file-by-file reads raised that is still open:\n");
    for step in &guided.steps {
        let Some(commentary) = guided.commentary.get(&step.key()).filter(|c| c.done) else {
            out.push_str(&format!("- {}: not read yet\n", step.path));
            continue;
        };
        let open = findings::read(&commentary.text)
            .findings
            .into_iter()
            .filter(|finding| !guided.is_ticked(&ticket_in(&step.path, finding)));
        for finding in open {
            out.push_str(&format!(
                "- {}:L{} {}: {}\n",
                step.path,
                finding.line,
                finding.severity.word(),
                finding.text
            ));
        }
    }
    for note in &guided.progress.notes {
        out.push_str(&format!(
            "- {}:L{} reviewer's note: {}\n",
            note.path, note.line, note.body
        ));
    }
    out
}

/// The working file's lines around each hunk, numbered; windows that meet
/// are joined, and the gaps between them marked.
fn source_around(step: &GuidedFile) -> Option<String> {
    let mut windows: Vec<(usize, usize)> = Vec::new();
    let mut hunk: Option<(usize, usize)> = None;
    for line in &step.lines {
        if line.kind == crate::git::guided::LineKind::Header {
            windows.extend(hunk.take());
            continue;
        }
        if let Some(n) = line.new {
            hunk = Some(hunk.map_or((n, n), |(first, _)| (first, n)));
        }
    }
    windows.extend(hunk);
    if windows.is_empty() {
        return None;
    }
    let root = crate::git::repo_dir()?;
    let text = std::fs::read_to_string(root.join(&step.path)).ok()?;
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (first, last) in windows {
        let from = first.saturating_sub(SOURCE_CONTEXT_LINES).max(1);
        let to = last + SOURCE_CONTEXT_LINES;
        match merged.last_mut() {
            Some((_, end)) if from <= *end + 1 => *end = (*end).max(to),
            _ => merged.push((from, to)),
        }
    }
    let mut out = String::new();
    for (at, (from, to)) in merged.into_iter().enumerate() {
        if at > 0 {
            out.push_str("   \u{22ef}\n");
        }
        for (index, line) in text.lines().enumerate().skip(from - 1).take(to + 1 - from) {
            out.push_str(&format!("{:>6} {line}\n", format!("L{}", index + 1)));
        }
    }
    Some(out)
}
