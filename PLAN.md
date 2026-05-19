# lg — Development Plan

A simplified lazygit clone in Rust using ratatui. Supports only **commit** and **push** operations with Ollama-powered commit message generation.

Git operations shell out to the `git` CLI — no libgit2 dependency. This sidesteps SSH auth, signing, and platform-specific libgit2 build pain, and inherits whatever the user already has configured.

---

## 1. Cargo.toml

```toml
[package]
name = "lg"
version = "0.1.0"
edition = "2024"
rust-version = "1.86"

[profile.dev]
debug = 1

[dependencies]
ratatui    = "0.30"
reqwest    = { version = "0.13", features = ["blocking", "json"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
chrono     = { version = "0.4", features = ["serde"] }
anyhow     = "1"

[dev-dependencies]
tempfile = "3"
```

Notes:
- `ratatui 0.30` re-exports `crossterm 0.29`; no direct crossterm dep.
- `reqwest` uses `blocking` — no tokio runtime, sync event loop throughout.
- `git2` deliberately excluded — we shell out to `git`.
- `clap` deliberately excluded — `lg` takes no CLI args.
- MSRV pinned to 1.86 (required by ratatui 0.30).

---

## 2. Feature List

### Commit flow
1. `c` opens the commit panel.
2. Panel shows: unstaged list, staged list, commit message input.
3. `y` stages highlighted file, `u` unstages. `A`/`U` stage/unstage all.
4. `o` generates a message via Ollama from the staged diff.
5. User edits the message (optional).
6. `Enter` runs `git commit -m "<message>"`.
7. On success, lists refresh; panel closes.

### Push flow
1. `p` opens the push panel.
2. Panel resolves current branch (`git rev-parse --abbrev-ref HEAD`) and remote URL (`git remote get-url origin`).
3. Displays `Branch: <name>` and `Remote: <url>`.
4. `Enter` runs `git push origin <branch>` (inherits user's SSH/HTTPS credentials from the shell).
5. Shows stdout/stderr in the status area on completion.

### Ollama integration
- Endpoint: `http://localhost:11434/api/generate`.
- Model: `qwen3.6:27b-coding-nvfp4` (installed locally; `lg` is local-only).
- Prompt = `OLLAMA_PROMPT_PREFIX` + `git diff --cached`.
- Non-streaming (`"stream": false`). Parse the `response` field.
- On any failure, fall back to an empty field the user can fill manually.

---

## 3. Key bindings

| Key | Action | Panel |
|-----|--------|-------|
| `q` / `Esc` | Quit / close panel | Global / context |
| `c` | Open commit panel | Files |
| `p` | Open push panel | Files |
| `y` / `u` | Stage / unstage selection | Files |
| `A` / `U` | Stage all / unstage all | Files |
| `o` | Generate commit message (Ollama) | Commit |
| `j` / `k` / `Down` / `Up` | Move selection | All |
| `Enter` | Confirm / commit / push | Context |
| `Tab` | Cycle focus within panel | Files, Commit |
| `?` | Show help overlay | Global |

---

## 4. Module Layout

```
src/
├── main.rs         — entry point
├── app.rs          — App struct, event loop, dispatch
├── state.rs        — AppState, Panel enum, StatusMsg
├── config.rs       — constants
├── ui.rs           — layout primitives
├── ollama.rs       — blocking HTTP call to Ollama
├── git.rs          — shell wrappers around `git`
└── panel/
    ├── mod.rs
    ├── files.rs    — staged/unstaged lists
    ├── commit.rs   — message input + commit
    ├── push.rs     — branch/remote + push
    └── help.rs     — keybinding overlay
```

### `src/main.rs`
```rust
fn main() -> anyhow::Result<()> {
    lg::app::App::new()?.run()
}
```

### `src/config.rs`
```rust
use ratatui::style::Color;

pub const OLLAMA_ENDPOINT: &str = "http://localhost:11434/api/generate";
pub const OLLAMA_MODEL: &str = "qwen3.6:27b-coding-nvfp4";
pub const OLLAMA_PROMPT_PREFIX: &str =
    "Write a concise one-line git commit message (<=72 chars) for the diff below. \
     No prose, no quotes, no trailing period.\n\n";
pub const DEFAULT_PUSH_REMOTE: &str = "origin";
pub const COMMIT_MSG_MAX_CHARS: usize = 72;
pub const STATUS_BAR_HEIGHT: u16 = 1;
pub const STATUS_MSG_LIFETIME_SECS: i64 = 3;
pub const BORDER_COLOR: Color = Color::LightBlue;
pub const TICK_MS: u64 = 250;
```

### `src/state.rs`
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel { Files, Commit, Push, Help }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFocus { Unstaged, Staged }

#[derive(Debug, Clone)]
pub struct StatusMsg {
    pub text: String,
    pub is_error: bool,
    pub at: chrono::DateTime<chrono::Utc>,
}

pub struct AppState {
    pub current_panel: Panel,
    pub previous_panel: Panel,     // for Help return
    pub unstaged: Vec<String>,
    pub staged: Vec<String>,
    pub focus: ListFocus,
    pub selected_idx: usize,
    pub commit_message: String,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub status: Option<StatusMsg>,
}
```

One enum for panel state — no parallel `PanelMode`.

### `src/git.rs`
Thin wrappers over `std::process::Command::new("git")`. All return `anyhow::Result`; non-zero exit surfaces `stderr`.

```rust
pub fn is_repo() -> bool;                              // `git rev-parse --git-dir`
pub fn status_porcelain() -> Result<(Vec<String>, Vec<String>)>;
                                                        // (unstaged, staged), parsed from `-z --porcelain=v1`
pub fn stage(path: &str) -> Result<()>;                // `git add -- <path>`
pub fn unstage(path: &str) -> Result<()>;              // `git reset -q HEAD -- <path>` (works pre-initial-commit too)
pub fn stage_all() -> Result<()>;                      // `git add -A`
pub fn unstage_all() -> Result<()>;                    // `git reset -q HEAD`
pub fn head_branch() -> Result<String>;                // `git rev-parse --abbrev-ref HEAD`
pub fn remote_url(name: &str) -> Result<String>;       // `git remote get-url <name>`
pub fn commit(msg: &str) -> Result<String>;            // `git commit -m <msg>` → stdout
pub fn push(remote: &str, branch: &str) -> Result<String>;
                                                        // `git push <remote> <branch>` → stdout+stderr
pub fn staged_diff() -> Result<String>;                // `git diff --cached`
```

Porcelain parsing rules (from `git status -z --porcelain=v1`):
- Two-byte XY code per record. X = index status, Y = worktree status.
- `staged` = records where X ∈ {M, A, D, R, C, U} and X ≠ ' '.
- `unstaged` = records where Y ∈ {M, D, A, ?, U} and Y ≠ ' '. (Includes untracked `??`.)
- Records can be both (staged AND unstaged) — list in both.
- Rename records `R  old -> new`: show `new`; `-z` splits old/new with NUL.

Execution helper:
```rust
fn run(args: &[&str]) -> Result<Output>;
// spawns `git` in CWD, inherits env, non-zero exit → Err with stderr.
```

### `src/ollama.rs`
```rust
pub fn generate_commit_message(diff: &str) -> anyhow::Result<String>;
```
- Build JSON body `{ "model": MODEL, "prompt": PREFIX + diff, "stream": false }`.
- `reqwest::blocking::Client::new().post(ENDPOINT).json(&body).send()?`.
- Deserialize into `struct Response { response: String }`.
- Trim, strip surrounding quotes, truncate to `COMMIT_MSG_MAX_CHARS`.
- Timeout: 60s (set via `Client::builder().timeout(..)`).

### `src/ui.rs`
Layout primitives:
```rust
pub fn split_main(area: Rect) -> (Rect, Rect, Rect); // header, body, status
pub fn bordered<'a>(title: &'a str) -> Block<'a>;    // block with LightBlue border
pub fn centered(area: Rect, w: u16, h: u16) -> Rect; // for help overlay
```

### `src/panel/*`
Each panel exposes:
```rust
pub fn render(state: &AppState, area: Rect, frame: &mut Frame);
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()>;
```

`commit.rs` additionally owns the Ollama call (blocking; the UI freezes while generating — acceptable for now, status bar shows "generating…" before the call via a redraw).

### `src/app.rs`
```rust
pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl App {
    pub fn new() -> Result<Self>;        // verify `git::is_repo()`, enter raw mode, alt screen
    pub fn run(&mut self) -> Result<()>;
    fn refresh(&mut self) -> Result<()>; // re-read status/branch/remote
    fn render(&mut self) -> Result<()>;
    fn handle_key(&mut self, k: KeyEvent) -> Result<()>;
}

impl Drop for App { /* leave alt screen, disable raw mode */ }
```

Event loop (ticking):
```rust
loop {
    if self.should_quit { break; }
    self.render()?;
    if event::poll(Duration::from_millis(TICK_MS))? {
        if let Event::Key(k) = event::read()? { self.handle_key(k)?; }
    }
    // tick: expire status messages older than STATUS_MSG_LIFETIME_SECS
    if let Some(s) = &self.state.status {
        if (Utc::now() - s.at).num_seconds() >= STATUS_MSG_LIFETIME_SECS {
            self.state.status = None;
        }
    }
}
```

`Drop` + `AtomicBool` / `std::panic::set_hook` to restore the terminal on panic.

---

## 5. Step-by-Step Build Order

### Phase 0 — Scaffolding
- `cargo init` in the existing dir (keeps CLAUDE.md, PLAN.md).
- Write `Cargo.toml` from §1.
- Create the tree from §4 with empty modules.
- `cargo check` clean.

### Phase 1 — `git.rs`
- `run()` helper; every wrapper.
- Porcelain parser as a pure function over `&[u8]`.
- Tests (see §6).
- `cargo test` green.

### Phase 2 — `state.rs`, `config.rs`
- Types only; no logic.
- `cargo check` clean.

### Phase 3 — UI skeleton
- `ui.rs` primitives; `panel/help.rs` (static overlay); `panel/mod.rs`.
- `app.rs`: terminal setup, blank render, tick loop, quit on `q`.
- `main.rs`.
- Binary launches, shows blank TUI, quits cleanly; terminal restored on panic.

### Phase 4 — Files panel
- Render staged/unstaged as side-by-side lists.
- `j`/`k` navigation within focus; `Tab` toggles focus.
- `y`/`u`/`A`/`U` mutate via `git.rs`, then `refresh`.
- `c` → `Panel::Commit`, `p` → `Panel::Push`, `?` → `Panel::Help`.

### Phase 5 — Commit panel
- Text input buffer in `AppState::commit_message`; Backspace/char insert.
- Sidebar = staged list from state.
- `o`: set status "generating…", redraw once, call `ollama::generate_commit_message(git::staged_diff()?)`, fill input on success or post an error status on failure.
- `Enter` (non-empty message): `git::commit(&msg)`, clear message, refresh, return to Files.
- `Esc`: return to Files (keep message).

### Phase 6 — Push panel
- On entry: populate `state.branch` + `state.remote_url` via `git.rs`.
- Render details.
- `Enter`: `git::push(DEFAULT_PUSH_REMOTE, &branch)`; show stdout/stderr as status.
- `Esc`: return to Files.

### Phase 7 — Help overlay
- Keybinding table (centered box).
- Any key returns to `state.previous_panel`.

### Phase 8 — Polish
- Colors from `config.rs`.
- Status bar: left = `lg`, center = branch, right = status message (red when `is_error`).
- `Event::Resize` → redraw.
- `Ctrl-C` → quit.
- Final `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`, `cargo test`.

---

## 6. Tests

All via `cargo test`; no external services needed.

### `git.rs` (integration, using `tempfile::TempDir` + real `git`)
- `parses porcelain with modified + untracked + renamed + staged-and-unstaged`
  — feed canned `-z` bytes to the parser; assert partition.
- `status_porcelain on fresh repo returns empty vectors`
  — `git init`, assert `(vec![], vec![])`.
- `stage then unstage round-trips`
  — init repo, write file, stage, assert in `staged`; unstage, assert in `unstaged`.
- `head_branch returns current branch`
  — init with `-b main`, assert `"main"`.
- `commit on empty message fails` — assert `Err`.

### `ollama.rs`
- `parses Ollama response` — fixture JSON → `"fix: …"`.
- `truncates to COMMIT_MSG_MAX_CHARS` — synth long string.
- `strips surrounding quotes`.

### State-level
- Smoke test on `AppState` transitions (pure): `Files` → `Commit` → `Files` after commit.

---

## 7. Hardcoded values

| Value | Constant | File |
|-------|----------|------|
| `http://localhost:11434/api/generate` | `OLLAMA_ENDPOINT` | `config.rs` |
| `qwen3.6:27b-coding-nvfp4` | `OLLAMA_MODEL` | `config.rs` |
| `"Write a concise one-line git commit message (<=72 chars) …"` | `OLLAMA_PROMPT_PREFIX` | `config.rs` |
| `origin` | `DEFAULT_PUSH_REMOTE` | `config.rs` |
| `72` | `COMMIT_MSG_MAX_CHARS` | `config.rs` |
| `1` | `STATUS_BAR_HEIGHT` | `config.rs` |
| `3` | `STATUS_MSG_LIFETIME_SECS` | `config.rs` |
| `Color::LightBlue` | `BORDER_COLOR` | `config.rs` |
| `250` | `TICK_MS` | `config.rs` |

---

## 8. Design notes

1. **Shell-out to `git`.** Inherits user's `~/.gitconfig`, credential helpers, SSH agent, GPG signing. No libgit2 build hassle; no credential callback wiring. Cost: one fork/exec per action — negligible at human-interaction rates.
2. **Single `AppState`.** Panels mutate it directly; no pub-sub.
3. **Sync throughout.** `reqwest::blocking` + `event::poll` tick loop; no tokio.
4. **Ollama call blocks the UI.** Acceptable; a spinner and pre-call "generating…" status keep it legible. If it starts to bite, move to a background thread + channel later — don't pre-build it.
5. **Error type.** `anyhow::Result<T>` everywhere; library errors propagate cleanly via `?`.
6. **Terminal restore.** `Drop` on `App` + panic hook — never leave the user's terminal in raw mode.
7. **No external config.** All knobs live in `config.rs`.

### Expected line count

| File | Lines |
|------|------:|
| `main.rs` | 5 |
| `config.rs` | 20 |
| `state.rs` | 40 |
| `git.rs` | 140 |
| `ollama.rs` | 50 |
| `ui.rs` | 60 |
| `app.rs` | 140 |
| `panel/files.rs` | 110 |
| `panel/commit.rs` | 90 |
| `panel/push.rs` | 50 |
| `panel/help.rs` | 35 |
| `panel/mod.rs` | 5 |
| **Total** | **~745** |

### Risks

| Risk | Mitigation |
|------|-----------|
| `git` not on `$PATH` | `App::new` fails fast with a clear error |
| Ollama not running | Status shows error; user types message manually |
| SSH auth prompts block the UI | Inherited from `git` CLI — agent must be pre-loaded; document in README |
| `git push` streams progress to stderr | Capture both; display last line in status |
| Renames in `status -z` split across records | Parser tests cover this (§6) |
| ratatui 0.30 MSRV 1.86 | Pinned `rust-version` field; CI can verify |

---

*Revised 2026-04-19 — dependencies refreshed, git via CLI, async inconsistency removed, libgit2 SSH pitfall eliminated, tests specified.*
