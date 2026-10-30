//! What the guided review tests share: walks set up in a known state, and
//! keys, clicks and the screen to drive and read them.

use ratatui::crossterm::event::{MouseEvent, MouseEventKind};

use crate::state::{AppState, Modal};

mod file_findings;
mod final_review;
mod issues;
mod list;
mod parts;
mod questions;
mod reset;
mod review_chat;
mod source;
mod walk_modes;

pub(super) use super::poll::*;
pub(super) use super::*;

pub(super) fn commentary(text: &str) -> Commentary {
    Commentary {
        text: text.to_string(),
        done: true,
        ..Default::default()
    }
}

pub(super) fn press(state: &mut AppState, code: ratatui::crossterm::event::KeyCode) {
    let key = ratatui::crossterm::event::KeyEvent::new(
        code,
        ratatui::crossterm::event::KeyModifiers::NONE,
    );
    handle_key(state, key).unwrap();
}

pub(super) const AREA: ratatui::layout::Rect = ratatui::layout::Rect::new(0, 0, 120, 40);

/// A walk over a branch, on its one hunk, long enough to need scrolling.
pub(super) fn walk_on_a_long_hunk() -> AppState {
    let mut diff =
        String::from("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,40 +1,40 @@\n");
    for n in 0..40 {
        diff.push_str(&format!("-old {n}\n+new {n}\n"));
    }
    let mut guided = Guided::new(Source::Branch {
        branch: "guided-test-branch".into(),
        base_ref: "main".into(),
    });
    guided.steps = crate::git::guided::parse_files(&diff);
    guided.current = 1;
    let mut state = AppState::new();
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state
}

pub(super) fn panes(state: &AppState) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
    draw::pane_areas(
        state.guided.as_deref().unwrap(),
        AREA,
        state.guided_list_width,
    )
    .unwrap()
}

pub(super) fn mouse(state: &mut AppState, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent {
        kind,
        column,
        row,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    };
    handle_mouse(state, AREA, &event);
}

/// a.rs of two hunks and b.rs of one. Both start reviewed, so walking
/// past them saves nothing to disk.
pub(super) fn walk_with_findings() -> AppState {
    let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
                @@ -1,3 +1,3 @@\n keep\n-old one\n+new one\n tail\n\
                @@ -20,3 +20,3 @@\n keep\n-old two\n+new two\n keep\n\
                diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
    let mut guided = Guided::new(Source::Branch {
        branch: "guided-test-findings".into(),
        base_ref: "main".into(),
    });
    guided.steps = crate::git::guided::parse_files(diff);
    guided.progress.reviewed = guided.steps.iter().map(GuidedFile::key).collect();
    guided.commentary.insert(
        guided.steps[0].key(),
        commentary(
            "Summary: Renames a line.\n\
             L2 issue: -1 stands in for missing and is used as an index.\n\
             L3 nit: tail could say what it ends.\n\
             Verdict: issue",
        ),
    );
    guided.commentary.insert(
        guided.steps[1].key(),
        commentary("Summary: Renames x.\nL1 nit: y says less than x.\nVerdict: nit"),
    );
    guided.current = 1;
    guided.cursor = guided.steps[0].first_changed();
    let mut state = AppState::new();
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state
}

pub(super) fn cursor_line(state: &AppState) -> (usize, String) {
    let guided = state.guided.as_deref().unwrap();
    let line = &guided.step().unwrap().lines[guided.cursor];
    (guided.current, line.text.clone())
}

pub(super) fn screen(state: &AppState) -> Vec<String> {
    let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(state, AREA, frame)).unwrap();
    let buf = terminal.backend().buffer();
    (0..AREA.height)
        .map(|y| (0..AREA.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// A review chat on a.rs running `script` in place of claude, once it has
/// printed `ready`.
pub(super) fn chatting(state: &mut AppState, script: &str, ready: &str) {
    let spawn = crate::term::Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        env_remove: Vec::new(),
    };
    let spec = crate::session::SessionSpec {
        label: "review chat".into(),
        cwd: std::env::temp_dir(),
        sandboxed: false,
        kind: crate::session::SessionKind::Claude,
        prompt: None,
    };
    let id = state.sessions.start_with(spec, &spawn, (24, 80)).unwrap();
    let started = std::time::Instant::now();
    while !state
        .sessions
        .get(id)
        .is_some_and(|session| session.screen().contents().contains(ready))
    {
        assert!(
            started.elapsed().as_secs() < 10,
            "the chat never printed {ready}"
        );
        state.sessions.pump();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let guided = state.guided.as_mut().unwrap();
    guided.chat = Some(id);
    guided.chat_file = Some("a.rs".into());
}

/// Four files and the plan the model gave for them. Progress starts empty
/// whatever an earlier run saved, since the tests save as they walk.
pub(super) fn walk_with_plan() -> AppState {
    let file = |path: &str, line: &str| {
        format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-old\n+{line}\n"
        )
    };
    let mut guided = Guided::new(Source::Branch {
        branch: "guided-test-modes".into(),
        base_ref: "main".into(),
    });
    guided.progress = GuidedProgress::default();
    guided.steps = crate::git::guided::parse_files(&format!(
        "{}{}{}{}",
        file("src/lookup.rs", "lookup calls the cache"),
        file("Cargo.lock", "bumped"),
        file("tests/cache.rs", "evicts the oldest"),
        file("src/cache.rs", "fn evict()"),
    ));
    guided.commentary.insert(
        OVERVIEW_KEY.to_string(),
        commentary(
            "Summary: Adds a cache in front of lookup.\n\
             Concern: The cache\n\
             Why: Keeps recent lookups.\n\
             Files: src/lookup.rs, src/cache.rs\n\
             Concern: Tests\n\
             Files: tests/cache.rs\n\
             Order: tests/cache.rs, src/cache.rs, src/lookup.rs\n\
             Risk: src/lookup.rs high: a new call path\n\
             Risk: src/cache.rs medium: eviction under the lock\n\
             Risk: tests/cache.rs low: adds cases\n\
             Skim: Cargo.lock: dependency bump",
        ),
    );
    let mut state = AppState::new();
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state
}

/// The step list's rows as drawn, its own columns only.
pub(super) fn list_text(state: &AppState) -> Vec<String> {
    let (list, _) = panes(state);
    screen(state)
        .into_iter()
        .skip(list.y as usize)
        .take(list.height as usize)
        .map(|row| {
            row.chars()
                .skip(list.x as usize)
                .take(list.width as usize)
                .collect()
        })
        .collect()
}

pub(super) fn row_of(rows: &[String], text: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(text))
        .unwrap_or_else(|| panic!("{text} is not listed: {rows:#?}"))
}
