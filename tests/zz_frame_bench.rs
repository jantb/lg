//! TEMPORARY frame-time benchmark. Run with:
//! cargo test --release --test zz_frame_bench -- --ignored --nocapture

use lg::{app::HeadlessApp, state::Modal};
use ratatui::backend::TestBackend;
use std::time::{Duration, Instant};

const BACKDROPS: [&str; 8] = [
    "rain", "night", "diff", "tron", "snake", "trench", "wolf3d", "doom",
];

fn app(w: u16, h: u16) -> HeadlessApp<TestBackend> {
    lg::git::set_active_repo(env!("CARGO_MANIFEST_DIR"));
    let mut app = HeadlessApp::new(TestBackend::new(w, h)).unwrap();
    let s = &mut app.state;
    s.files = lg::git::status_entries().unwrap();
    s.branches = lg::git::list_branches().unwrap();
    s.remote_branches = lg::git::list_remote_branches().unwrap_or_default();
    s.commits = lg::git::list_commits(200).unwrap();
    s.worktrees = lg::git::worktrees().unwrap_or_default();
    s.decorative_animations = true;
    let sha = s.commits[1].sha.clone();
    s.set_diff_text(lg::git::show_commit(&sha).unwrap());
    app
}

fn measure(label: &str, app: &mut HeadlessApp<TestBackend>, frames: usize) {
    for _ in 0..5 {
        app.state.skip_animation(Duration::from_millis(8));
        app.render().unwrap();
    }
    let mut times = Vec::with_capacity(frames);
    let mut changed = 0usize;
    let mut prev = app.terminal.backend().buffer().clone();
    for _ in 0..frames {
        app.state.skip_animation(Duration::from_millis(8));
        let t = Instant::now();
        app.render().unwrap();
        times.push(t.elapsed());
        let cur = app.terminal.backend().buffer().clone();
        changed += prev.diff(&cur).len();
        prev = cur;
    }
    times.sort();
    let mean = times.iter().sum::<Duration>() / frames as u32;
    let p95 = times[frames * 95 / 100];
    let max = *times.last().unwrap();
    println!(
        "{label:<28} mean {:>7.2}ms  p95 {:>7.2}ms  max {:>7.2}ms  changed cells/frame {:>6}",
        mean.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3,
        changed / frames
    );
}

#[test]
#[ignore = "benchmark"]
fn frame_times() {
    for (w, h) in [(220u16, 60u16), (320, 90)] {
        println!("== {w}x{h} ==");
        let mut a = app(w, h);
        for modal in [
            Modal::None,
            Modal::Help,
            Modal::Flow,
            Modal::Worktree,
            Modal::Settings,
            Modal::Commands,
            Modal::Push,
        ] {
            a.state.modal = modal;
            measure(&format!("{modal:?}"), &mut a, 200);
        }
        a.state.modal = Modal::Commit;
        measure("Commit (idle)", &mut a, 200);
        let (_tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(|| {});
        let diff = a.state.diff_text.clone();
        a.state
            .start_generation(rx, handle, lg::panel::commit_art::Feed::from_diff(&diff));
        for (i, name) in BACKDROPS.iter().enumerate() {
            a.state
                .commit_drafts
                .last_mut()
                .unwrap()
                .generation
                .as_mut()
                .unwrap()
                .scene = i;
            measure(&format!("Commit gen: {name}"), &mut a, 200);
        }
    }
}
