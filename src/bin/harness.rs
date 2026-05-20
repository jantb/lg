/// Evaluation harness for `lg`.
///
/// Creates throwaway git repos in tempdirs, exercises every `lg::git` function,
/// drives a headless TUI session, and optionally calls the local LLM if reachable.
/// Prints a final summary and exits with code 0 iff failed == 0.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use ratatui::{
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use tempfile::TempDir;

// ── Counters ──────────────────────────────────────────────────────────────────

static PASSED: AtomicUsize = AtomicUsize::new(0);
static FAILED: AtomicUsize = AtomicUsize::new(0);
static SKIPPED: AtomicUsize = AtomicUsize::new(0);

macro_rules! check {
    ($label:expr, $expr:expr) => {{
        match (|| -> anyhow::Result<()> { $expr })() {
            Ok(()) => {
                println!("OK   {}", $label);
                PASSED.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                println!("FAIL {} — {e}", $label);
                FAILED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }};
}

macro_rules! skip {
    ($label:expr, $reason:expr) => {{
        println!("SKIP {} — {}", $label, $reason);
        SKIPPED.fetch_add(1, Ordering::Relaxed);
    }};
}

// ── Buffer dump helpers ───────────────────────────────────────────────────────

fn buf_to_string(app: &lg::app::HeadlessApp<ratatui::backend::TestBackend>) -> String {
    let buf = app.terminal.backend().buffer().clone();
    let mut s = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            s.push_str(buf[(col, row)].symbol());
        }
        s.push('\n');
    }
    s
}

fn dump_buffer(label: &str, app: &lg::app::HeadlessApp<ratatui::backend::TestBackend>) {
    println!("---- {label} ----");
    print!("{}", buf_to_string(app));
    println!();
}

// ── Git helpers ───────────────────────────────────────────────────────────────

fn git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Harness User")
        .env("GIT_AUTHOR_EMAIL", "harness@example.com")
        .env("GIT_COMMITTER_NAME", "Harness User")
        .env("GIT_COMMITTER_EMAIL", "harness@example.com")
        .output()
        .expect("failed to spawn git")
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Initialise a repo with an initial commit so HEAD resolves.
fn init_repo_with_commit() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git_ok(dir.path(), &["init", "-b", "main"]);
    git_ok(dir.path(), &["config", "user.name", "Harness User"]);
    git_ok(dir.path(), &["config", "user.email", "harness@example.com"]);
    fs::write(dir.path().join("README.md"), "# harness repo").unwrap();
    git_ok(dir.path(), &["add", "README.md"]);
    let out = Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(dir.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Harness User")
        .env("GIT_AUTHOR_EMAIL", "harness@example.com")
        .env("GIT_COMMITTER_NAME", "Harness User")
        .env("GIT_COMMITTER_EMAIL", "harness@example.com")
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "initial commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

/// Seed the repo with a variety of statuses:
///   - modified tracked file (unstaged)
///   - new untracked file
///   - staged-but-not-committed file
///   - MM file (staged then re-modified)
fn seed_mixed_status(dir: &Path) {
    // Modified tracked (unstaged only)
    fs::write(dir.join("README.md"), "# modified").unwrap();

    // New untracked
    fs::write(dir.join("untracked.txt"), "untracked").unwrap();

    // Staged new file
    fs::write(dir.join("staged.txt"), "staged v1").unwrap();
    git_ok(dir, &["add", "staged.txt"]);

    // MM: stage a file, then modify it again
    fs::write(dir.join("both.txt"), "both v1").unwrap();
    git_ok(dir, &["add", "both.txt"]);
    fs::write(dir.join("both.txt"), "both v2").unwrap();
}

// ── LLM probe ─────────────────────────────────────────────────────────────────

fn llama_server_reachable() -> bool {
    let endpoint = lg::llm::current_endpoint();
    let base = endpoint
        .trim_end_matches("/v1/chat/completions")
        .trim_end_matches('/');
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .ok()
        .and_then(|c| c.get(format!("{base}/v1/models")).send().ok())
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    println!("=== lg evaluation harness ===\n");

    // Ensure the correct git binary is on PATH.
    // /usr/bin/git is an Xcode shim that requires xcodebuild; the real binary lives inside
    // the developer toolchain. Prepend it so every Command::new("git") and lg::git::run()
    // in this process find the right executable.
    let git_paths = [
        "/Applications/Xcode.app/Contents/Developer/usr/bin",
        "/Library/Developer/CommandLineTools/usr/bin",
    ];
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = git_paths
        .iter()
        .filter(|p| std::path::Path::new(p).exists())
        .map(|p| p.to_string())
        .chain(std::iter::once(current_path.clone()))
        .collect::<Vec<_>>()
        .join(":");
    // SAFETY: single-threaded at this point — no other threads have started yet.
    unsafe { std::env::set_var("PATH", &new_path) };

    // Set up primary repo and change CWD to it (process-global, fine for harness).
    let repo = init_repo_with_commit();
    seed_mixed_status(repo.path());
    std::env::set_current_dir(repo.path()).expect("set_current_dir");

    // ── git::is_repo ──────────────────────────────────────────────────────────
    check!("git::is_repo() returns true", {
        anyhow::ensure!(lg::git::is_repo(), "expected true");
        Ok(())
    });

    // ── git::status_porcelain ─────────────────────────────────────────────────
    check!("git::status_porcelain returns non-empty lists", {
        let (unstaged, staged) = lg::git::status_porcelain()?;
        anyhow::ensure!(
            !unstaged.is_empty(),
            "expected unstaged files, got {unstaged:?}"
        );
        anyhow::ensure!(!staged.is_empty(), "expected staged files, got {staged:?}");
        Ok(())
    });

    check!("both.txt appears in both staged and unstaged (MM)", {
        let (unstaged, staged) = lg::git::status_porcelain()?;
        anyhow::ensure!(
            unstaged.iter().any(|p| p == "both.txt"),
            "both.txt not in unstaged: {unstaged:?}"
        );
        anyhow::ensure!(
            staged.iter().any(|p| p == "both.txt"),
            "both.txt not in staged: {staged:?}"
        );
        Ok(())
    });

    // ── git::stage / unstage ──────────────────────────────────────────────────
    check!("git::stage adds untracked file to staged list", {
        lg::git::stage("untracked.txt")?;
        let (_, staged) = lg::git::status_porcelain()?;
        anyhow::ensure!(
            staged.iter().any(|p| p == "untracked.txt"),
            "untracked.txt not staged after git::stage: {staged:?}"
        );
        Ok(())
    });

    check!("git::unstage removes file from staged list", {
        lg::git::unstage("untracked.txt")?;
        let (_, staged) = lg::git::status_porcelain()?;
        anyhow::ensure!(
            !staged.iter().any(|p| p == "untracked.txt"),
            "untracked.txt still staged after git::unstage: {staged:?}"
        );
        Ok(())
    });

    // ── git::stage_all / unstage_all ──────────────────────────────────────────
    check!("git::stage_all stages everything", {
        lg::git::stage_all()?;
        let (_, staged) = lg::git::status_porcelain()?;
        anyhow::ensure!(!staged.is_empty(), "nothing staged after stage_all");
        Ok(())
    });

    check!("git::unstage_all clears staged list", {
        lg::git::unstage_all()?;
        let (_, staged) = lg::git::status_porcelain()?;
        anyhow::ensure!(
            staged.is_empty(),
            "staged not empty after unstage_all: {staged:?}"
        );
        Ok(())
    });

    // ── git::head_branch ─────────────────────────────────────────────────────
    check!("git::head_branch returns 'main'", {
        let branch = lg::git::head_branch()?;
        anyhow::ensure!(branch == "main", "expected 'main', got '{branch}'");
        Ok(())
    });

    // ── git::staged_diff ─────────────────────────────────────────────────────
    check!(
        "git::staged_diff returns empty string when nothing staged",
        {
            let diff = lg::git::staged_diff()?;
            anyhow::ensure!(
                diff.trim().is_empty(),
                "expected empty diff, got: {diff:.80}"
            );
            Ok(())
        }
    );

    check!("git::staged_diff returns non-empty string after staging", {
        lg::git::stage("staged.txt")?;
        let diff = lg::git::staged_diff()?;
        anyhow::ensure!(
            !diff.trim().is_empty(),
            "expected non-empty diff after staging"
        );
        Ok(())
    });

    // ── git::commit ───────────────────────────────────────────────────────────
    check!("git::commit with empty message returns Err", {
        anyhow::ensure!(
            lg::git::commit("").is_err(),
            "expected Err for empty message"
        );
        Ok(())
    });

    check!("git::commit with valid message succeeds", {
        // Stage README.md (which was modified)
        lg::git::stage("README.md")?;
        lg::git::stage("staged.txt")?;
        lg::git::stage("both.txt")?;
        let out = lg::git::commit("harness: test commit")?;
        anyhow::ensure!(!out.is_empty(), "expected non-empty commit output");
        Ok(())
    });

    // ── git::push with local bare remote ─────────────────────────────────────
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    check!("git::remote_url fails before remote is added", {
        anyhow::ensure!(
            lg::git::remote_url("origin").is_err(),
            "expected Err before remote is configured"
        );
        Ok(())
    });

    git_ok(
        repo.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );

    check!("git::remote_url returns bare repo path after add", {
        let url = lg::git::remote_url("origin")?;
        anyhow::ensure!(
            url == bare.path().to_str().unwrap(),
            "unexpected remote url: {url}"
        );
        Ok(())
    });

    check!("git::push succeeds to local bare remote", {
        lg::git::push("origin", "main")?;
        // Verify: git --git-dir=<bare> log should show our commit.
        let log_out = Command::new("git")
            .args([
                "--git-dir",
                bare.path().to_str().unwrap(),
                "log",
                "--oneline",
            ])
            .output()
            .expect("git log");
        anyhow::ensure!(
            log_out.status.success(),
            "git log on bare failed: {}",
            String::from_utf8_lossy(&log_out.stderr)
        );
        let log = String::from_utf8_lossy(&log_out.stdout);
        anyhow::ensure!(
            log.contains("harness: test commit"),
            "commit not found in bare log: {log}"
        );
        Ok(())
    });

    // ── LLM (skipped if not reachable) ────────────────────────────────────────
    if llama_server_reachable() {
        check!("llm stream_commit_message yields a final message", {
            let diff = "diff --git a/foo.rs b/foo.rs\n--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() {}";
            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn({
                let diff = diff.to_owned();
                move || lg::llm::stream_commit_message(diff, tx)
            });
            let mut final_msg: Option<String> = None;
            let mut err: Option<String> = None;
            while let Ok(msg) = rx.recv() {
                match msg {
                    lg::state::GenMsg::Done(m) => {
                        final_msg = Some(m);
                        break;
                    }
                    lg::state::GenMsg::Error(e) => {
                        err = Some(e);
                        break;
                    }
                    _ => {}
                }
            }
            handle.join().ok();
            if let Some(e) = err {
                anyhow::bail!("stream error: {e}");
            }
            let msg = final_msg.unwrap_or_default();
            anyhow::ensure!(!msg.is_empty(), "expected non-empty commit message");
            println!("       LLM message: {msg}");
            Ok(())
        });
    } else {
        skip!(
            "llm stream_commit_message",
            "llama-server not reachable at configured endpoint"
        );
    }

    // ── Seed extra repo content so Branches and Commits panels have data ────────
    // Add a second branch and another commit so all 4 panels have real data.
    git_ok(repo.path(), &["branch", "feat/lg-lazygit"]);
    git_ok(
        repo.path(),
        &["commit", "--allow-empty", "-m", "feat: wire branches"],
    );
    // Re-seed some dirty files so the Files panel shows colored status glyphs.
    fs::write(repo.path().join("untracked.txt"), "untracked again").unwrap();
    fs::write(repo.path().join("README.md"), "# re-modified").unwrap();
    git_ok(repo.path(), &["add", "README.md"]);

    // ── Headless TUI session ──────────────────────────────────────────────────
    check!("headless TUI: new layout renders with live repo data", {
        let backend = TestBackend::new(100, 30);
        let mut app = lg::app::HeadlessApp::new(backend)?;

        // Populate state from the real tempdir repo (CWD was set above).
        app.state.files = lg::git::status_entries()?;
        app.state.branches = lg::git::list_branches()?;
        app.state.commits = lg::git::list_commits(lg::config::COMMIT_LIST_LIMIT)?;
        app.state.branch = lg::git::head_branch().ok();
        app.state.remote_url = lg::git::remote_url(lg::config::DEFAULT_PUSH_REMOTE).ok();
        app.state.ahead_behind = lg::git::counts_ahead_behind().ok();

        fn key(code: KeyCode) -> KeyEvent {
            KeyEvent::new(code, KeyModifiers::NONE)
        }

        // Render initial frame (Files focus).
        app.render()?;
        dump_buffer("initial (Files focus)", &app);

        // 1 → Status focus
        app.send_key(key(KeyCode::Char('1')))?;
        dump_buffer("after '1' — Status focus", &app);

        // 3 → Branches focus
        app.send_key(key(KeyCode::Char('3')))?;
        dump_buffer("after '3' — Branches focus", &app);

        // 4 → Commits focus, then j to move selection
        app.send_key(key(KeyCode::Char('4')))?;
        app.send_key(key(KeyCode::Char('j')))?;
        dump_buffer("after '4' then j — Commits focus second row", &app);

        // 0 → Diff pane focus
        app.send_key(key(KeyCode::Char('0')))?;
        dump_buffer("after '0' — Diff focus", &app);

        // ? → Help overlay from Diff focus
        app.send_key(key(KeyCode::Char('?')))?;
        dump_buffer("after '?' — help overlay", &app);

        // any key closes help
        app.send_key(key(KeyCode::Char('x')))?;

        // Back to Files and open Help — verify it reflects Files as active.
        app.send_key(key(KeyCode::Char('2')))?;
        app.send_key(key(KeyCode::Char('?')))?;
        let help_buf = buf_to_string(&app);
        dump_buffer("after '2' then '?' — help overlay from Files", &app);

        // ── Assertions on rendered buffers ─────────────────────────────────────
        // The help overlay must mark Files as active with '▶'.
        anyhow::ensure!(
            help_buf.contains("▶ Files"),
            "help overlay did not highlight Files section"
        );

        // Close help, then grab the layout frame and assert every panel title appears.
        app.send_key(key(KeyCode::Char('x')))?;
        app.send_key(key(KeyCode::Char('2')))?;
        let layout_buf = buf_to_string(&app);
        for title in [
            "[1] Status",
            "[2] Files",
            "[3] Branches",
            "[4] Commits",
            "[0] Diff",
        ] {
            anyhow::ensure!(layout_buf.contains(title), "missing panel title: {title}");
        }

        // Footer content on Files focus.
        anyhow::ensure!(layout_buf.contains("space"), "files footer missing 'space'");
        anyhow::ensure!(
            layout_buf.contains("commit"),
            "files footer missing 'commit'"
        );

        // Switch to Branches and verify its footer mentions 'checkout'.
        app.send_key(key(KeyCode::Char('3')))?;
        let branches_buf = buf_to_string(&app);
        anyhow::ensure!(
            branches_buf.contains("checkout"),
            "branches footer missing 'checkout'"
        );

        // Verify that at least one cell in the Files panel row has a colored status glyph.
        // Switch back to Files, then walk cells looking for a known color.
        app.send_key(key(KeyCode::Char('2')))?;
        app.render()?;
        let buf = app.terminal.backend().buffer().clone();
        let found_colored_code = (0..buf.area.height).any(|row| {
            (0..buf.area.width).any(|col| {
                let cell = &buf[(col, row)];
                matches!(
                    cell.style().fg,
                    Some(
                        ratatui::style::Color::Yellow
                            | ratatui::style::Color::Green
                            | ratatui::style::Color::Red
                            | ratatui::style::Color::Cyan
                            | ratatui::style::Color::Magenta
                    )
                ) && matches!(cell.symbol(), "M" | "A" | "D" | "?" | "R" | "C" | "U")
            })
        });
        anyhow::ensure!(
            found_colored_code,
            "expected a colored status glyph cell in the rendered Files panel"
        );

        Ok(())
    });

    // ── Summary ───────────────────────────────────────────────────────────────
    let p = PASSED.load(Ordering::Relaxed);
    let f = FAILED.load(Ordering::Relaxed);
    let s = SKIPPED.load(Ordering::Relaxed);
    println!("\n=== {p} passed, {f} failed, {s} skipped ===");

    // Keep TempDirs alive until here so they're cleaned up on drop.
    drop(repo);
    drop(bare);

    std::process::exit(if f == 0 { 0 } else { 1 });
}
