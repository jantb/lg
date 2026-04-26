use ratatui::style::Color;

pub const OLLAMA_CHAT_ENDPOINT: &str = "http://localhost:11434/api/chat";
pub const OLLAMA_TEMPERATURE: f32 = 0.2;
pub const OLLAMA_TOP_P: f32 = 0.9;
pub const OLLAMA_TOP_K: u32 = 40;
pub const OLLAMA_MIN_P: f32 = 0.0;
pub const OLLAMA_NUM_CTX: u32 = 16_384;
pub const OLLAMA_NUM_PREDICT: i32 = 256;
pub const OLLAMA_REPEAT_PENALTY: f32 = 1.05;
pub const OLLAMA_PRESENCE_PENALTY: f32 = 0.0;
pub const OLLAMA_MODEL: &str = "qwen3.6:35b-a3b-coding-nvfp4";
pub const OLLAMA_PROMPT_PREFIX: &str = "\
Write ONE Conventional Commits subject line for this git diff.

Rules:
- Format: `type(scope): summary` — scope is optional.
- type is one of: feat, fix, refactor, perf, docs, test, chore, build, ci, style.
- Imperative mood, lowercase summary, at most 72 characters, no trailing period.
- Describe the behavior change, not the files touched. Be specific.
- Output ONLY the subject line. No prose, no quotes, no markdown, no code fences.

Examples:
- feat(ollama): stream commit-message tokens
- fix(git): include untracked files in porcelain parse
- refactor(state): interleave dirs and files in tree rows
- perf(ollama): reuse shared http client across requests

Diff:

";
pub const DEFAULT_PUSH_REMOTE: &str = "origin";
pub const COMMIT_MSG_MAX_CHARS: usize = 2048;
pub const COMMIT_MSG_GEN_MAX_CHARS: usize = 72;
pub const STATUS_BAR_HEIGHT: u16 = 1;
pub const STATUS_MSG_LIFETIME_SECS: i64 = 3;
pub const BORDER_COLOR: Color = Color::LightBlue;
pub const TICK_MS: u64 = 250;
pub const COMMIT_LIST_LIMIT: usize = 200;
pub const LEFT_COLUMN_WIDTH: u16 = 32;
pub const DIFF_PAGE: u16 = 20;
