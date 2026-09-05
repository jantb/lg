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
/// It describes one team's Kotlin/Spring codebase, which is what these features
/// were built for. It is the default rather than the rule: a checkout drops a
/// `review-style.md` next to its commit prompt and that replaces this verbatim,
/// because telling a model to check Rust for Mockito usage is worse than
/// telling it nothing.
pub const REVIEW_STYLE_GUIDE: &str = "\
Established repo style:
- Kotlin/Spring, but immutable code by default: prefer val, immutable collections, data-class .copy(), focused functions, and pure helper functions.
- Constructor injection only. Inject narrow interfaces/services, not broad infrastructure.
- Controllers stay thin: auth, validation, DTO assembly, ResponseEntity. Business decisions go in service-layer files/classes whose path or name contains Service, or in explicit hub flow code.
- Treat business rules in controllers, adapters, Kafka consumers/listeners, repositories, DTOs, configuration, or other non-Service/non-flow files as a style issue unless the shown code only delegates or translates data.
- Flow start state construction may call repositories/services to load initial data before the flow begins. Once a flow has started, later state constructors/steps should stay pure; flag direct repository/service calls there.
- Domain IDs use inline value classes like UserId, MembershipId; wrap raw primitives at repository boundaries.
- Names should describe domain intent and behavior. Flag vague, misleading, or overly generic names and suggest a concrete replacement.
- Use sealed interfaces/classes for variants with different data; enums only for simple tags.
- JSON uses the shared configuredJson; avoid Jackson in app code except generated/Spring/Avro internals.
- Time uses kotlinx.datetime; java.time only at interop edges.
- Logging uses private val log by Logger(), not direct LoggerFactory.
- Outbound HTTP uses Ktor CIO adapters. Each external system gets one adapter.
- Persistence is PostgreSQL via Exposed + Flyway.
- Kafka/outbound side effects from flows go through the outbox, not direct Kafka publishing.
- Tests prefer real small fakes over mocks. Use Mockk only when a fake is impractical; never Mockito.
- Integration tests use @SpringBootTest + TestConfiguration + Testcontainers.
- Do not edit generated code under target/generated-sources.
- Run the repo formatter/lint before declaring work done; linter wins on formatting.";

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

/// How long one animation step lasts. Spinners advance on this clock rather
/// than once per frame, because the poll interval ranges from `SESSION_TICK_MS`
/// to `TICK_MS` and a per-frame animation runs at whatever rate that happens to
/// be.
pub const ANIMATION_STEP_MS: u64 = 120;
/// Poll interval while a background job is in flight, so its result lands
/// without waiting out a full `TICK_MS`.
pub const JOB_TICK_MS: u64 = 80;
/// How many queued input events one frame may take before drawing again. A
/// trackpad sends wheel events far faster than lg redraws, so they are handled
/// in a batch rather than one per frame — but a flood must not starve the
/// redraw that makes the scrolling visible.
pub const MAX_EVENTS_PER_FRAME: usize = 64;
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
    std::env::var("LG_LLM_CONTEXT_BYTES")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|bytes| *bytes >= 4_000)
        .unwrap_or(DEFAULT_REVIEW_CONTEXT_BYTES)
}
pub const LEFT_COLUMN_WIDTH: u16 = 64;
pub const DIFF_PAGE: u16 = 20;
