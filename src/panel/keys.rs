//! Every key binding lg documents, in one table.
//!
//! The help overlay, the footer and the unbound-key hint all read it, so a key
//! documented here reads the same way in each of them.

use crate::state::{MainKeys, Pane};

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

pub const SECTIONS: &[Section] = &[
    Section {
        title: "Global",
        pane: None,
        bindings: &[
            Binding {
                key: "?",
                help: "Toggle help",
                footer: Some(("?", "help")),
            },
            Binding {
                key: "Esc",
                help: "Dismiss an error, or cancel running LLM work",
                footer: Some(("Esc", "back")),
            },
            Binding {
                key: "Ctrl-C / q",
                help: "Quit",
                footer: Some(("q", "quit")),
            },
            Binding {
                key: "1/2/3/4",
                help: "Focus Status/Files/Branches/Commits",
                footer: None,
            },
            Binding {
                key: "0",
                help: "Focus Diff",
                footer: None,
            },
            Binding {
                key: "Tab/Shift-Tab",
                help: "Cycle focus",
                footer: None,
            },
            Binding {
                key: "a",
                help: "Edit author settings",
                footer: Some(("a", "author")),
            },
            Binding {
                key: "c",
                help: "Open commit modal",
                footer: Some(("c", "commit")),
            },
            Binding {
                key: "f",
                help: "Fetch remote updates",
                footer: Some(("f", "fetch")),
            },
            Binding {
                key: "L",
                help: "Model and per-checkout settings",
                footer: Some(("L", "model")),
            },
            Binding {
                key: "p",
                help: "Pull current branch when behind",
                footer: Some(("p", "pull")),
            },
            Binding {
                key: "P",
                help: "Push current branch",
                footer: Some(("P", "push")),
            },
            Binding {
                key: "R",
                help: "Enter review mode against main",
                footer: None,
            },
            Binding {
                key: "Ctrl-n / Ctrl-p",
                help: "Next or previous session (Ctrl-] first)",
                footer: None,
            },
            Binding {
                key: "click pane",
                help: "Focus pane",
                footer: None,
            },
            Binding {
                key: "drag divider",
                help: "Resize columns or rows",
                footer: None,
            },
            Binding {
                key: "F2",
                help: "Swap between the git and workspace views",
                footer: Some(("F2", "workspace")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Files",
        pane: Some(Pane::Files),
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move up/down",
                footer: None,
            },
            Binding {
                key: "space / y",
                help: "Stage selected",
                footer: Some(("space", "stage")),
            },
            Binding {
                key: "u",
                help: "Unstage selected",
                footer: Some(("u", "unstage")),
            },
            Binding {
                key: "A / U",
                help: "Stage all / unstage all",
                footer: Some(("A/U", "all")),
            },
            Binding {
                key: "r",
                help: "Roll back file or folder (confirms)",
                footer: Some(("r", "rollback")),
            },
            Binding {
                key: "i",
                help: "Add selected file or folder to .gitignore",
                footer: Some(("i", "ignore")),
            },
            Binding {
                key: "d",
                help: "Delete file or folder (confirms first)",
                footer: Some(("d", "delete")),
            },
            Binding {
                key: "o",
                help: "Open file or project in IntelliJ/RustRover",
                footer: Some(("o", "open IDE")),
            },
            Binding {
                key: "Enter",
                help: "Refresh diff",
                footer: None,
            },
            Binding {
                key: "g / G",
                help: "First / last row",
                footer: None,
            },
            Binding {
                key: "Ctrl-d / Ctrl-u",
                help: "Half a page down / up",
                footer: None,
            },
            Binding {
                key: "PgDn / PgUp",
                help: "A whole page down / up",
                footer: None,
            },
        ],
        footer_meta: Some((2, "Files")),
        footer_order: &[
            "space / y",
            "u",
            "A / U",
            "r",
            "i",
            "d",
            "o",
            "c",
            "a",
            "L",
            "p",
            "P",
            "f",
            "?",
        ],
    },
    Section {
        title: "Repositories",
        pane: Some(Pane::Status),
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move up/down",
                footer: Some(("j/k", "repo tree")),
            },
            Binding {
                key: "Enter",
                help: "Select repo, worktree, session, or branch",
                footer: Some(("Enter", "expand/checkout")),
            },
            Binding {
                key: "n",
                help: "New worktree for a branch",
                footer: Some(("n", "new worktree")),
            },
            Binding {
                key: "s",
                help: "Start or show a sandboxed session here",
                footer: Some(("s", "claude session")),
            },
            Binding {
                key: "S",
                help: "Session without the sandbox, in auto mode",
                footer: None,
            },
            Binding {
                key: "D",
                help: "Remove worktree, or prune a missing one",
                footer: None,
            },
            Binding {
                key: "x",
                help: "Stop the selected session and forget it",
                footer: Some(("x", "close session")),
            },
            Binding {
                key: "m",
                help: "Merge worktree into main, then clean up",
                footer: Some(("m", "land worktree")),
            },
            Binding {
                key: "M",
                help: "Merge main into the worktree's branch",
                footer: Some(("M", "sync main")),
            },
            Binding {
                key: "b",
                help: "Move its branch to the main checkout",
                footer: Some(("b", "branch home")),
            },
            Binding {
                key: "o",
                help: "Open selected repository in editor",
                footer: Some(("o", "open IDE")),
            },
            Binding {
                key: "r",
                help: "Toggle local/remote branches",
                footer: Some(("r", "remotes")),
            },
            Binding {
                key: "Esc/Backspace",
                help: "Collapse expanded repository",
                footer: None,
            },
            Binding {
                key: "g / G",
                help: "First / last row",
                footer: None,
            },
            Binding {
                key: "Ctrl-d / Ctrl-u",
                help: "Half a page down / up",
                footer: None,
            },
            Binding {
                key: "PgDn / PgUp",
                help: "A whole page down / up",
                footer: None,
            },
        ],
        footer_meta: Some((1, "Status")),
        footer_order: &[
            "j/k",
            "Enter",
            "n",
            "m",
            "M",
            "b",
            "s",
            "x",
            "o",
            "r",
            "Esc",
            "f",
            "a",
            "L",
            "p",
            "F2",
            "?",
            "Ctrl-C / q",
        ],
    },
    Section {
        title: "Branches",
        pane: Some(Pane::Branches),
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move up/down",
                footer: None,
            },
            Binding {
                key: "Enter",
                help: "Checkout branch or remote tracking branch",
                footer: Some(("Enter", "checkout")),
            },
            Binding {
                key: "o",
                help: "Open repository in editor",
                footer: Some(("o", "open IDE")),
            },
            Binding {
                key: "u",
                help: "Set upstream to matching remote branch",
                footer: Some(("u", "set upstream")),
            },
            Binding {
                key: "r",
                help: "Toggle local and remote branch views",
                footer: Some(("r", "remotes")),
            },
            Binding {
                key: "m",
                help: "Pull main or merge origin/main",
                footer: Some(("m", "pull/merge main")),
            },
            Binding {
                key: "M",
                help: "Merge main into all branches and push",
                footer: Some(("M", "sync all")),
            },
            Binding {
                key: "d",
                help: "Delete local branch with no upstream",
                footer: Some(("d", "drop local")),
            },
            Binding {
                key: "D",
                help: "Delete branch: local/remote/force options",
                footer: Some(("D", "delete")),
            },
            Binding {
                key: "F",
                help: "Branch action menu",
                footer: Some(("F", "actions")),
            },
            Binding {
                key: "g / G",
                help: "First / last row",
                footer: None,
            },
            Binding {
                key: "Ctrl-d / Ctrl-u",
                help: "Half a page down / up",
                footer: None,
            },
            Binding {
                key: "PgDn / PgUp",
                help: "A whole page down / up",
                footer: None,
            },
        ],
        footer_meta: Some((3, "Branches")),
        footer_order: &[
            "Enter", "r", "m", "M", "d", "D", "o", "u", "p", "a", "L", "f", "F", "?",
        ],
    },
    Section {
        title: "Commits",
        pane: Some(Pane::Commits),
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move up/down (auto-diff)",
                footer: Some(("j/k", "navigate")),
            },
            Binding {
                key: "Enter",
                help: "Focus diff pane",
                footer: Some(("Enter", "focus diff")),
            },
            Binding {
                key: "g / G",
                help: "First / last row",
                footer: None,
            },
            Binding {
                key: "Ctrl-d / Ctrl-u",
                help: "Half a page down / up",
                footer: None,
            },
            Binding {
                key: "PgDn / PgUp",
                help: "A whole page down / up",
                footer: None,
            },
        ],
        footer_meta: Some((4, "Commits")),
        footer_order: &["j/k", "Enter", "p", "a", "L", "f", "?"],
    },
    Section {
        title: "Session",
        pane: None,
        bindings: &[
            Binding {
                key: "i / Enter",
                help: "Give the keyboard to the session",
                footer: None,
            },
            Binding {
                key: "Ctrl-]",
                help: "Take the keyboard back; claude runs on",
                footer: None,
            },
            Binding {
                key: "x",
                help: "Close the session \u{2014} Ctrl-] first",
                footer: None,
            },
            Binding {
                key: "Backspace",
                help: "Back to the diff; session keeps running",
                footer: None,
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Review mode",
        pane: Some(Pane::Main),
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move selected review item",
                footer: Some(("j/k", "move")),
            },
            Binding {
                key: "Enter / s",
                help: "Toggle source for selected item",
                footer: Some(("Enter/s", "source")),
            },
            Binding {
                key: "space",
                help: "Expand or collapse selected item",
                footer: Some(("space", "expand")),
            },
            Binding {
                key: "d",
                help: "Drill into first child item",
                footer: Some(("d", "drill")),
            },
            Binding {
                key: "n / N",
                help: "Jump next or previous inline review note",
                footer: Some(("n/N", "notes")),
            },
            Binding {
                key: "o",
                help: "Open selected source file in IDE",
                footer: Some(("o", "open IDE")),
            },
            Binding {
                key: "f",
                help: "Run style flag pass",
                footer: Some(("f", "flag")),
            },
            Binding {
                key: "l",
                help: "Explain or generate PR text",
                footer: Some(("l", "explain")),
            },
            Binding {
                key: "y",
                help: "Copy LLM/PR text",
                footer: None,
            },
            Binding {
                key: "C",
                help: "Chat with LLM about the full review",
                footer: Some(("C", "chat")),
            },
            Binding {
                key: "g / G",
                help: "Top / bottom",
                footer: None,
            },
            Binding {
                key: "v",
                help: "Toggle unified or side-by-side diff",
                footer: Some(("v", "view")),
            },
            Binding {
                key: "R",
                help: "Rebuild assisted review",
                footer: Some(("R", "refresh")),
            },
            Binding {
                key: "Esc",
                help: "Cancel running LLM work",
                footer: Some(("Esc", "cancel")),
            },
            Binding {
                key: "g/G",
                help: "Jump to top or bottom",
                footer: Some(("g/G", "top/bot")),
            },
        ],
        footer_meta: Some((0, "Review")),
        footer_order: &[
            "j/k",
            "Enter / s",
            "space",
            "d",
            "n / N",
            "o",
            "l",
            "C",
            "g/G",
            "v",
            "f",
            "a",
            "L",
            "R",
            "Esc",
            "?",
        ],
    },
    Section {
        title: "Diff pane",
        pane: Some(Pane::Main),
        bindings: &[
            Binding {
                key: "j/k",
                help: "Scroll line",
                footer: Some(("j/k", "scroll")),
            },
            Binding {
                key: "Ctrl-d/Ctrl-u",
                help: "Scroll half page",
                footer: None,
            },
            Binding {
                key: "g / G",
                help: "Top / bottom",
                footer: None,
            },
            Binding {
                key: "R",
                help: "Enter review mode against main",
                footer: Some(("R", "review mode")),
            },
            Binding {
                key: "v",
                help: "Toggle unified or side-by-side diff",
                footer: Some(("v", "view")),
            },
            Binding {
                key: "o",
                help: "Open current source file in IDE",
                footer: Some(("o", "open IDE")),
            },
            Binding {
                key: "wheel",
                help: "Scroll 3 lines (mouse)",
                footer: None,
            },
            Binding {
                key: "Shift+drag",
                help: "Select text (terminal native)",
                footer: None,
            },
            Binding {
                key: "g/G",
                help: "Jump to top or bottom",
                footer: Some(("g/G", "top/bot")),
            },
        ],
        footer_meta: Some((0, "Diff")),
        footer_order: &["R", "v", "o", "j/k", "g/G", "p", "a", "L", "f", "?"],
    },
    Section {
        title: "Review chat",
        pane: None,
        bindings: &[
            Binding {
                key: "Enter",
                help: "Send prompt",
                footer: Some(("Enter", "send")),
            },
            Binding {
                key: "Esc",
                help: "Cancel the running answer, then close chat",
                footer: Some(("Esc", "close")),
            },
            Binding {
                key: "Ctrl+A / Ctrl+E",
                help: "Start / end of prompt",
                footer: None,
            },
            Binding {
                key: "Up / Down",
                help: "Scroll conversation",
                footer: None,
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Commit modal",
        pane: None,
        bindings: &[
            Binding {
                key: "Ctrl+S",
                help: "Commit",
                footer: Some(("Ctrl+S", "commit")),
            },
            Binding {
                key: "Ctrl+P",
                help: "Commit and push",
                footer: None,
            },
            Binding {
                key: "Enter",
                help: "New line",
                footer: Some(("Enter", "newline")),
            },
            Binding {
                key: "Ctrl+R",
                help: "Regenerate message",
                footer: Some(("Ctrl+R", "regen")),
            },
            Binding {
                key: "Ctrl+U",
                help: "Clear message",
                footer: Some(("Ctrl+U", "clear")),
            },
            Binding {
                key: "Ctrl+A / Ctrl+E",
                help: "Start or end of the line",
                footer: None,
            },
            Binding {
                key: "Home / End",
                help: "Start or end of the message",
                footer: None,
            },
            Binding {
                key: "Arrows",
                help: "Move the cursor",
                footer: None,
            },
            Binding {
                key: "Backspace",
                help: "Delete char",
                footer: None,
            },
            Binding {
                key: "Delete",
                help: "Delete the char to the right",
                footer: None,
            },
            Binding {
                key: "Esc",
                help: "Cancel",
                footer: Some(("Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Stage all",
        pane: None,
        bindings: &[
            Binding {
                key: "y",
                help: "Stage everything, then commit",
                footer: Some(("y", "stage all")),
            },
            Binding {
                key: "n / Esc",
                help: "Cancel",
                footer: Some(("n/Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "New worktree",
        pane: None,
        bindings: &[
            Binding {
                key: "Tab / arrows",
                help: "Switch field",
                footer: Some(("Tab", "field")),
            },
            Binding {
                key: "Enter",
                help: "Create the worktree",
                footer: Some(("Enter", "create")),
            },
            Binding {
                key: "Esc",
                help: "Cancel",
                footer: Some(("Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Branch actions",
        pane: None,
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move between actions",
                footer: Some(("j/k", "select")),
            },
            Binding {
                key: "Enter",
                help: "Run the selected action",
                footer: Some(("Enter", "continue")),
            },
            Binding {
                key: "Esc",
                help: "Back",
                footer: Some(("Esc", "back")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Conflict",
        pane: None,
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move between conflicted files",
                footer: Some(("j/k", "select")),
            },
            Binding {
                key: "o / Enter",
                help: "Open the file in the editor",
                footer: Some(("o/Enter", "open")),
            },
            Binding {
                key: "v",
                help: "Check the resolution, then continue",
                footer: Some(("v", "validate")),
            },
            Binding {
                key: "a",
                help: "Abort merge, rebase or cherry-pick",
                footer: Some(("a", "abort")),
            },
            Binding {
                key: "Esc",
                help: "Close",
                footer: Some(("Esc", "close")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Delete branch",
        pane: None,
        bindings: &[
            Binding {
                key: "Tab / j/k",
                help: "Move between the delete options",
                footer: Some(("Tab", "field")),
            },
            Binding {
                key: "space",
                help: "Toggle the selected option",
                footer: Some(("Space", "toggle")),
            },
            Binding {
                key: "Enter",
                help: "Delete with the chosen options",
                footer: Some(("Enter", "confirm")),
            },
            Binding {
                key: "Esc",
                help: "Cancel",
                footer: Some(("Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Help overlay",
        pane: None,
        bindings: &[
            Binding {
                key: "j/k",
                help: "Scroll",
                footer: Some(("j/k", "scroll")),
            },
            Binding {
                key: "Ctrl-d / Ctrl-u",
                help: "Half a page down / up",
                footer: None,
            },
            Binding {
                key: "g / G",
                help: "Top / bottom",
                footer: Some(("g/G", "top/bot")),
            },
            Binding {
                key: "q / Esc",
                help: "Close",
                footer: Some(("q/Esc", "close")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Author settings",
        pane: None,
        bindings: &[
            Binding {
                key: "Tab / arrows",
                help: "Switch field",
                footer: Some(("Tab", "field")),
            },
            Binding {
                key: "Enter",
                help: "Save subtree rule",
                footer: Some(("Enter", "save subtree")),
            },
            Binding {
                key: "Ctrl+L",
                help: "Save repo-local author",
                footer: Some(("Ctrl+L", "save local")),
            },
            Binding {
                key: "Ctrl+U",
                help: "Clear subtree rule",
                footer: Some(("Ctrl+U", "clear subtree")),
            },
            Binding {
                key: "Ctrl+X",
                help: "Clear repo-local author",
                footer: Some(("Ctrl+X", "clear local")),
            },
            Binding {
                key: "Esc",
                help: "Cancel",
                footer: Some(("Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Settings",
        pane: None,
        bindings: &[
            Binding {
                key: "Tab / Shift-Tab",
                help: "Move between fields",
                footer: None,
            },
            Binding {
                key: "Up / Down",
                help: "Move field, or step the open one",
                footer: Some(("Up/Down", "select")),
            },
            Binding {
                key: "Enter",
                help: "Open the field, or run what it does",
                footer: Some(("Enter", "open/confirm")),
            },
            Binding {
                key: "Ctrl+S",
                help: "Save model and per-checkout settings",
                footer: Some(("Ctrl+S", "save")),
            },
            Binding {
                key: "Ctrl+E",
                help: "Edit the commit-message prompt",
                footer: None,
            },
            Binding {
                key: "Ctrl+U",
                help: "Reset saved settings to defaults",
                footer: Some(("Ctrl+U", "reset")),
            },
            Binding {
                key: "Esc",
                help: "Leave the open field, then close",
                footer: Some(("Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Confirm prompts",
        pane: None,
        bindings: &[
            Binding {
                key: "y",
                help: "Confirm the destructive action",
                footer: Some(("y", "confirm")),
            },
            Binding {
                key: "n / Esc",
                help: "Cancel",
                footer: Some(("n/Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
    Section {
        title: "Push modal",
        pane: None,
        bindings: &[
            Binding {
                key: "Enter",
                help: "Push to origin",
                footer: Some(("Enter", "push")),
            },
            Binding {
                key: "Esc",
                help: "Cancel",
                footer: Some(("Esc", "cancel")),
            },
        ],
        footer_meta: None,
        footer_order: &[],
    },
];

/// Every modal footer, in the order its keys are printed.
pub const MODAL_FOOTERS: &[ModalFooter] = &[
    ModalFooter {
        section: "Commit modal",
        prefix: "Commit modal ",
        tone: Tone::Normal,
        order: &["Ctrl+S", "Enter", "Ctrl+R", "Ctrl+U", "Esc"],
    },
    ModalFooter {
        section: "Stage all",
        prefix: "Commit ",
        tone: Tone::Caution,
        order: &["y", "n / Esc"],
    },
    ModalFooter {
        section: "Push modal",
        prefix: "Push modal ",
        tone: Tone::Normal,
        order: &["Enter", "Esc"],
    },
    ModalFooter {
        section: "New worktree",
        prefix: "New worktree ",
        tone: Tone::Normal,
        order: &["Tab / arrows", "Enter", "Esc"],
    },
    ModalFooter {
        section: "Author settings",
        prefix: "Author ",
        tone: Tone::Normal,
        order: &["Tab / arrows", "Enter", "Ctrl+L", "Ctrl+U", "Ctrl+X", "Esc"],
    },
    ModalFooter {
        section: "Settings",
        prefix: "Settings ",
        tone: Tone::Normal,
        order: &["Up / Down", "Enter", "Ctrl+S", "Ctrl+U", "Esc"],
    },
    ModalFooter {
        section: "Help overlay",
        prefix: "Help ",
        tone: Tone::Normal,
        order: &["j/k", "g / G", "q / Esc"],
    },
    ModalFooter {
        section: "Branch actions",
        prefix: "Branches ",
        tone: Tone::Normal,
        order: &["j/k", "Enter", "Esc"],
    },
    ModalFooter {
        section: "Conflict",
        prefix: "Conflict ",
        tone: Tone::Danger,
        order: &["j/k", "o / Enter", "v", "a", "Esc"],
    },
    ModalFooter {
        section: "Delete branch",
        prefix: "Delete branch ",
        tone: Tone::Danger,
        order: &["Tab / j/k", "space", "Enter", "Esc"],
    },
    ModalFooter {
        section: "Confirm prompts",
        prefix: "Confirm ",
        tone: Tone::Danger,
        order: &["y", "n / Esc"],
    },
    ModalFooter {
        section: "Review chat",
        prefix: "Review chat ",
        tone: Tone::Normal,
        order: &["Enter", "Esc"],
    },
];

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
