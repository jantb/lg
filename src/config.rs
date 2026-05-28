use ratatui::style::Color;

pub const OLLAMA_CHAT_ENDPOINT: &str = "http://localhost:11434/api/chat";
pub const LLM_TEMPERATURE: f32 = 0.2;
pub const LLM_TOP_P: f32 = 0.9;
pub const LLM_NUM_PREDICT: i32 = 160;
pub const LLM_MODEL: &str = "qwen3.6:27b-coding-nvfp4";
pub const LLM_MODEL_CHOICES: &[&str] =
    &["qwen3.6:27b-coding-nvfp4", "qwen3.6:35b-a3b-coding-nvfp4"];
pub const COMMIT_PROMPT_PREFIX: &str = "\
Write a concise commit message for these staged changes.

Rules:
- First line format: `type(scope): summary` — scope is optional.
- type is one of: feat, fix, refactor, perf, docs, test, chore, build, ci, style.
- First line uses imperative mood, lowercase summary, and no trailing period.
- Keep the first line complete; do not omit important detail to fit a fixed character limit.
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
pub const BRANCH_TEST: &str = "release/next";
pub const STATUS_BAR_HEIGHT: u16 = 1;
pub const STATUS_MSG_LIFETIME_SECS: i64 = 3;
pub const BORDER_COLOR: Color = Color::LightBlue;
pub const TICK_MS: u64 = 250;
pub const BACKGROUND_FETCH_INTERVAL_SECS: u64 = 300;
pub const COMMIT_LIST_LIMIT: usize = 200;
pub const LEFT_COLUMN_WIDTH: u16 = 64;
pub const DIFF_PAGE: u16 = 20;
