//! Reading the conflicts git wrote into a file, and putting the file back
//! together once each one has an answer.
//!
//! Splicing rather than rewriting is the point: everything git managed to merge
//! is carried through byte for byte, so whatever settles a conflict only has to
//! be right about the conflict.

use super::{CONFLICT_MARKER_WIDTH, has_conflict_markers, is_marker, marker_label};

/// One conflict git left in a file: what each side wrote, and what it called
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictHunk {
    pub ours_label: String,
    pub ours: String,
    /// The common ancestor, when the file was written in diff3 style. Plain
    /// merge style leaves no base, which is why this is optional rather than
    /// empty.
    pub base: Option<String>,
    pub theirs: String,
    pub theirs_label: String,
}

impl ConflictHunk {
    /// The text both sides agree on, when they wrote the same thing. Git still
    /// reports a conflict when only the surroundings differ, and there is
    /// nothing to decide about one.
    pub fn agreed_text(&self) -> Option<String> {
        (self.ours == self.theirs).then(|| self.ours.clone())
    }

    /// How many lines the larger side spans — the measure of how much of a
    /// decision this conflict is.
    pub fn widest_side_lines(&self) -> usize {
        line_count(&self.ours).max(line_count(&self.theirs))
    }

    /// How long a resolution may reasonably be: both sides, plus room to join
    /// them up. Anything longer is the model writing rather than merging.
    pub fn resolution_budget_lines(&self) -> usize {
        line_count(&self.ours) + line_count(&self.theirs) + RESOLUTION_SLACK_LINES
    }
}

/// How many lines beyond both sides put together a resolution may run to.
const RESOLUTION_SLACK_LINES: usize = 8;

/// A stretch of a conflicted file: either text git merged on its own, or a
/// conflict it gave up on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Kept(String),
    Conflict(ConflictHunk),
}

/// A conflicted file split into the parts git merged and the parts it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedFile {
    parts: Vec<Part>,
}

/// Which side of a conflict the parser is reading.
enum Side {
    Ours,
    Base,
    Theirs,
}

/// A conflict being read, before its closing marker arrives.
struct Open {
    ours_label: String,
    ours: String,
    base: Option<String>,
    theirs: String,
    side: Side,
}

impl ConflictedFile {
    /// Split `text` on the conflicts git wrote into it.
    ///
    /// `None` when the markers are not a shape that can be spliced back
    /// together — nested, out of order, or unterminated — and `None` too when
    /// there is no conflict in the file at all. Both mean the same thing to a
    /// caller: this is not a file to settle hunk by hunk.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = Vec::new();
        let mut kept = String::new();
        let mut open: Option<Open> = None;

        for line in text.split_inclusive('\n') {
            let bare = line.trim_end_matches(['\n', '\r']);
            if let Some(label) = marker_label(bare, '<') {
                if open.is_some() {
                    return None;
                }
                if !kept.is_empty() {
                    parts.push(Part::Kept(std::mem::take(&mut kept)));
                }
                open = Some(Open {
                    ours_label: label.to_string(),
                    ours: String::new(),
                    base: None,
                    theirs: String::new(),
                    side: Side::Ours,
                });
                continue;
            }
            let Some(building) = open.as_mut() else {
                kept.push_str(line);
                continue;
            };
            if is_marker(bare, '|') {
                if !matches!(building.side, Side::Ours) {
                    return None;
                }
                building.side = Side::Base;
                building.base = Some(String::new());
            } else if is_marker(bare, '=') {
                if matches!(building.side, Side::Theirs) {
                    return None;
                }
                building.side = Side::Theirs;
            } else if let Some(label) = marker_label(bare, '>') {
                let building = open.take()?;
                if !matches!(building.side, Side::Theirs) {
                    return None;
                }
                parts.push(Part::Conflict(ConflictHunk {
                    ours_label: building.ours_label,
                    ours: building.ours,
                    base: building.base,
                    theirs: building.theirs,
                    theirs_label: label.to_string(),
                }));
            } else {
                match building.side {
                    Side::Ours => building.ours.push_str(line),
                    Side::Base => building.base.as_mut()?.push_str(line),
                    Side::Theirs => building.theirs.push_str(line),
                }
            }
        }

        if open.is_some() {
            return None;
        }
        if !kept.is_empty() {
            parts.push(Part::Kept(kept));
        }
        parts
            .iter()
            .any(|part| matches!(part, Part::Conflict(_)))
            .then_some(Self { parts })
    }

    pub fn hunks(&self) -> impl Iterator<Item = &ConflictHunk> {
        self.parts.iter().filter_map(|part| match part {
            Part::Conflict(hunk) => Some(hunk),
            Part::Kept(_) => None,
        })
    }

    pub fn hunk_count(&self) -> usize {
        self.hunks().count()
    }

    /// The last `lines` merged lines ahead of conflict `index`, as the context
    /// whatever settles it reads to know where it is.
    pub fn context_before(&self, index: usize, lines: usize) -> String {
        let Some(position) = self.position_of(index) else {
            return String::new();
        };
        let Some(Part::Kept(text)) = position.checked_sub(1).and_then(|i| self.parts.get(i)) else {
            return String::new();
        };
        last_lines(text, lines)
    }

    /// The first `lines` merged lines after conflict `index`.
    pub fn context_after(&self, index: usize, lines: usize) -> String {
        let Some(position) = self.position_of(index) else {
            return String::new();
        };
        let Some(Part::Kept(text)) = self.parts.get(position + 1) else {
            return String::new();
        };
        first_lines(text, lines)
    }

    /// The file with each conflict replaced by its resolution, in order.
    /// Everything git merged comes through untouched.
    pub fn render(&self, resolutions: &[String]) -> String {
        let mut out = String::new();
        let mut resolved = resolutions.iter();
        for part in &self.parts {
            match part {
                Part::Kept(text) => out.push_str(text),
                Part::Conflict(_) => {
                    let text = resolved.next().map(String::as_str).unwrap_or_default();
                    out.push_str(text);
                    if !text.is_empty() && !text.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
        }
        out
    }

    fn position_of(&self, index: usize) -> Option<usize> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, part)| matches!(part, Part::Conflict(_)))
            .nth(index)
            .map(|(position, _)| position)
    }
}

/// Whether any line of `text` is one of git's conflict markers.
///
/// `=======` is left out on purpose: it is a heading rule in half the
/// documentation ever written, and a resolution is not disqualified by
/// containing one. The three asymmetric markers have no such second life, and a
/// paired `<`/`>` conflict is caught by [`has_conflict_markers`] anyway.
pub fn holds_conflict_marker(text: &str) -> bool {
    has_conflict_markers(text)
        || text.lines().any(|line| {
            ['<', '|', '>']
                .iter()
                .any(|marker| is_marker(line, *marker))
        })
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

fn last_lines(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.split_inclusive('\n').collect();
    all[all.len().saturating_sub(lines)..].concat()
}

fn first_lines(text: &str, lines: usize) -> String {
    text.split_inclusive('\n')
        .take(lines)
        .collect::<Vec<_>>()
        .concat()
}

/// A marker line git would write for `marker`, for the prompts that have to
/// show one without leaving a real one in the text.
pub fn marker_line(marker: char) -> String {
    marker.to_string().repeat(CONFLICT_MARKER_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MERGE_STYLE: &str = "\
fn main() {
<<<<<<< HEAD
    println!(\"ours\");
=======
    println!(\"theirs\");
>>>>>>> origin/main
}
";

    #[test]
    fn a_conflict_is_read_as_two_sides_with_the_file_around_them() {
        let file = ConflictedFile::parse(MERGE_STYLE).expect("git wrote a conflict here");
        let hunk = file.hunks().next().expect("one conflict");

        assert_eq!(hunk.ours, "    println!(\"ours\");\n");
        assert_eq!(hunk.theirs, "    println!(\"theirs\");\n");
        assert_eq!(hunk.base, None);
        assert_eq!(hunk.ours_label, "HEAD");
        assert_eq!(hunk.theirs_label, "origin/main");
    }

    #[test]
    fn a_diff3_conflict_keeps_the_common_ancestor() {
        let text = "<<<<<<< ours\na\n||||||| base\nb\n=======\nc\n>>>>>>> theirs\n";
        let file = ConflictedFile::parse(text).expect("a diff3 conflict");

        assert_eq!(file.hunks().next().unwrap().base.as_deref(), Some("b\n"));
    }

    #[test]
    fn splicing_a_resolution_leaves_the_merged_text_untouched() {
        let file = ConflictedFile::parse(MERGE_STYLE).unwrap();

        assert_eq!(
            file.render(&["    println!(\"both\");".to_string()]),
            "fn main() {\n    println!(\"both\");\n}\n",
            "the resolution goes in, the rest of the file comes through as it was"
        );
    }

    #[test]
    fn a_resolution_that_drops_both_sides_leaves_no_blank_line_behind() {
        let file = ConflictedFile::parse(MERGE_STYLE).unwrap();

        assert_eq!(file.render(&[String::new()]), "fn main() {\n}\n");
    }

    #[test]
    fn every_conflict_in_a_file_is_read_and_spliced_in_order() {
        let text = "a\n<<<<<<< HEAD\n1\n=======\n2\n>>>>>>> them\nb\n<<<<<<< HEAD\n3\n=======\n4\n>>>>>>> them\nc\n";
        let file = ConflictedFile::parse(text).unwrap();

        assert_eq!(file.hunk_count(), 2);
        assert_eq!(
            file.render(&["one".to_string(), "two".to_string()]),
            "a\none\nb\ntwo\nc\n"
        );
    }

    #[test]
    fn the_text_around_a_conflict_travels_with_it() {
        let text = "one\ntwo\nthree\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> them\nfour\nfive\n";
        let file = ConflictedFile::parse(text).unwrap();

        assert_eq!(file.context_before(0, 2), "two\nthree\n");
        assert_eq!(file.context_after(0, 1), "four\n");
    }

    #[test]
    fn a_file_git_merged_cleanly_has_nothing_to_splice() {
        assert_eq!(ConflictedFile::parse("fn main() {}\n"), None);
    }

    #[test]
    fn markers_that_do_not_pair_up_are_left_alone() {
        for text in [
            "<<<<<<< HEAD\nours\n",
            "ours\n=======\ntheirs\n>>>>>>> them\n",
            "<<<<<<< a\n1\n<<<<<<< b\n2\n=======\n3\n>>>>>>> c\n",
        ] {
            assert_eq!(
                ConflictedFile::parse(text),
                None,
                "lg must not try to splice {text:?}"
            );
        }
    }

    #[test]
    fn identical_sides_need_no_deciding() {
        let text = "<<<<<<< HEAD\nsame\n=======\nsame\n>>>>>>> them\n";
        let hunk = ConflictedFile::parse(text).unwrap();

        assert_eq!(
            hunk.hunks().next().unwrap().agreed_text().as_deref(),
            Some("same\n")
        );
    }

    #[test]
    fn a_marker_left_in_a_resolution_is_spotted() {
        assert!(holds_conflict_marker("ok\n<<<<<<< HEAD\n"));
        assert!(holds_conflict_marker(">>>>>>> origin/main\n"));
        assert!(holds_conflict_marker("||||||| base\n"));
    }

    /// A heading rule is not a leaked marker, and rejecting one would make
    /// every conflict in a Markdown file unresolvable.
    #[test]
    fn a_heading_rule_is_not_a_marker() {
        assert!(!holds_conflict_marker("Heading\n=======\nbody\n"));
    }
}
