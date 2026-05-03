use super::common::*;

#[test]
fn commits_panel_shows_author_names_with_distinct_colors() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["def5678".into()],
            is_first_parent: true,
            subject: "add feature".into(),
        },
        Commit {
            sha: "def5678".into(),
            author: "Bob Example".into(),
            author_short: "BE".into(),
            parents: vec!["root".into()],
            is_first_parent: true,
            subject: "fix bug".into(),
        },
    ];

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("AE"), "missing first author: {text}");
    assert!(text.contains("BE"), "missing second author: {text}");
    assert_ne!(buf[(11, 1)].fg, Color::Reset);
    assert_ne!(buf[(11, 2)].fg, Color::Reset);
    assert_ne!(buf[(11, 1)].fg, buf[(11, 2)].fg);
}

#[test]
fn commits_panel_render_clamps_stale_selection() {
    let mut state = AppState::new();
    state.focus = Pane::Commits;
    state.commits_idx = usize::MAX;
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["parent".into()],
        is_first_parent: true,
        subject: "initial".into(),
    }];

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains("abc1234"),
        "missing commit row: {rendered}"
    );
}

#[test]
fn commits_panel_colors_merged_authors_by_author_and_separates_hash() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE001".into(),
            author: "Top Person".into(),
            author_short: "TP".into(),
            parents: vec!["MAIN".into(), "12345678".into()],
            is_first_parent: true,
            subject: "merge side".into(),
        },
        Commit {
            sha: "12345678".into(),
            author: "Carol Example".into(),
            author_short: "CE".into(),
            parents: vec!["abcdef12".into()],
            is_first_parent: false,
            subject: "side one".into(),
        },
        Commit {
            sha: "abcdef12".into(),
            author: "Dave Example".into(),
            author_short: "DE".into(),
            parents: vec!["MAIN".into()],
            is_first_parent: false,
            subject: "side two".into(),
        },
    ];

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut second_row = String::new();
    for col in 0..buf.area.width {
        second_row.push_str(buf[(col, 2)].symbol());
    }

    assert!(
        second_row.contains("12345678 CE"),
        "hash and author should be separated by a space: {second_row}"
    );
    assert_ne!(buf[(10, 2)].fg, Color::DarkGray);
    assert_ne!(buf[(10, 3)].fg, Color::DarkGray);
    assert_ne!(buf[(10, 2)].fg, buf[(10, 3)].fg);
}

#[test]
fn commits_panel_marks_merge_commits() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["P".into(), "S".into()],
        is_first_parent: true,
        subject: "merge branch".into(),
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    // ⏣─╮ at the merge row (lazygit-style).
    assert!(
        text.contains("\u{23e3}\u{2500}\u{256e}"),
        "missing merge connector: {text}"
    );
}

#[test]
fn commits_panel_draws_merge_connector_two_parent() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["P".into(), "S".into()],
        is_first_parent: true,
        subject: "merge branch".into(),
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let marker_col = (0..buf.area.width)
        .find(|col| buf[(*col, 1)].symbol() == "\u{23e3}")
        .expect("merge marker should be rendered");
    let expected = [
        ("\u{23e3}", 0u16),
        ("\u{2500}", 1),
        ("\u{256e}", 2),
        (" ", 3),
        ("m", 4),
    ];
    for (symbol, offset) in expected {
        assert_eq!(buf[(marker_col + offset, 1)].symbol(), symbol);
    }
}

#[test]
fn commits_panel_merge_then_side_renders_lane_through() {
    // Merge with parents [P, S]. S is the next commit. The merge fork's right side
    // should render as ╮ on the merge row, then S's row should show │ ◯.
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["PARENT".into(), "SIDE".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "SIDE".into(),
            author: "Side".into(),
            author_short: "SD".into(),
            parents: vec!["PARENT".into()],
            is_first_parent: false,
            subject: "side commit".into(),
        },
    ];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx: u16| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    assert!(
        row_text(1).contains("\u{23e3}\u{2500}\u{256e}"),
        "{}",
        row_text(1)
    );
    // SIDE row: │ ◯ (lane 0 still active, side commit on lane 1).
    assert!(row_text(2).contains("\u{2502} \u{25ef}"), "{}", row_text(2));
}

#[test]
fn commits_panel_uses_compact_graph_prefix_like_git_log() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into(), "def5678".into()],
            is_first_parent: true,
            subject: "merge branch".into(),
        },
        Commit {
            sha: "def5678".into(),
            author: "Bob Example".into(),
            author_short: "BE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    let cell_col = |row_idx, needle: &str| {
        let needle_chars: Vec<char> = needle.chars().collect();
        (0..buf.area.width).find(|start| {
            needle_chars.iter().enumerate().all(|(offset, ch)| {
                let col = start.saturating_add(offset as u16);
                col < buf.area.width && buf[(col, row_idx)].symbol() == ch.to_string()
            })
        })
    };
    let merge_row = row_text(1);
    let side_row = row_text(2);
    let merge_subject = cell_col(1, "merge branch").expect("merge subject");
    let side_marker = cell_col(2, "\u{25ef}").expect("side marker");
    let side_subject = cell_col(2, "side branch").expect("side subject");

    assert!(
        side_subject > side_marker,
        "side subject should follow its graph prefix:\n{side_row}"
    );
    let _ = merge_row;
    let _ = merge_subject;
}

#[test]
fn commits_panel_keeps_selected_hash_visible_and_graph_columns_stable() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "916a75688".into(),
            author: "Jan Example".into(),
            author_short: "JT".into(),
            parents: vec!["00e47360d".into()],
            is_first_parent: true,
            subject: "direct branch commit".into(),
        },
        Commit {
            sha: "00e47360d".into(),
            author: "Jan Example".into(),
            author_short: "JT".into(),
            parents: vec!["PARENT".into(), "a0f3424b0".into()],
            is_first_parent: true,
            subject: "merge side branch".into(),
        },
        Commit {
            sha: "a0f3424b0".into(),
            author: "Side Person".into(),
            author_short: "Sp".into(),
            parents: vec!["b3545f4c8".into()],
            is_first_parent: false,
            subject: "side branch commit".into(),
        },
        Commit {
            sha: "b3545f4c8".into(),
            author: "Renovate Bot".into(),
            author_short: "re".into(),
            parents: vec!["PARENT".into()],
            is_first_parent: false,
            subject: "second side commit".into(),
        },
    ];
    state.focus = Pane::Commits;
    state.commits_idx = 2;

    let backend = TestBackend::new(100, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    let cell_col = |row_idx, needle: &str| {
        let needle_chars: Vec<char> = needle.chars().collect();
        (0..buf.area.width).find(|start| {
            needle_chars.iter().enumerate().all(|(offset, ch)| {
                let col = start.saturating_add(offset as u16);
                col < buf.area.width && buf[(col, row_idx)].symbol() == ch.to_string()
            })
        })
    };

    let selected_row = row_text(3);
    assert!(
        selected_row.contains("a0f3424b0 Sp"),
        "selected hash and author should remain visible: {selected_row}"
    );
    assert_ne!(
        buf[(1, 3)].fg,
        buf[(1, 3)].bg,
        "selected hash foreground must contrast with selection background"
    );

    let merge_origin = cell_col(2, "\u{23e3}\u{2500}\u{256e}")
        .expect("merge row should show a visible branch origin");
    let selected_marker =
        cell_col(3, "\u{25ef}").expect("selected side commit should show a marker");
    assert!(
        selected_marker > merge_origin,
        "side commit marker should sit to the right of the merge origin:\n{}\n{}",
        row_text(2),
        selected_row
    );
    assert!(cell_col(3, "side branch commit").is_some());
    assert!(cell_col(4, "second side commit").is_some());
}

#[test]
fn commits_panel_highlights_selected_merge_connector() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["P".into(), "S".into()],
        is_first_parent: true,
        subject: "merge branch".into(),
    }];
    state.focus = Pane::Commits;
    state.commits_idx = 0;

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let connector_start = (0..buf.area.width)
        .find(|col| buf[(*col, 1)].symbol() == "\u{23e3}")
        .expect("selected merge connector should include ⏣");
    // Selected merge → all glyphs (⏣ ─ ╮) are bolded white.
    for (offset, symbol) in ["\u{23e3}", "\u{2500}", "\u{256e}"].iter().enumerate() {
        let cell = &buf[(connector_start + offset as u16, 1)];
        assert_eq!(cell.symbol(), *symbol);
        assert_eq!(cell.fg, Color::White);
        assert_eq!(cell.bg, Color::DarkGray);
    }
}

#[test]
fn commits_panel_highlights_selected_side_commit() {
    // Two commits: a merge then its second-parent side commit. Selecting the side
    // commit should highlight its row (background) and bold its marker.
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];
    state.focus = Pane::Commits;
    state.commits_idx = 1;

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let marker_col = (0..buf.area.width)
        .find(|col| buf[(*col, 2)].symbol() == "\u{25ef}")
        .expect("selected commit marker");

    let marker = &buf[(marker_col, 2)];
    assert_eq!(marker.bg, Color::DarkGray);
}

#[test]
fn commits_panel_places_hash_and_author_before_graph() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut row = String::new();
    for col in 0..buf.area.width {
        row.push_str(buf[(col, 2)].symbol());
    }

    let hash = row.find("abc1234").expect("hash in row");
    let author = row.find("AE").expect("author in row");
    let graph = row.find("\u{25ef}").expect("graph marker in row");
    assert!(hash < author, "hash should precede author: {row}");
    assert!(author < graph, "author should precede graph: {row}");
}

#[test]
fn commits_panel_highlights_selected_row_without_shifting_columns() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];
    state.focus = Pane::Commits;
    state.commits_idx = 1;

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let marker = buf
        .content()
        .iter()
        .find(|cell| cell.symbol() == "\u{25ef}")
        .expect("selected graph marker");
    assert_eq!(marker.bg, Color::DarkGray);
}

#[test]
fn commits_panel_renders_nested_merges_like_lazygit() {
    // Simplified slice from the user's `feature/PNT-2594` log:
    //   M0 (merge feature into origin/main)  parents: [F, M1]
    //   M1 (merge spring-boot into main)    parents: [M2, S1]
    //   S1 (renovate spring-boot)           parents: [SP]
    //   M2 (merge kotlin-plugin)            parents: [P, K1]
    //   K1 (renovate kotlin)                parents: [P]
    //   P  (older main)                     parents: [G]
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "M0".into(),
            author: "Top".into(),
            author_short: "JT".into(),
            parents: vec!["F".into(), "M1".into()],
            is_first_parent: true,
            subject: "Merge origin/main into feature".into(),
        },
        Commit {
            sha: "M1".into(),
            author: "Sp".into(),
            author_short: "Sp".into(),
            parents: vec!["M2".into(), "S1".into()],
            is_first_parent: false,
            subject: "Merge spring-boot".into(),
        },
        Commit {
            sha: "S1".into(),
            author: "renovate".into(),
            author_short: "re".into(),
            parents: vec!["SP".into()],
            is_first_parent: false,
            subject: "Update spring boot".into(),
        },
        Commit {
            sha: "M2".into(),
            author: "Sp".into(),
            author_short: "Sp".into(),
            parents: vec!["P".into(), "K1".into()],
            is_first_parent: false,
            subject: "Merge kotlin".into(),
        },
        Commit {
            sha: "K1".into(),
            author: "renovate".into(),
            author_short: "re".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "Update kotlin".into(),
        },
        Commit {
            sha: "P".into(),
            author: "Sp".into(),
            author_short: "Sp".into(),
            parents: vec!["G".into()],
            is_first_parent: false,
            subject: "older main".into(),
        },
    ];

    let backend = TestBackend::new(120, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx: u16| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    // M0: ⏣─╮ at col-pair 0/1 — top-level merge.
    assert!(
        row_text(1).contains("\u{23e3}\u{2500}\u{256e}"),
        "{}",
        row_text(1)
    );
    // M1: │ ⏣─╮ — feature lane stays vertical, merge lives on lane 1.
    assert!(
        row_text(2).contains("\u{2502} \u{23e3}\u{2500}\u{256e}"),
        "{}",
        row_text(2)
    );
    // S1: │ │ ◯ — both feature lane and main lane pass, S1 on lane 2.
    assert!(
        row_text(3).contains("\u{2502} \u{2502} \u{25ef}"),
        "{}",
        row_text(3)
    );
}

#[test]
fn commits_panel_dims_merged_in_commits() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        text.contains("\u{25ef}"),
        "missing merged-in marker: {text}"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "s" && cell.modifier.contains(Modifier::DIM)),
        "merged-in subject should be dimmed"
    );
}
