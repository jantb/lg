use ratatui::style::Color;

pub const OLLAMA_CHAT_ENDPOINT: &str = "http://localhost:11434/api/chat";
pub const LLM_TEMPERATURE: f32 = 0.2;
pub const LLM_TOP_P: f32 = 0.9;
pub const LLM_NUM_PREDICT: i32 = 160;
pub const LLM_MODEL: &str = "qwen3.8:27b-nvfp4";
pub const LLM_MODEL_CHOICES: &[&str] = &["qwen3.8:27b-nvfp4"];
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
pub const DEFAULT_PUSH_REMOTE: &str = "origin";
pub const BRANCH_MAIN: &str = "main";
pub const BRANCH_DEV: &str = "develop";
/// Second spelling of the dev deploy branch. Checkouts name it either way, and
/// both deploy the same environment, so lg accepts whichever one exists.
pub const BRANCH_DEV_SHORT: &str = "dev";
pub const BRANCH_TEST: &str = "test";
/// Dev deploy branch names in preference order.
pub const DEV_BRANCH_NAMES: [&str; 2] = [BRANCH_DEV, BRANCH_DEV_SHORT];

/// Whether the name is one lg deploys from rather than treating as a feature
/// branch. A repository needs only one of them: alv.no deploys `test` and has
/// no develop branch, so the names are checked one at a time.
pub fn is_deploy_branch_name(name: &str) -> bool {
    name == BRANCH_TEST || DEV_BRANCH_NAMES.contains(&name)
}

/// Deploy branches plus `main`, the branches lg refuses to treat as a feature
/// branch.
pub fn is_protected_branch_name(name: &str) -> bool {
    name == BRANCH_MAIN || is_deploy_branch_name(name)
}

/// Deploy branch names for error messages, in promotion order.
pub fn deploy_branch_list() -> String {
    let mut names = DEV_BRANCH_NAMES.to_vec();
    names.push(BRANCH_TEST);
    names.join(", ")
}

/// Protected branch names for error messages, in promotion order.
pub fn protected_branch_list() -> String {
    format!("{BRANCH_MAIN}, {}", deploy_branch_list())
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
pub const BACKGROUND_FETCH_INTERVAL_SECS: u64 = 300;
pub const COMMIT_LIST_LIMIT: usize = 200;
pub const LEFT_COLUMN_WIDTH: u16 = 64;
pub const DIFF_PAGE: u16 = 20;
