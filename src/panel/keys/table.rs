//! The table itself: every section, its bindings, and the modal footers.

use super::{Binding, ModalFooter, Section, Tone};
use crate::state::Pane;

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
                help: "Pick a sandboxed agent to start here",
                footer: Some(("s", "agent session")),
            },
            Binding {
                key: "S",
                help: "The same picker, without the sandbox",
                footer: None,
            },
            Binding {
                key: "t",
                help: "Start or show a sandboxed terminal here",
                footer: Some(("t", "terminal")),
            },
            Binding {
                key: "T",
                help: "Terminal without the sandbox",
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
            "t",
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
                help: "Branch actions, or back to a live conflict",
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
                help: "Take the keyboard back; the program runs on",
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
        title: "Agent picker",
        pane: None,
        bindings: &[
            Binding {
                key: "j/k",
                help: "Move between claude, codex and pi",
                footer: Some(("j/k", "select")),
            },
            Binding {
                key: "Enter",
                help: "Start the highlighted agent here",
                footer: Some(("Enter", "start")),
            },
            Binding {
                key: "c / x / p",
                help: "Start claude / codex / pi outright",
                footer: Some(("c/x/p", "claude/codex/pi")),
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
                key: "l",
                help: "Try the local model, then claude on the rest",
                footer: Some(("l", "local")),
            },
            Binding {
                key: "c",
                help: "Hand it to a sandboxed agent here",
                footer: Some(("c", "agent")),
            },
            Binding {
                key: "C",
                help: "The same, without the sandbox",
                footer: None,
            },
            Binding {
                key: "v",
                help: "Check the resolution, then continue the flow",
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
        section: "Agent picker",
        prefix: "Start agent ",
        tone: Tone::Normal,
        order: &["j/k", "Enter", "c / x / p", "Esc"],
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
        order: &["j/k", "o / Enter", "c", "v", "a", "Esc"],
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
