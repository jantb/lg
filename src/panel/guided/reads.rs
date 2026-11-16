//! What becomes of a read once the model has answered: taken in by the rules
//! of [`super::take_in`], kept in the review's progress, and given back when
//! the review is opened again.

use crate::git::guided::{FileRead, GuidedFile, WholeRead};

use super::take_in::{self, FILE_NIT_BUDGET, TakeIn, WHOLE_NIT_BUDGET};
use super::{Commentary, Guided, OVERVIEW_KEY, findings};

impl Guided {
    /// How the read of file `index` is asked for and taken in: against its
    /// earlier read when the file changed since that read, else afresh.
    pub(super) fn file_take_in(&self, index: usize) -> TakeIn {
        let step = &self.steps[index - 1];
        match self
            .earlier
            .get(&step.path)
            .filter(|earlier| earlier.step.key != step.key())
        {
            Some(earlier) => {
                TakeIn::FileDelta(take_in::file_basis(earlier, step, &self.progress.ticked))
            }
            None => TakeIn::FileFirst {
                path: step.path.clone(),
            },
        }
    }

    /// The same for the read of the whole change.
    pub(super) fn whole_take_in(&self) -> TakeIn {
        match self
            .earlier_whole
            .as_ref()
            .filter(|earlier| earlier.key != self.final_key())
        {
            Some(earlier) => TakeIn::WholeDelta(take_in::whole_basis(
                earlier,
                &self.steps,
                &self.progress.ticked,
            )),
            None => TakeIn::WholeFirst,
        }
    }

    /// `reply`, the answer to the read of step `key`, as the walk keeps it.
    pub(super) fn taken_in(&self, key: &str, how: &TakeIn, reply: &str) -> String {
        let ticked = &self.progress.ticked;
        match how {
            TakeIn::AsWritten => reply.to_string(),
            TakeIn::FileFirst { path } => {
                take_in::take_in_first(reply, Some(path), FILE_NIT_BUDGET, ticked)
            }
            TakeIn::FileDelta(basis) => match self.steps.iter().find(|step| step.key() == key) {
                Some(step) => take_in::take_in_file_delta(basis, step, reply, ticked),
                None => take_in::take_in_first(reply, Some(&basis.path), FILE_NIT_BUDGET, ticked),
            },
            TakeIn::WholeFirst => take_in::take_in_first(reply, None, WHOLE_NIT_BUDGET, ticked),
            TakeIn::WholeDelta(basis) => {
                take_in::take_in_whole_delta(basis, &self.steps, reply, ticked)
            }
        }
    }

    /// A read taken in is what the next read is set against, so the one it
    /// was set against is let go.
    pub(super) fn let_go_of_earlier(&mut self, how: &TakeIn) {
        match how {
            TakeIn::AsWritten => {}
            TakeIn::FileFirst { path } => {
                self.earlier.remove(path);
            }
            TakeIn::FileDelta(basis) => {
                self.earlier.remove(&basis.path);
            }
            TakeIn::WholeFirst | TakeIn::WholeDelta(_) => self.earlier_whole = None,
        }
    }

    /// Keeps the finished read of `key` in the progress, for the review to be
    /// opened again with it; whether there was one to keep.
    pub(super) fn remember_read(&mut self, key: &str) -> bool {
        let Some(text) = self
            .commentary
            .get(key)
            .filter(|c| c.done && !c.checking && c.error.is_none())
            .map(|c| c.text.clone())
        else {
            return false;
        };
        if key == OVERVIEW_KEY {
            self.progress.reads.plan = Some(text);
            return true;
        }
        if key == self.final_key() {
            self.progress.reads.whole = Some(whole_read_of(&self.steps, key, text));
            return true;
        }
        let Some(step) = self.steps.iter().find(|step| step.key() == key) else {
            return false;
        };
        let read = file_read_of(step, text);
        let files = &mut self.progress.reads.files;
        match files
            .iter_mut()
            .find(|saved| saved.step.path == read.step.path)
        {
            Some(saved) => *saved = read,
            None => files.push(read),
        }
        true
    }

    /// A review opened again takes back its saved reads: a read of a file
    /// that has not changed since is shown as it was, without asking again;
    /// one of a file changed since is what the file's next read is set
    /// against. Likewise the whole change's, and the plan as it was.
    pub(super) fn restore_reads(&mut self) {
        let reads = self.progress.reads.clone();
        if let Some(plan) = reads.plan {
            self.commentary
                .entry(OVERVIEW_KEY.to_string())
                .or_insert_with(|| finished(plan));
        }
        for read in reads.files {
            let Some(step) = self.steps.iter().find(|step| step.path == read.step.path) else {
                continue;
            };
            let key = step.key();
            if read.step.key == key {
                self.commentary
                    .entry(key)
                    .or_insert_with(|| finished(read.text));
            } else {
                self.earlier.entry(read.step.path.clone()).or_insert(read);
            }
        }
        if let Some(whole) = reads.whole {
            let key = self.final_key();
            if whole.key == key {
                self.commentary
                    .entry(key)
                    .or_insert_with(|| finished(whole.text));
            } else if self.earlier_whole.is_none() {
                self.earlier_whole = Some(whole);
            }
        }
    }
}

fn finished(text: String) -> Commentary {
    Commentary {
        text,
        done: true,
        ..Commentary::default()
    }
}

/// The read `text` of `step`, with the step as it was read.
pub(super) fn file_read_of(step: &GuidedFile, text: String) -> FileRead {
    let cited: Vec<usize> = findings::read(&text)
        .findings
        .iter()
        .map(|finding| finding.line)
        .collect();
    FileRead {
        step: take_in::read_step(step, cited),
        text,
    }
}

/// The read `text` of the whole change of `steps`, whose final step is `key`.
pub(super) fn whole_read_of(steps: &[GuidedFile], key: &str, text: String) -> WholeRead {
    let cited = findings::read(&text).cited;
    let files = steps
        .iter()
        .map(|step| {
            let lines = cited
                .iter()
                .filter(|(path, _)| *path == step.path)
                .map(|(_, finding)| finding.line);
            take_in::read_step(step, lines)
        })
        .collect();
    WholeRead {
        key: key.to_string(),
        text,
        files,
    }
}
