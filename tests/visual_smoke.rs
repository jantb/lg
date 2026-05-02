//! Visual smoke test: renders a real DAG from a core-service-style log fixture
//! and prints it for manual comparison against lazygit. Run with `--nocapture`.
//!
//! Doesn't assert anything; this exists for human-in-the-loop verification.

use lg::{git::Commit, panel, state::AppState};
use ratatui::{Terminal, backend::TestBackend};

fn c(sha: &str, parents: &[&str], short: &str, subject: &str, first: bool) -> Commit {
    Commit {
        sha: sha.into(),
        author: "x".into(),
        author_short: short.into(),
        parents: parents.iter().map(|s| (*s).into()).collect(),
        is_first_parent: first,
        subject: subject.into(),
    }
}

#[test]
#[ignore = "visual only"]
fn render_screenshot_dag_to_stdout() {
    // DAG matching the user's screenshot from core-service feature/PNT-2594.
    // Top of log is the current feature branch; below is origin/main being merged in,
    // which itself contains a chain of renovate merges.
    let commits = vec![
        c(
            "5858c5714",
            &["73536e2c"],
            "JT",
            "Fix test to use correct reverse transaction ID",
            true,
        ),
        c(
            "73536e2cd",
            &["00e47360"],
            "JT",
            "Enhance balance handling for multi-member households",
            true,
        ),
        c(
            "00e47360d",
            &["FEAT_PARENT", "a0f3424b0"],
            "JT",
            "Merge remote-tracking branch 'origin/main'",
            true,
        ),
        c(
            "a0f3424b0",
            &["8cce243fa", "b3545f4c8"],
            "Sp",
            "Merged in renovate/spring-boot",
            false,
        ),
        c(
            "b3545f4c8",
            &["79efedbdf"],
            "re",
            "Update spring boot to v4.0.6",
            false,
        ),
        c(
            "8cce243fa",
            &["b04d1bb9e", "6311c5d26"],
            "Sp",
            "Merged in renovate/org.jetbrains.kotlin.plugin",
            false,
        ),
        c(
            "6311c5d26",
            &["79efedbdf"],
            "re",
            "Update plugin org.jetbrains.kotlin.plugin",
            false,
        ),
        c(
            "b04d1bb9e",
            &["f8eaf1bbb", "9d23a645e"],
            "Sp",
            "Merged in renovate/ktor-monorepo",
            false,
        ),
        c(
            "9d23a645e",
            &["7f65d5d5e"],
            "re",
            "Update ktor monorepo to v3.4.3",
            false,
        ),
        c(
            "f8eaf1bbb",
            &["79efedbdf", "76f5bd1e2"],
            "HK",
            "Merged in feature/programId-to-bit",
            false,
        ),
        c(
            "76f5bd1e2",
            &["91933ec2", "7f65d5d5e"],
            "HK",
            "Merge branch 'main' into feature/programId-to-bit",
            false,
        ),
        c(
            "91933ec21",
            &["1f1a0de80"],
            "HK",
            "Default programId to null in EarnTransactionFto",
            false,
        ),
        c(
            "1f1a0de80",
            &["79efedbdf"],
            "HK",
            "Add support for programId in transactions",
            false,
        ),
        c(
            "79efedbdf",
            &["30711d4b9", "d5c228db8"],
            "Sp",
            "Merged in renovate/swaggerannotationsversion",
            false,
        ),
        c(
            "d5c228db8",
            &["7f65d5d5e"],
            "re",
            "Update dependency io.swagger.core.v3:swagger-annotations",
            false,
        ),
        c(
            "30711d4b9",
            &["4cbeb8edd", "679cab6de"],
            "Sp",
            "Merged in renovate/org.jetbrains.kotlin.plugin.serialization",
            false,
        ),
        c(
            "679cab6de",
            &["7f65d5d5e"],
            "re",
            "Update plugin org.jetbrains.kotlin.plugin.serialization",
            false,
        ),
        c(
            "4cbeb8edd",
            &["7f65d5d5e", "cdaf4bf84"],
            "Sp",
            "Merged in renovate/org.jetbrains.kotlin.jvm",
            false,
        ),
        c(
            "cdaf4bf84",
            &["7f65d5d5e"],
            "re",
            "Update plugin org.jetbrains.kotlin.jvm",
            false,
        ),
        c("7f65d5d5e", &["GRANDPARENT"], "Sp", "older main", false),
    ];

    let mut state = AppState::new();
    state.commits = commits;

    let backend = TestBackend::new(140, 25);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| panel::commits::render(&state, f.area(), f, false))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    println!("\n=== rendered (no selection) ===");
    for r in 0..buf.area.height {
        let mut row = String::new();
        for c in 0..buf.area.width {
            row.push_str(buf[(c, r)].symbol());
        }
        println!("{row}");
    }

    // Now with the merge commit selected.
    state.commits_idx = 2;
    state.focus = lg::state::Pane::Commits;
    let mut terminal = Terminal::new(TestBackend::new(140, 25)).unwrap();
    terminal
        .draw(|f| panel::commits::render(&state, f.area(), f, true))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    println!("\n=== rendered (idx=2 selected, focused) ===");
    for r in 0..buf.area.height {
        let mut row = String::new();
        for c in 0..buf.area.width {
            row.push_str(buf[(c, r)].symbol());
        }
        println!("{row}");
    }
}
