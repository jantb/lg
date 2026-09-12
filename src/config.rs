use ratatui::style::Color;

pub const MTPLX_CHAT_ENDPOINT: &str = "http://localhost:8000/v1/chat/completions";
pub const LLM_TEMPERATURE: f32 = 0.2;
pub const LLM_TOP_P: f32 = 0.9;
pub const LLM_NUM_PREDICT: i32 = 160;
pub const LLM_MODEL: &str = "mtplx-qwen38-27b-optimized-speed";
pub const LLM_MODEL_CHOICES: &[&str] = &["mtplx-qwen38-27b-optimized-speed"];
/// Languages offered for generated prose. Free text still works, so this is a
/// shortcut for the common picks rather than a closed set.
pub const PR_LANGUAGE_CHOICES: &[&str] = &[
    "English",
    "Norwegian",
    "Swedish",
    "Danish",
    "German",
    "French",
    "Spanish",
];
pub const COMMIT_PROMPT_PREFIX: &str = "\
Write a concise commit message for these staged changes.

Rules:
- First line format: `type(scope): summary` — scope is optional.
- type is one of: feat, fix, refactor, perf, docs, test, chore, build, ci, style.
- First line uses imperative mood, lowercase summary, and no trailing period.
- Keep the first line complete; drop detail only where a stated length limit requires it.
- Describe the behavior change, not the files touched. Be specific.
- Prefer concrete user-visible outcomes over vague words like update, improve, or change.
- Use the change summary first; use the diff excerpt only for extra detail.
- For non-trivial changes, include a short body after a blank line.
- Detail lines should explain the important behavior, condition, control-flow path, or test coverage.
- Prefer one line only when the staged diff is tiny and obvious.
- Do not use emoji.
- Output ONLY the commit message. No prose, no quotes, no markdown, no code fences.

Examples:
- feat(llm): stream commit-message tokens
- fix(git): include untracked files in porcelain parse
- refactor(state): interleave dirs and files in tree rows
- perf(llm): reuse shared http client across requests
- feat(tui): show staged and unstaged counts in status panel
- feat(flow): retry release validation after conflict resolution

  Add a follow-up validation path once resolved files are staged.
  Cover the new continuation branch with a release-flow test.

Staged changes:

";
/// The style guide the review tasks measure a change against, when a checkout
/// has not written one of its own.
///
/// It is deliberately about code in general rather than one stack: lg appends
/// notes for each language a review touches (see `git::Language`), and a
/// checkout that wants its own conventions drops a `review-style.md` next to
/// its commit prompt, which replaces this verbatim and turns the language
/// notes off.
pub const REVIEW_STYLE_GUIDE: &str = "\
Established repo style:
- Immutable by default: prefer values that are set once, read-only collections, and copies over in-place mutation.
- Names describe domain intent and behavior. Flag vague, misleading, or overly generic names and suggest a concrete replacement.
- Functions do one thing and stay short enough to read in one screen; extract helpers with names that say what they decide.
- Dependencies are passed in (constructor or parameter injection), never reached for through globals, singletons, or service locators.
- Layers stay honest: request handlers, controllers, adapters, and message consumers translate and validate; business rules live in the service or domain layer.
- Errors carry context and are handled where something can be done about them; never swallow one silently.
- Side effects (I/O, messaging, time, randomness) sit at the edges so the rules in the middle can be tested without them.
- Model variants with sum types (sealed types, enums with data, discriminated unions) rather than flags and nullable fields.
- Tests assert behavior a caller depends on, not the shape of the implementation; prefer small real fakes over mocks.
- Generated code is never edited by hand.
- Run the repo formatter and linter before declaring work done; the linter wins on formatting.";

pub const DEFAULT_PUSH_REMOTE: &str = "origin";
pub const BRANCH_MAIN: &str = "main";
pub const BRANCH_DEV: &str = "develop";
/// Second spelling of the dev deploy branch. Checkouts name it either way, and
/// both deploy the same environment, so lg accepts whichever one exists.
pub const BRANCH_DEV_SHORT: &str = "dev";
pub const BRANCH_TEST: &str = "test";
/// Dev deploy branch names in preference order.
pub const DEV_BRANCH_NAMES: [&str; 2] = [BRANCH_DEV, BRANCH_DEV_SHORT];

/// The branches the environments deploy from, other than the integration
/// branch: the ones a feature branch is released into. Read from the saved
/// configuration, or detected from the checkout's own branches when nothing
/// is saved, so a branch the checkout does not have is never one of them.
pub fn deploy_branches() -> Vec<String> {
    let branches = crate::preferences::load().config.branches;
    branches
        .environments
        .into_iter()
        .filter(|e| !e.branch.is_empty() && e.branch != branches.base)
        .map(|e| e.branch)
        .collect()
}

/// Whether the name is one lg deploys from rather than treating as a feature
/// branch.
pub fn is_deploy_branch_name(name: &str) -> bool {
    deploy_branches().iter().any(|b| b == name)
}

/// Deploy branches plus `main`, the branches lg refuses to treat as a feature
/// branch.
pub fn is_protected_branch_name(name: &str) -> bool {
    let branches = crate::preferences::load().config.branches;
    name == branches.base || branches.protected.iter().any(|b| b == name)
}

/// Deploy branch names for error messages, in promotion order.
pub fn deploy_branch_list() -> String {
    deploy_branches().join(", ")
}

/// Protected branch names for error messages, in promotion order.
pub fn protected_branch_list() -> String {
    let config = crate::preferences::load().config.branches;
    let mut names = config.protected;
    names.push(config.base);
    names.sort();
    names.dedup();
    names.join(", ")
}
pub const STATUS_BAR_HEIGHT: u16 = 1;
pub const STATUS_MSG_LIFETIME_SECS: i64 = 3;
/// Errors linger far longer than successes so a failure cannot scroll past unseen.
/// Esc dismisses one early.
pub const ERROR_MSG_LIFETIME_SECS: i64 = 30;
pub const BORDER_COLOR: Color = Color::LightBlue;
pub const TICK_MS: u64 = 250;

/// Poll interval while a session is on screen: its output is the echo of what
/// is being typed into it, so it has to keep up with typing.
pub const SESSION_TICK_MS: u64 = 16;

/// How long one animation step lasts. Spinners and other frame-by-frame
/// animations advance on this clock rather than once per redraw, because the
/// poll interval ranges from `ANIMATION_FRAME_MS` to `TICK_MS` and a per-frame
/// animation runs at whatever rate that happens to be.
pub const ANIMATION_STEP_MS: u64 = 120;
/// Poll interval while something on screen is moving: a pulsing frame, a
/// settling status line, a running job's spinner. Colour fades are continuous
/// in time, so the more often they are drawn the smoother they look; this is
/// about 120 frames a second.
pub const ANIMATION_FRAME_MS: u64 = 8;
/// How many queued input events one frame may take before drawing again. A
/// trackpad sends wheel events far faster than lg redraws, so they are handled
/// in a batch rather than one per frame — but a flood must not starve the
/// redraw that makes the scrolling visible.
pub const MAX_EVENTS_PER_FRAME: usize = 64;

/// How much of a frame is held before any of it reaches the terminal.
///
/// Rust hands out a stdout that flushes every kilobyte, and a frame on a large
/// window is hundreds of them: the terminal is given the picture in pieces and
/// paints each piece as it arrives, so what is on screen is part of this frame
/// beside part of the last. Worse, ratatui only ever sends the cells that
/// changed, so a piece that arrived torn stays torn until something happens to
/// write over it. A buffer that holds a whole frame makes it one write.
pub const FRAME_BUFFER_BYTES: usize = 4 * 1024 * 1024;
pub const BACKGROUND_FETCH_INTERVAL_SECS: u64 = 300;
pub const COMMIT_LIST_LIMIT: usize = 200;

/// How much context one review task may send.
///
/// Bigger is not free. Prefill runs about an order of magnitude faster than
/// decode, but it is still seconds per thousand tokens on a local model, and
/// every byte of it is paid before the first token of the answer appears. This
/// sits well under what the model can hold on purpose: the limit worth tuning
/// is the wait, not the window.
///
/// `LG_LLM_CONTEXT_BYTES` moves it, for a checkout whose diffs need more room
/// or a machine that reads them faster.
pub const DEFAULT_REVIEW_CONTEXT_BYTES: usize = 48_000;

/// The context budget in force, honouring `LG_LLM_CONTEXT_BYTES`. A value that
/// does not parse, or is too small to hold anything useful, leaves the default
/// alone rather than crippling the review.
pub fn review_context_bytes() -> usize {
    // Read once: the builders ask for it per line of context, and the
    // environment does not change under a running process.
    static BYTES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BYTES.get_or_init(|| {
        std::env::var("LG_LLM_CONTEXT_BYTES")
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .filter(|bytes| *bytes >= 4_000)
            .unwrap_or(DEFAULT_REVIEW_CONTEXT_BYTES)
    })
}
pub const LEFT_COLUMN_WIDTH: u16 = 64;
pub const DIFF_PAGE: u16 = 20;
