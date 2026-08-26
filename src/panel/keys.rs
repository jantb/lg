//! Every key binding lg documents, in one table.
//!
//! The help overlay, the footer and the unbound-key hint all read it, so a key
//! documented here reads the same way in each of them.

use crate::state::{MainKeys, Pane};

mod table;

pub use table::{MODAL_FOOTERS, SECTIONS};

pub struct Binding {
    /// How the help overlay names the keys.
    pub key: &'static str,
    /// The sentence the help overlay shows. Kept within [`DESC_WIDTH`], which
    /// the overlay does not wrap past.
    pub help: &'static str,
    /// The short key string and label the footer uses, when it shows this
    /// binding at all. The footer has room for a word, not a sentence.
    pub footer: Option<(&'static str, &'static str)>,
}

pub struct Section {
    pub title: &'static str,
    /// The pane whose heading this section is highlighted under, when it
    /// belongs to one.
    pub pane: Option<Pane>,
    pub bindings: &'static [Binding],
    /// The pane number and name the footer labels itself with, for the
    /// sections a footer is built from.
    pub footer_meta: Option<(u8, &'static str)>,
    /// Keys this pane's footer shows, in the order it shows them. Each resolves
    /// against this section first and then Global, so a pane repeats a global
    /// key without restating what it does.
    pub footer_order: &'static [&'static str],
}

/// How much care a footer's colour should ask for. Named by meaning rather than
/// by colour, so the mapping to a terminal colour stays in the footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    /// A step that stages something before it happens.
    Caution,
    /// Something destructive.
    Danger,
}

/// A modal's footer. The help overlay and the footer both read it, so a modal
/// documents each of its keys once.
pub struct ModalFooter {
    /// The section whose bindings this footer prints.
    pub section: &'static str,
    /// The words in front of the keys.
    pub prefix: &'static str,
    pub tone: Tone,
    /// Keys to print, in order, resolved against the named section.
    pub order: &'static [&'static str],
}

/// The section with this title.
pub fn section(title: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|section| section.title == title)
}

/// The modal footer for the section with this title.
pub fn modal_footer(section: &str) -> Option<&'static ModalFooter> {
    MODAL_FOOTERS.iter().find(|modal| modal.section == section)
}

/// The section whose bindings are live right now.
///
/// The main pane holds three of them, so `keys` decides which. The footer, the
/// help overlay and the unbound-key hint all ask this, which is what keeps them
/// describing the same set.
pub fn active_section(pane: Pane, keys: MainKeys) -> Option<&'static Section> {
    section(match pane {
        Pane::Status => "Repositories",
        Pane::Files => "Files",
        Pane::Branches => "Branches",
        Pane::Commits => "Commits",
        Pane::Main => match keys {
            MainKeys::Session => "Session",
            MainKeys::Review => "Review mode",
            MainKeys::Diff => "Diff pane",
        },
    })
}

/// What the footer calls a section — the name the user reads on screen, which
/// is not always the title the help gives it.
pub fn footer_label(section: &Section) -> &'static str {
    match section.footer_meta {
        Some((_, name)) => name,
        None => section.title,
    }
}

/// Body lines `title`'s section starts at in the help overlay, counting the
/// heading and blank line each earlier section takes.
pub fn section_line(title: &str) -> u16 {
    let mut line = 0u16;
    for section in SECTIONS {
        if section.title == title {
            break;
        }
        // Heading, bindings, and the blank line before the next section.
        line = line.saturating_add(2 + section.bindings.len() as u16);
    }
    line
}

/// What the footer prints for one of `section`'s footer keys: the section's own
/// binding when it has one, otherwise the global binding of the same name.
pub fn footer_entry(from: &Section, key: &str) -> Option<(&'static str, &'static str)> {
    let named = |haystack: &Section| {
        haystack
            .bindings
            .iter()
            .find(|binding| binding.key == key)
            .and_then(|binding| binding.footer)
    };
    named(from).or_else(|| section("Global").and_then(named))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The main pane is three panes wearing one name, and each has its own keys.
    #[test]
    fn the_main_pane_reports_the_key_set_that_is_live() {
        let title = |keys| active_section(Pane::Main, keys).map(|section| section.title);
        assert_eq!(title(MainKeys::Diff), Some("Diff pane"));
        assert_eq!(title(MainKeys::Review), Some("Review mode"));
        assert_eq!(title(MainKeys::Session), Some("Session"));
    }

    /// Every pane's footer, hint for hint, in the order it prints them. The
    /// shared table has to rebuild each one exactly.
    #[test]
    fn the_shared_table_rebuilds_every_footer_unchanged() {
        /// A section title, the pane number and name its footer carries, and
        /// the key/label pairs it prints.
        type FooterSnapshot = (
            &'static str,
            u8,
            &'static str,
            &'static [(&'static str, &'static str)],
        );

        let expected: &[FooterSnapshot] = &[
            (
                "Repositories",
                1,
                "Status",
                &[
                    ("j/k", "repo tree"),
                    ("Enter", "expand/checkout"),
                    ("n", "new worktree"),
                    ("m", "land worktree"),
                    ("M", "sync main"),
                    ("b", "branch home"),
                    ("s", "claude session"),
                    ("t", "terminal"),
                    ("x", "close session"),
                    ("o", "open IDE"),
                    ("r", "remotes"),
                    ("Esc", "back"),
                    ("f", "fetch"),
                    ("a", "author"),
                    ("L", "model"),
                    ("p", "pull"),
                    ("F2", "workspace"),
                    ("?", "help"),
                    ("q", "quit"),
                ][..],
            ),
            (
                "Files",
                2,
                "Files",
                &[
                    ("space", "stage"),
                    ("u", "unstage"),
                    ("A/U", "all"),
                    ("r", "rollback"),
                    ("i", "ignore"),
                    ("d", "delete"),
                    ("o", "open IDE"),
                    ("c", "commit"),
                    ("a", "author"),
                    ("L", "model"),
                    ("p", "pull"),
                    ("P", "push"),
                    ("f", "fetch"),
                    ("?", "help"),
                ][..],
            ),
            (
                "Branches",
                3,
                "Branches",
                &[
                    ("Enter", "checkout"),
                    ("r", "remotes"),
                    ("m", "pull/merge main"),
                    ("M", "sync all"),
                    ("d", "drop local"),
                    ("D", "delete"),
                    ("o", "open IDE"),
                    ("u", "set upstream"),
                    ("p", "pull"),
                    ("a", "author"),
                    ("L", "model"),
                    ("f", "fetch"),
                    ("F", "actions"),
                    ("?", "help"),
                ][..],
            ),
            (
                "Commits",
                4,
                "Commits",
                &[
                    ("j/k", "navigate"),
                    ("Enter", "focus diff"),
                    ("p", "pull"),
                    ("a", "author"),
                    ("L", "model"),
                    ("f", "fetch"),
                    ("?", "help"),
                ][..],
            ),
            (
                "Review mode",
                0,
                "Review",
                &[
                    ("j/k", "move"),
                    ("Enter/s", "source"),
                    ("space", "expand"),
                    ("d", "drill"),
                    ("n/N", "notes"),
                    ("o", "open IDE"),
                    ("l", "explain"),
                    ("C", "chat"),
                    ("g/G", "top/bot"),
                    ("v", "view"),
                    ("f", "flag"),
                    ("a", "author"),
                    ("L", "model"),
                    ("R", "refresh"),
                    ("Esc", "cancel"),
                    ("?", "help"),
                ][..],
            ),
            (
                "Diff pane",
                0,
                "Diff",
                &[
                    ("R", "review mode"),
                    ("v", "view"),
                    ("o", "open IDE"),
                    ("j/k", "scroll"),
                    ("g/G", "top/bot"),
                    ("p", "pull"),
                    ("a", "author"),
                    ("L", "model"),
                    ("f", "fetch"),
                    ("?", "help"),
                ][..],
            ),
        ];

        for (title, index, name, pairs) in expected {
            let section = section(title).expect(title);
            assert_eq!(
                section.footer_meta,
                Some((*index, *name)),
                "{title} labels its footer differently"
            );
            let rebuilt: Vec<_> = section
                .footer_order
                .iter()
                .map(|key| footer_entry(section, key).expect(key))
                .collect();
            assert_eq!(&rebuilt, pairs, "{title} footer changed");
        }
    }
}
