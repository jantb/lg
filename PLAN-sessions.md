# lg — worktrees and in-app claude sessions

Adds two capabilities to lg:

1. **Worktrees** as first-class objects: list, create, switch, remove.
2. **Claude Code sessions** running inside lg, one per worktree, sandboxed by
   terrarium, all live at once and switchable.

Together these let one repo be worked on from several branches simultaneously,
with lg as the switchboard.

## Status

All seven phases are implemented, on top of 521 passing tests with clippy clean.
Verified against the real tools: claude renders correctly inside a session pane,
and `terrarium run --project <worktree>` starts inside the profile lg derives
(`cargo test --test session_smoke -- --ignored --nocapture` shows both).

Four details ended up different from the plan below:

- **Switching while a session has the keyboard.** A captured session receives
  every key, Ctrl-C and Ctrl-n included, so switching is `Ctrl-]` first and then
  `Ctrl-n` / `Ctrl-p`. Stealing keys from the program inside would be the
  opposite of running it faithfully.
- **Sandbox choice is a key, not a setting.** `s` starts a sandboxed session,
  `S` starts one without the sandbox. No settings migration was needed, and the
  choice is per session rather than per checkout.
- **Zoom.** Workspace mode (`F2`) replaced the separate zoom key: it gives the
  session the whole right-hand side with the checkout list still one Tab away.
- **Ended sessions go.** The plan kept them listed with their final screen, to
  be dismissed by hand. They are dropped the moment their program ends instead:
  nothing to press `x` on, and the status line says which session went and how.
- **Two kinds, one of each per checkout.** `SessionKind::{Claude, Terminal}`
  landed as planned, on the keys `s`/`S` and `t`/`T`. The uniqueness key is the
  pair, not the directory: a checkout can run claude and a shell side by side,
  and the tree lists both under it. A terminal is the same PTY through the same
  terrarium sandbox, running `$SHELL`; only the hooks are claude's.

Known gaps, in rough order of how much they would be missed: no scrollback in
the session pane (the program's own scrolling works, lg cannot scroll back
through what left the screen); no mouse forwarding into a session; sessions do
not survive lg exiting (the `start_with` seam is where a tmux-backed session
would plug in); worktrees are listed for the active repository only, not for
every nested repository at once.

## Decisions

| Decision | Choice |
|---|---|
| Session host | In-pane PTY (`portable-pty` + `vt100` + hand-rolled cell renderer) |
| Worktree location | Sibling: `<repo-parent>/<repo>.worktrees/<slug>` (configurable) |
| Sandbox default | Sandboxed: `terrarium run --project <worktree> -- claude` |
| Session backend seam | Kept, so a tmux-wrapped command can be opted into later |

## Two modes

`AppMode::Git` is today's layout, unchanged, with a session badge in the header
(`3 sessions · 1 needs input`). The badge counts the states the tree's dots show
— blocked outranks busy outranks idle — rather than unread output, which for
anything mid-turn would be every session at once.

`AppMode::Workspace` is session-centric:

```
┌ Sessions ─────────┐┌ claude · lg@feat/parser · sandboxed ──────────┐
│ lg (main)         ││                                              │
│  ● feat/parser  ▲ ││   [live claude PTY rendered here]            │
│  ○ fix/edge       ││                                              │
│ benk/lg           ││                                              │
│  ● test         ● ││                                              │
├ feat/parser ──────┤│                                              │
│ ↑2 ↓0  ~3 files   ││                                              │
└───────────────────┘└──────────────────────────────────────────────┘
 input → claude · Ctrl-] release · z zoom · F2 git mode
```

Toggle with `F2` (and `Ctrl-]` then `F2` while input is captured). Target flow:
in Git mode, `W` on a branch row creates the worktree, writes its terrarium
profile, starts claude in it, and switches to Workspace mode.

## Part A — repo-context plumbing (prerequisite, no UX change)

Today ~120 git call sites funnel through `run` / `run_combined` (`src/git.rs:65`)
and use **process CWD**; repo switching is `std::env::set_current_dir`
(`src/app/actions.rs:387`). That breaks three ways once worktrees and concurrent
sessions exist: a job started before a switch finishes against the new dir; the
`notify` watcher is created once in `App::new` (`src/app.rs:126`) and never
re-registered, so after a switch it watches the old repo; and in Workspace mode
lg's own git ops must target the focused worktree.

Add to `src/git.rs`:

```rust
static ACTIVE_REPO: RwLock<Option<PathBuf>>;            // process default
thread_local! { static REPO_OVERRIDE: RefCell<Option<PathBuf>> }

pub fn set_active_repo(dir: impl Into<PathBuf>);
pub fn with_repo<T>(dir: &Path, f: impl FnOnce() -> T) -> T;
fn repo_dir() -> Option<PathBuf>;   // override, else active
```

`run`, `run_in_dir`, `run_combined` and the four raw `Command::new("git")` sites
(`src/git.rs:123,271,826,846`) prepend `-C <dir>` when set. Then:

- Drop `set_current_dir` from `SwitchRepository`; call `git::set_active_repo`
  and re-create the file watcher.
- Wrap job closures in `git::with_repo(captured_dir, …)` so each job is pinned
  to the repo it was started against (they already clone `workspace_root`,
  e.g. `src/app/spawn.rs:318`).
- Generalize to `SwitchRepository { target: RepoTarget }` with
  `RepoTarget::{Workspace, Nested(String), Path(PathBuf)}` — worktrees are
  absolute and may live outside the workspace, which the current relative join
  cannot express.
- Rename `OperationKind::Worktree` (`src/state/jobs.rs:99`) to `Checkout`; it
  means "touches the working tree" and the name is needed elsewhere now.

## Part B — worktrees

New `src/git/worktree.rs`:

```rust
pub struct Worktree { path: PathBuf, branch: Option<String>, head: String,
                      is_main: bool, locked: Option<String>, prunable: bool,
                      has_changes: bool }
pub fn parse_worktree_list(porcelain: &str) -> Vec<Worktree>   // pure, tested
pub fn worktrees() -> Result<Vec<Worktree>>                    // list --porcelain
pub fn worktree_add(path, branch, base: Option<&str>, create_branch: bool) -> Result<String>
pub fn worktree_remove(path, force) -> Result<String>
pub fn worktree_prune() -> Result<String>
```

Any worktree can answer `worktree list` for the whole set, so this is one cheap
call per repo per refresh — fold into `build_refresh_snapshot`
(`src/app/refresh.rs:10`) next to `nested_repositories`.

Placement is a sibling directory: no `.gitignore` churn, no confusing
`collect_nested_repo_dirs` (`src/git.rs:705`) which would otherwise list
worktrees as nested repos, and no build-tool or IDE recursion into them. The UI
lists worktrees from `git worktree list`, so location never matters to display.

UI: extend `NestedRepoTreeRow` (`src/panel/environments.rs:399`) with
`Worktree { repo_idx, wt_idx }`, indented under its repo; `Enter` switches. New
`Modal::Worktree` add-form (branch / base ref / path preview / "start claude
here"), modelled on `panel/flow.rs`. Removal routes through the existing
`Modal::ConfirmDestructive`. Add and remove run as `spawn_operation` jobs.

Dirty markers for non-focused worktrees come from a bounded poll of
`git -C <wt> status --porcelain` on the refresh tick, not from N watchers.

## Part C — claude sessions

New `src/term/` (`pty.rs`, `screen.rs`, `keys.rs`) and `src/session.rs`:

```rust
pub struct Session {
    id: SessionId, label: String, cwd: PathBuf,
    kind: SessionKind,          // Claude | Shell
    sandbox: bool,
    status: SessionStatus,      // Starting | Running | Exited(i32) | Failed(String)
    parser: vt100::Parser,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    rx: Receiver<PtyMsg>,       // Output(Vec<u8>) | Exited(i32)
    reader: JoinHandle<()>,
    attention: bool,            // bell, or output while unfocused
    size: (u16, u16),
}
```

- **Pump**: reader thread → mpsc → drained in the main loop beside the other
  `drain_*` calls (`src/app.rs:180`), `try_recv` until empty, fed to the parser.
  Unfocused sessions keep draining, so a background claude never blocks on a
  full pipe. This is what makes many live sessions work.
- **Render**: walk the `vt100::Screen` cells into a ratatui `Buffer`; cursor via
  `frame.set_cursor_position` when focused. Deliberately not `tui-term` — it
  tracks ratatui 0.29 and this crate is on 0.30.
- **Input capture**: when `focus == Pane::Main` and the view is a session and
  capture is on, keys are encoded to bytes and written to the PTY *before* the
  modal match in `handle_key` (`src/app/input.rs:477`). Release with `Ctrl-]`
  (`Esc` must pass through). While captured, enable bracketed paste (forward
  `Event::Paste` as `ESC[200~…ESC[201~`) and crossterm keyboard-enhancement
  flags so Shift+Enter reaches claude.
- **Sizing**: resize the PTY on layout change and on session switch; the resize
  also forces a repaint, which is how a backgrounded session comes back clean.
- **Lifecycle**: a session that exits is dropped as soon as that is noticed —
  its row goes, and a pane showing it returns to the diff — so `x` is only for
  stopping one that is still running. Children are killed in `App::drop` next to
  `join_background_jobs` (`src/app.rs:238`); quitting with live sessions goes
  through a confirm modal.
- **Env**: set `TERM=xterm-256color`, `COLORTERM=truecolor`; scrub `CLAUDECODE`
  and `CLAUDE_CODE_ENTRYPOINT` so a nested claude does not misdetect its
  context; keep `ANTHROPIC_*`.

## Part D — terrarium as the per-worktree jail

Terrarium's own schema (from the binary) is
`ProjectProfile { name, preset, rules, allow_commit, params, network, command }`
with `Rule { action, operation, path_type: literal|subpath|regex|ancestors,
path_value, comment, source }`, so lg writes
`~/.terrarium/projects/<slug>/profile.toml` itself — no interactive
`terrarium init`:

```toml
name = "lg@feat-parser"
preset = "rust"                 # copied from the parent repo's profile
allow_commit = false
command = "claude"

[[rules]]
action = "allow"
operation = "file-read* file-write*"
path_type = "subpath"
path_value = "/Users/jantb/dev/priv/lg/.git"
comment = "worktree needs the main repo's shared git dir"

[params]
PROJECT_ROOT = "/Users/jantb/dev/priv/lg.worktrees/feat-parser"

[network]
proxy_enabled = false
allowed_domains = []
```

`PROJECT_ROOT = <worktree>` is the cwd jail: session A cannot read or write
session B's files. The one manual rule is unavoidable — a worktree's `.git` is a
file pointing at `<main>/.git/worktrees/<name>`, and commits write objects and
refs into the shared dir. Isolation is therefore at the **working-tree** level,
not the ref level; full ref isolation would need separate clones.

`terrarium run --project <worktree> -- claude` regenerates `active.sb` from that
profile. Surface `terrarium profile validate` failures as the session's startup
error rather than a black pane. When the lg root is itself a terrarium
workspace, write a `WorkspaceConfig.projects` entry instead.

## Part E — mode switch

`state.mode: AppMode` and `state.main_view: MainView::{Diff, Review,
Session(SessionId)}` — a separate enum from `DiffSource`, which is about diff
content. `src/ui/layout.rs` gains `split_workspace_layout`; `handle_key` gains a
mode-dispatch layer above the modal match. Session switching: `Ctrl-n`/`Ctrl-p`
cycle, `Alt-1..9` jump, `Modal::Sessions` picker. `z` zooms the session pane.

Two duplications to respect: `HeadlessApp::send_key` (`src/app/input.rs:184`)
and `App::handle_key` (`src/app/input.rs:477`) are near-identical, so every new
binding lands in both (or they get unified first); and `panel/help.rs` is a
hand-maintained key table.

## Dependencies

`portable-pty` and `vt100`. Neither is in the local registry cache, so adding
them needs network — run `cargo add` / `cargo update` outside the terrarium
sandbox. Manifest goes from 8 to ~15 crates (`filedescriptor`, `libc`, `log`
come in with portable-pty). `wezterm-term` would buy sixel and hyperlinks for a
much heavier tree; not worth it for v1.

## Testing

- `parse_worktree_list` unit tests: detached, locked, prunable, bare, multi.
- `tests/git_integration.rs`: add/list/switch/remove a worktree in a tempfile
  repo; assert `git::with_repo` pins a call while `ACTIVE_REPO` differs.
- PTY tests without claude: spawn `sh -c 'printf "hi\r\n"'`, assert screen text
  and exit detection. Hermetic.
- Key-encoding table tests: Enter/Tab/Esc/arrows/Ctrl-*/Alt-*/paste → bytes.
- Render tests: make `Session` constructible with a pre-seeded parser and no
  child, then cover sessions and Workspace mode in the `tests/tiny_term.rs`
  small-size sweep and `visual_smoke.rs`.

## Phasing

| # | Scope | Rough size |
|---|---|---|
| 1 | Repo-context plumbing, watcher re-register, `RepoTarget` | ~150 lines |
| 2 | Worktree read model + tree rows + switch | ~350 lines |
| 3 | Worktree add/remove modal + jobs | ~300 lines |
| 4 | PTY runtime + render + capture, one session | ~600 lines |
| 5 | Multi-session, switcher, attention, zoom, quit guard | ~400 lines |
| 6 | Terrarium profile writer + sandbox toggle + settings | ~250 lines |
| 7 | Workspace mode layout + mode switch | ~350 lines |

Phases 1–3 stand alone (worktree management, no claude). Phase 4 is the risky
one; everything after it is additive.

## Caveats

- vt100 emulation is not pixel-perfect for exotic output (sixel, some OSC).
  Claude's TUI is well within scope; expect small polish rounds.
- Sessions die with lg unless the tmux-wrapped backend is opted into.
- Two claudes sharing one `.git` can contend on refs and objects; per-worktree
  index files mean no `index.lock` conflict, and git locks the rest.
