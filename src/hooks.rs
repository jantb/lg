//! What a session is doing, in claude's own words.
//!
//! claude runs a command of lg's choosing at each point of its turn it has a
//! hook for. lg points those at one file per session and reads the lines back,
//! which is exact. Reading the spinner off the screen instead was a guess at
//! strings that change release to release: it lost `esc to interrupt` when that
//! stopped being printed, and then read every turn longer than a minute as ready
//! because the clock had grown a `1m ` in front of the seconds.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::session::SessionActivity;

/// The hooks lg asks for, and what each one says the session has become.
///
/// `SubagentStop` is deliberately absent: a subagent finishing does not end the
/// turn, and reading it as idle would drop the dot mid-work. So is `SessionEnd` —
/// a session going away is noticed by its pty closing, which also covers the
/// ways claude can die without getting to run a hook.
const HOOK_EVENTS: &[(&str, SessionActivity)] = &[
    ("SessionStart", SessionActivity::Idle),
    ("UserPromptSubmit", SessionActivity::Working),
    ("PreToolUse", SessionActivity::Working),
    ("PostToolUse", SessionActivity::Working),
    ("Notification", SessionActivity::NeedsInput),
    ("Stop", SessionActivity::Idle),
];

/// Hooks that fire per tool, and so are configured under a matcher. The rest
/// match everything by having none.
const TOOL_HOOKS: &[&str] = &["PreToolUse", "PostToolUse"];

/// Everything one session needs to report what it is doing. Each session has
/// a channel of its own, which goes away with it.
pub struct HookChannel {
    /// Settings file to start claude with, holding hooks that report here.
    pub settings: PathBuf,
    /// Read side of the file those hooks append to.
    pub events: HookEvents,
    dir: PathBuf,
}

impl Drop for HookChannel {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Channels this lg has handed out, so that two sessions in one checkout never
/// share one.
static CHANNELS_OPENED: AtomicU64 = AtomicU64::new(0);

/// Give the session about to run in `cwd` a settings file whose hooks report to
/// a file only that session writes, and open the read side of it.
pub fn install(cwd: &Path) -> Result<HookChannel> {
    let name = format!(
        "{}-{}",
        std::process::id(),
        CHANNELS_OPENED.fetch_add(1, Ordering::Relaxed)
    );
    let checkout_dir = channel_dir(cwd)?;
    if let Some(sessions_dir) = checkout_dir.parent() {
        clear_channels_of_exited_lgs(sessions_dir);
    }
    let dir = checkout_dir.join(name);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    // Created empty: a channel left behind by an lg that died under the same
    // pid describes a turn that is over.
    let events_path = dir.join("events");
    File::create(&events_path).with_context(|| format!("create {}", events_path.display()))?;
    let file =
        File::open(&events_path).with_context(|| format!("read {}", events_path.display()))?;

    let channel = HookChannel {
        settings: dir.join("settings.json"),
        events: HookEvents {
            file,
            pending: String::new(),
        },
        dir,
    };
    std::fs::write(&channel.settings, settings_json(&events_path))
        .with_context(|| format!("write {}", channel.settings.display()))?;
    Ok(channel)
}

/// Remove the channels, in every checkout's directory under `sessions_dir`, of
/// lg processes that have exited without dropping them. A pid reused by some
/// other process keeps its channels, which only costs leaving them for later.
fn clear_channels_of_exited_lgs(sessions_dir: &Path) {
    let Ok(checkouts) = std::fs::read_dir(sessions_dir) else {
        return;
    };
    for channel in checkouts
        .flatten()
        .filter_map(|checkout| std::fs::read_dir(checkout.path()).ok())
        .flatten()
        .flatten()
    {
        let name = channel.file_name();
        let Some((pid, counter)) = name.to_str().and_then(|name| name.split_once('-')) else {
            continue;
        };
        let (Ok(pid), Ok(_)) = (pid.parse::<libc::pid_t>(), counter.parse::<u64>()) else {
            continue;
        };
        if pid > 0 && !process_exists(pid) && channel.path().is_dir() {
            let _ = std::fs::remove_dir_all(channel.path());
        }
    }
}

fn process_exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the pid can be signalled; nothing is sent.
    let status = unsafe { libc::kill(pid, 0) };
    status == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Where a session's hook file and the settings pointing at it live: under the
/// repository's git directory. That is out of the working tree, so it shows up in
/// no status listing, and it is the one place outside its own checkout that a
/// sandboxed session is already allowed to write.
/// Where a review session is told to write its findings: beside the hook
/// files, which is a place a sandboxed session is already allowed to write.
/// Whatever an earlier review left there is removed, so the next one is read
/// from nothing rather than from stale findings.
pub fn review_findings_path(cwd: &Path) -> Result<PathBuf> {
    fresh_session_file(cwd, "review-findings.md")
}

/// A file named `name` beside the hook files for a session to write an answer
/// into, cleared of whatever an earlier answer left there.
pub fn fresh_session_file(cwd: &Path, name: &str) -> Result<PathBuf> {
    let dir = channel_dir(cwd)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(name);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("clear {}", path.display())),
    }
    Ok(path)
}

fn channel_dir(cwd: &Path) -> Result<PathBuf> {
    let git_dir = crate::git::with_repo(cwd, crate::git::common_git_dir)
        .context("locate the git directory to keep session hooks in")?;
    Ok(git_dir
        .join("lg/sessions")
        .join(crate::terrarium::profile_slug(cwd)))
}

/// The settings claude is started with. Written out readable, because this is
/// the first place to look when a session stops reporting.
fn settings_json(events: &Path) -> String {
    let hooks: serde_json::Map<String, serde_json::Value> = HOOK_EVENTS
        .iter()
        .map(|(event, _)| ((*event).to_string(), hook_entry(event, events)))
        .collect();
    let settings = serde_json::json!({ "hooks": hooks });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings.to_string())
    )
}

fn hook_entry(event: &str, events: &Path) -> serde_json::Value {
    let mut entry = serde_json::Map::new();
    if TOOL_HOOKS.contains(&event) {
        entry.insert("matcher".to_string(), serde_json::json!("*"));
    }
    entry.insert(
        "hooks".to_string(),
        serde_json::json!([{ "type": "command", "command": report_command(event, events) }]),
    );
    serde_json::Value::Array(vec![serde_json::Value::Object(entry)])
}

/// The whole hook: append the event's own name to the file.
///
/// The path is written into the command rather than read out of the environment,
/// because a sandboxed session is started by terrarium and need not be handed
/// lg's variables.
fn report_command(event: &str, events: &Path) -> String {
    format!(
        "printf '{event}\\n' >> {}",
        shell_quote(&events.to_string_lossy())
    )
}

/// `text` as one single-quoted shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The read side of one session's hook file.
pub struct HookEvents {
    file: File,
    /// A line the hook had not finished appending when lg last read.
    pending: String,
}

impl HookEvents {
    /// Every event appended since the last call, oldest first.
    ///
    /// Hooks append while lg reads, so a read can land mid-line. Only whole
    /// lines are events; the rest waits for the write that finishes it.
    pub fn drain(&mut self) -> Vec<HookEvent> {
        let mut fresh = String::new();
        if self.file.read_to_string(&mut fresh).is_err() || fresh.is_empty() {
            return Vec::new();
        }
        self.pending.push_str(&fresh);

        let mut events = Vec::new();
        while let Some(newline) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=newline).collect();
            if let Some(event) = HookEvent::parse(line.trim()) {
                events.push(event);
            }
        }
        events
    }
}

/// One thing claude said it was about to do, or had just finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookEvent {
    pub name: &'static str,
    pub activity: SessionActivity,
}

impl HookEvent {
    /// The event `line` names, or nothing when it names none — a hook lg does
    /// not know about is not worth acting on.
    fn parse(line: &str) -> Option<Self> {
        HOOK_EVENTS
            .iter()
            .find(|(event, _)| *event == line)
            .map(|(name, activity)| HookEvent {
                name,
                activity: *activity,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hook_lg_asks_for_reports_something_it_can_act_on() {
        let settings = settings_json(Path::new("/repo/.git/lg/sessions/x/events"));
        let parsed: serde_json::Value = serde_json::from_str(&settings).expect("valid json");

        for (event, _) in HOOK_EVENTS {
            let entry = &parsed["hooks"][event][0];
            let command = entry["hooks"][0]["command"].as_str().expect("a command");
            assert_eq!(
                command,
                &format!("printf '{event}\\n' >> '/repo/.git/lg/sessions/x/events'"),
                "{event} must report itself by name"
            );
            assert_eq!(entry["hooks"][0]["type"], "command");
            assert_eq!(
                entry.get("matcher").is_some(),
                TOOL_HOOKS.contains(event),
                "{event}: only per-tool hooks take a matcher"
            );
        }
    }

    /// A turn that has started is working, and only `Stop` ends it. Reading
    /// `SubagentStop` as the end of a turn would clear the dot while the session
    /// carried on, which is the mistake worth guarding.
    #[test]
    fn a_turn_is_working_until_it_stops() {
        assert_eq!(
            HookEvent::parse("UserPromptSubmit").map(|event| event.activity),
            Some(SessionActivity::Working)
        );
        assert_eq!(
            HookEvent::parse("Stop").map(|event| event.activity),
            Some(SessionActivity::Idle)
        );
        assert_eq!(
            HookEvent::parse("Notification").map(|event| event.activity),
            Some(SessionActivity::NeedsInput)
        );
        assert_eq!(HookEvent::parse("SubagentStop"), None);
        assert_eq!(HookEvent::parse(""), None);
    }

    /// Every session gets a channel of its own, and a closed session takes its
    /// channel with it rather than leaving it in the git directory for good.
    #[test]
    fn a_closed_session_leaves_no_hook_channel_behind() {
        let repo = tempfile::tempdir().expect("tempdir");
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .expect("run git init");
        assert!(status.success(), "git init");

        let channel = install(repo.path()).expect("install");
        assert!(channel.settings.exists(), "claude is handed its settings");

        let settings = channel.settings.clone();
        drop(channel);
        assert!(!settings.exists(), "{}", settings.display());
    }

    /// An lg that crashed never dropped its channels. The next lg clears them
    /// away, but never one whose lg is still running its session.
    #[test]
    fn channels_of_an_lg_that_is_gone_are_cleared_and_a_running_ones_kept() {
        let repo = tempfile::tempdir().expect("tempdir");
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .expect("run git init");
        assert!(status.success(), "git init");
        let other_checkout = channel_dir(repo.path())
            .expect("channel dir")
            .with_file_name("elsewhere");

        let mut gone = std::process::Command::new("true").spawn().expect("spawn");
        let gone_pid = gone.id();
        gone.wait().expect("wait");
        let mut running = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn");

        let crashed = other_checkout.join(format!("{gone_pid}-0"));
        let live = other_checkout.join(format!("{}-0", running.id()));
        for dir in [&crashed, &live] {
            std::fs::create_dir_all(dir).expect("create channel");
            std::fs::write(dir.join("events"), "Stop\n").expect("write events");
        }

        let channel = install(repo.path()).expect("install");
        let _ = running.kill();
        let _ = running.wait();

        assert!(!crashed.exists(), "{}", crashed.display());
        assert!(live.join("events").exists(), "{}", live.display());
        assert!(channel.settings.exists());
    }

    #[test]
    fn a_path_with_a_quote_in_it_stays_one_shell_word() {
        assert_eq!(shell_quote("/repo/it's/events"), "'/repo/it'\\''s/events'");
    }

    /// Hooks append while lg reads, so the file can be read mid-line. The half
    /// line has to wait for the write that finishes it rather than being dropped.
    #[test]
    fn a_line_split_across_two_reads_is_read_once_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("events");
        std::fs::write(&path, "Stop\nUserPrompt").expect("write");
        let mut events = HookEvents {
            file: File::open(&path).expect("open"),
            pending: String::new(),
        };

        let first = events.drain();
        assert_eq!(first.len(), 1, "only the whole line is an event");
        assert_eq!(first[0].name, "Stop");
        assert!(events.drain().is_empty(), "nothing new to read");

        std::fs::write(&path, "Stop\nUserPromptSubmit\n").expect("append");
        let second = events.drain();
        assert_eq!(
            second.iter().map(|event| event.name).collect::<Vec<_>>(),
            vec!["UserPromptSubmit"],
            "the line finishes with what arrived after it"
        );
    }

    #[test]
    fn an_unreported_event_leaves_the_reading_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("events");
        std::fs::write(&path, "PreCompact\nSubagentStop\nStop\n").expect("write");
        let mut events = HookEvents {
            file: File::open(&path).expect("open"),
            pending: String::new(),
        };

        assert_eq!(
            events.drain().iter().map(|e| e.name).collect::<Vec<_>>(),
            vec!["Stop"],
            "hooks lg did not ask for are skipped, not guessed at"
        );
    }
}
