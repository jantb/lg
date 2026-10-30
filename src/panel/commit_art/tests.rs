use super::*;

/// A show with no diff behind it: the stream falls back to abstract
/// tokens, which is what most of these tests are looking at.
fn show(lang: Language, seed: usize, ms: u64) -> Show<'static> {
    static EMPTY: std::sync::OnceLock<Feed> = std::sync::OnceLock::new();
    Show {
        lang,
        seed,
        ms,
        feed: EMPTY.get_or_init(Feed::default),
        boom: None,
    }
}

fn text_of(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const ALL: [Language; 7] = [
    Language::Rust,
    Language::Kotlin,
    Language::Java,
    Language::Go,
    Language::Python,
    Language::TypeScript,
    Language::Other,
];

#[test]
fn the_language_with_most_files_wins() {
    let paths = ["README.md", "a/Main.kt", "b/Other.kt", "c/lib.rs"];
    assert_eq!(Language::dominant(paths), Language::Kotlin);
    assert_eq!(Language::dominant(["notes.txt"]), Language::Other);
    // JavaScript has no figure of its own; its commits get the generic
    // one rather than a stand-in.
    assert_eq!(Language::dominant(["app.js", "b.jsx"]), Language::Other);
    assert_eq!(Language::dominant([]), Language::Other);
}

/// A diff with the awkward things a real one has in it: a tab, a line
/// far longer than any terminal, characters that take two columns and
/// ones that take none. The scene draws these, so the box must still
/// come out square.
const AWKWARD_DIFF: &str = "diff --git a/x.rs b/x.rs\n@@ -1,4 +1,4 @@\n\
     \x20fn wake() {\n\
     -\tlet sleepy = 1; // \u{5e73}\u{4eee}\u{540d} \u{1f980} e\u{301}\n\
     +\tlet sparkly = 2;\n\
     +    // and a line that runs on and on and on and on and on and on and \
     on and on and on and on and on and on and on and on and on past any \
     terminal anyone has ever sat in front of\n";

#[test]
fn every_scene_fills_the_box_it_was_given_and_holds_still() {
    // With nothing behind the scene, and with a real diff behind it: the
    // diff backdrop lays out lines that came out of a repository, not
    // glyphs of its own choosing.
    for feed in [Feed::default(), Feed::from_diff(AWKWARD_DIFF)] {
        for lang in ALL {
            for seed in 0..BACKDROPS.len() {
                for (w, h) in [(90u16, 16u16), (120, 40), (200, 60)] {
                    for frame in 0..40 {
                        let ms = frame * 230;
                        let show = Show {
                            lang,
                            seed,
                            ms,
                            feed: &feed,
                            boom: None,
                        };
                        let lines = scene(show, w, h, true);
                        assert_eq!(
                            lines.len(),
                            h as usize,
                            "{lang:?} fills {w}x{h} top to bottom"
                        );
                        for line in &lines {
                            assert_eq!(
                                line.width(),
                                w as usize,
                                "{lang:?} seed {seed} at {ms}ms fills {w} across: {}",
                                text_of(std::slice::from_ref(line))
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_scene_moves_between_frames_and_says_what_the_model_is_doing() {
    let first = text_of(&scene(show(Language::Rust, 0, 0), 110, 30, true));
    // Eight milliseconds on, the figure has turned a hair and the light on
    // it has moved: a frame rate that high must not draw the same picture
    // twice.
    let next_frame = scene(show(Language::Rust, 0, 8), 110, 30, true);
    assert_ne!(
        scene(show(Language::Rust, 0, 0), 110, 30, true),
        next_frame,
        "8ms later is a new frame"
    );
    assert!(
        first.contains(TOKEN_GLYPHS[0]) || first.contains(TOKEN_GLYPHS[1]),
        "tokens flow into the network:\n{first}"
    );
    assert!(first.contains("\u{25cb}"), "the network is drawn:\n{first}");
    assert!(
        first.contains('O'),
        "Ferris is drawn, eyes and all:\n{first}"
    );
    assert!(first.contains("tokenizing the diff"), "{first}");

    let seen: std::collections::HashSet<String> = (0..(CAPTIONS.len() as u64 + 1))
        .map(|slot| {
            caption(Language::Rust, slot * CAPTION_MS)
                .trim_end_matches(['.', ' '])
                .to_owned()
        })
        .collect();
    assert!(seen.contains("arguing with the borrow checker"), "{seen:?}");
    assert!(
        seen.len() > CAPTIONS.len(),
        "every caption gets its turn: {seen:?}"
    );
}

#[test]
fn different_seeds_give_different_backdrops_behind_the_same_mascot() {
    let scenes: Vec<String> = (0..BACKDROPS.len())
        .map(|seed| text_of(&scene(show(Language::Go, seed, 1_440), 110, 30, true)))
        .collect();
    for (i, a) in scenes.iter().enumerate() {
        // The gopher may have its back turned at this instant, so look
        // for its shaded body rather than its eyes: no backdrop uses the
        // dense shading glyphs.
        assert!(
            a.contains(['#', '%', '@']),
            "the gopher is in scene {i}:\n{a}"
        );
        for b in &scenes[i + 1..] {
            assert_ne!(a, b, "seeds pick different backdrops");
        }
    }
}

#[test]
fn a_box_too_small_gets_nothing_rather_than_a_cropped_crab() {
    assert!(scene(show(Language::Rust, 0, 0), 70, 2, true).is_empty());
    assert!(scene(show(Language::Rust, 0, 0), 8, 12, true).is_empty());
    assert!(scene(show(Language::Rust, 0, 0), 0, 0, true).is_empty());
}

#[test]
fn a_flight_leaves_its_place_in_the_text_empty_until_it_lands() {
    let text = vec!["feat: add flights".to_owned()];
    let flight = |age_ms| Flight {
        row: 0,
        col: 10,
        text: "flights".to_owned(),
        age_ms,
    };
    let (w, h) = (110, 30);
    // Just left the model: the word is somewhere in the box, not yet
    // where it belongs.
    let early = text_of(&stage(
        show(Language::Rust, 0, 100),
        w,
        h,
        &text,
        &[flight(0)],
    ));
    assert_eq!(early.lines().count(), h as usize);
    assert!(early.starts_with("feat: add "), "{early}");
    assert!(!early.starts_with("feat: add flights"), "{early}");
    assert!(early.contains("flights"), "the word is in flight:\n{early}");
    // Landed and cooling: in place.
    let late = text_of(&stage(
        show(Language::Rust, 0, 500),
        w,
        h,
        &text,
        &[flight(FLIGHT_MS)],
    ));
    assert!(late.starts_with("feat: add flights"), "{late}");
    // Long gone: plain text, nothing else drawn for it.
    let done = text_of(&stage(
        show(Language::Rust, 0, 900),
        w,
        h,
        &text,
        &[flight(FLIGHT_TOTAL_MS)],
    ));
    assert!(done.starts_with("feat: add flights"), "{done}");
    assert_eq!(done.matches("flights").count(), 1, "{done}");
}

#[test]
fn a_flight_moves_between_frames_and_stands_out_from_the_text() {
    let text = vec!["fix: it".to_owned()];
    let flight = |age_ms| Flight {
        row: 0,
        col: 5,
        text: "it".to_owned(),
        age_ms,
    };
    // Each letter in flight has a colour of its own, so the word is
    // found cell by cell rather than span by span.
    let at = |ms: u64, age: u64| {
        let lines = stage(show(Language::Go, 1, ms), 110, 30, &text, &[flight(age)]);
        lines.iter().enumerate().find_map(|(row, line)| {
            let cells: Vec<(char, Style)> = line
                .spans
                .iter()
                .flat_map(|span| span.content.chars().map(move |c| (c, span.style)))
                .collect();
            cells
                .windows(2)
                .position(|w| w[0].0 == 'i' && w[1].0 == 't')
                .map(|col| (row, col, cells[col].1))
        })
    };
    let a = at(0, 0).expect("in the picture at take-off");
    let b = at(150, 150).expect("in the picture midway");
    let c = at(300, 300).expect("in the picture near landing");
    assert!(a != b && b != c, "the word moves: {a:?} {b:?} {c:?}");
    assert!(
        a.2.add_modifier.contains(Modifier::BOLD) && a.2.fg.is_some(),
        "in flight it is coloured and bold, unlike the text: {a:?}"
    );
    let landed = at(FLIGHT_TOTAL_MS, FLIGHT_TOTAL_MS).expect("in place at the end");
    assert_eq!((landed.0, landed.1), (0, 5));
    assert_eq!(
        landed.2,
        Style::default(),
        "settled text is plain: {landed:?}"
    );
}

#[test]
fn without_a_scene_the_text_still_fills_the_box() {
    let text = vec!["feat: tiny".to_owned()];
    let flight = Flight {
        row: 0,
        col: 6,
        text: "tiny".to_owned(),
        age_ms: 0,
    };
    let lines = stage(show(Language::Rust, 0, 0), 20, 3, &text, &[flight]);
    assert_eq!(lines.len(), 3);
    let shown = text_of(&lines);
    assert!(shown.starts_with("feat: tiny"), "{shown}");
}

#[test]
fn a_narrow_box_keeps_the_mascot_and_drops_the_pipeline() {
    let lines = scene(show(Language::Go, 1, 7_000), 40, 16, true);
    assert_eq!(lines.len(), 16);
    assert!(lines.iter().all(|line| line.width() == 40));
    let text = text_of(&lines);
    assert!(text.contains('O'), "the gopher is still there:\n{text}");
    assert!(
        !text.contains(TOKEN_GLYPHS[0]) && !text.contains(TOKEN_GLYPHS[1]),
        "no room for the token stream:\n{text}"
    );
}

#[test]
fn the_stream_carries_the_diff_the_model_is_reading() {
    let feed = Feed::from_diff(
        "diff --git a/x.rs b/x.rs\nindex 111..222 100644\n@@ -1 +1 @@\n\
         -let sleepy = 1;\n+let sparkly = 2;\n",
    );
    // Headers are not code and do not go down the stream.
    let text: String = feed.tape.iter().map(|&(c, _)| c).collect();
    assert!(!text.contains("100644"), "{text}");
    assert!(text.contains("sparkly"), "{text}");

    // Over a few seconds the whole of a short diff goes past.
    let seen: String = (0..80)
        .map(|frame| {
            let show = Show {
                lang: Language::Rust,
                seed: 0,
                ms: frame * 90,
                feed: &feed,
                boom: None,
            };
            text_of(&scene(show, 130, 30, false))
        })
        .collect();
    assert!(seen.contains("sleepy"), "the diff is legible in the stream");
    assert!(seen.contains("sparkly"), "both sides of it go in");

    // With no diff to show the stream still runs, on abstract tokens.
    let bare = text_of(&scene(show(Language::Rust, 0, 0), 130, 30, false));
    assert!(
        bare.contains(TOKEN_GLYPHS[0]) || bare.contains(TOKEN_GLYPHS[1]),
        "{bare}"
    );
}

#[test]
fn the_stream_carries_several_lines_of_the_diff_at_once() {
    // Every line of this diff says `let`, so a row of the stream that is
    // carrying code says it too, wherever in the diff the row is.
    let body: String = (0..30)
        .map(|i| format!("+    let value_{i} = {i} + 1;\n"))
        .collect();
    let feed = Feed::from_diff(&format!("diff --git a/x.rs b/x.rs\n@@ -1 +1,30 @@\n{body}"));
    let rows = (0..60)
        .map(|frame| {
            let show = Show {
                lang: Language::Rust,
                seed: 0,
                ms: frame * 80,
                feed: &feed,
                boom: None,
            };
            text_of(&scene(show, 130, 30, false))
                .lines()
                .filter(|line| line.contains("let"))
                .count()
        })
        .max()
        .unwrap_or(0);
    assert!(rows > 1, "the stream is a block of code, not one line");
}

#[test]
fn the_diff_backdrop_scrolls_the_change_itself_past_the_reader() {
    let seed = BACKDROPS
        .iter()
        .position(|b| *b == Backdrop::Diff)
        .expect("the diff is one of the backdrops");
    let feed = Feed::from_diff(
        "diff --git a/x.rs b/x.rs\nindex 111..222 100644\n@@ -1,2 +1,2 @@\n\
         \x20fn wake() {\n-    let sleepy = 1;\n+\tlet sparkly = 2;\n",
    );
    let seen: String = (0..40)
        .map(|frame| {
            let show = Show {
                lang: Language::Rust,
                seed,
                ms: frame * 120,
                feed: &feed,
                boom: None,
            };
            text_of(&scene(show, 130, 30, false))
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(seen.contains("let sparkly = 2;"), "{seen}");
    assert!(seen.contains("let sleepy = 1;"), "both sides show");
    assert!(seen.contains("fn wake() {"), "and the context around them");

    // And the code is highlighted where it lands, not painted one flat
    // colour: the keyword, the name and the marker are all told apart.
    let row = (0..40)
        .flat_map(|frame| {
            let show = Show {
                lang: Language::Rust,
                seed,
                ms: frame * 120,
                feed: &feed,
                boom: None,
            };
            scene(show, 130, 30, false)
        })
        .find(|line| text_of(std::slice::from_ref(line)).contains("let sparkly = 2;"))
        .expect("a row of the backdrop carries the added line");
    let colors: std::collections::HashSet<Option<Color>> = row
        .spans
        .iter()
        .filter(|span| !span.content.trim().is_empty())
        .map(|span| span.style.fg)
        .collect();
    assert!(colors.len() > 2, "the line is highlighted: {colors:?}");

    // Nothing to show falls back to a stand-in rather than a blank sky.
    let bare = scene(show(Language::Rust, seed, 0), 130, 30, false);
    let sky = text_of(&bare[..6]);
    assert!(!sky.trim().is_empty(), "the sky is not left blank:\n{sky}");
}

#[test]
fn the_trench_run_blows_the_station_when_the_first_word_flies_in() {
    let seed = BACKDROPS
        .iter()
        .position(|b| *b == Backdrop::Trench)
        .expect("the trench run is one of the backdrops");
    let mut show = show(Language::Rust, seed, 4_000);
    let waiting = text_of(&scene(show, 130, 34, true));
    assert!(waiting.contains(">=[O]=<"), "the run is on:\n{waiting}");
    show.ms = 8_000;
    show.boom = Some(6_000);
    let burning = text_of(&scene(show, 130, 34, false));
    let sky: String = burning.lines().take(12).collect::<Vec<_>>().join("\n");
    assert!(
        sky.matches(['#', '%', '@']).count() > 60,
        "the station is going up:\n{sky}"
    );
    assert_ne!(waiting, burning);
}

/// The sky of the scene for `backdrop`, after running the clock the way
/// a real wait would, since the arcade games build up as they go.
fn sky_after_a_wait(backdrop: Backdrop) -> String {
    let seed = BACKDROPS
        .iter()
        .position(|b| *b == backdrop)
        .expect("the game is one of the backdrops");
    let mut last = String::new();
    for frame in 0..600u64 {
        last = text_of(&scene(
            show(Language::Rust, seed, frame * 50),
            120,
            34,
            false,
        ));
    }
    last.lines().take(12).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_tron_backdrop_has_riders_and_their_walls_above_the_mascot() {
    let sky = sky_after_a_wait(Backdrop::Tron);
    assert!(
        sky.contains(['>', '<', '^', 'v']),
        "riders are out there:\n{sky}"
    );
    assert!(
        sky.contains('\u{2500}') || sky.contains('\u{2502}'),
        "and they have left walls:\n{sky}"
    );
}

/// The top rows of the scene for `backdrop` as cells of glyph, ink and
/// ground, after running the clock the way a real wait would. Short of
/// the ground line, so every row of it is sky.
fn sky_cells_after_a_wait(backdrop: Backdrop) -> Vec<(char, Option<Color>, Option<Color>)> {
    let seed = BACKDROPS
        .iter()
        .position(|b| *b == backdrop)
        .expect("the game is one of the backdrops");
    let mut last = Vec::new();
    for frame in 0..600u64 {
        let lines = scene(show(Language::Rust, seed, frame * 50), 120, 34, false);
        last = lines
            .iter()
            .take(8)
            .flat_map(|line| {
                line.spans.iter().flat_map(|span| {
                    span.content
                        .chars()
                        .map(move |c| (c, span.style.fg, span.style.bg))
                })
            })
            .collect();
    }
    last
}

/// Both first-person backdrops fill their sky with a lit, solid picture
/// rather than glyphs on black: every cell of it is painted top and
/// bottom, and it is painted in many shades, which is what a room seen
/// in perspective looks like and a flat wall of one colour does not.
#[test]
fn the_first_person_backdrops_fill_the_sky_with_a_lit_room() {
    for backdrop in [Backdrop::Wolf3D, Backdrop::Doom] {
        let sky = sky_cells_after_a_wait(backdrop);
        assert!(!sky.is_empty(), "{backdrop:?} draws something");
        assert!(
            sky.iter().all(|(_, fg, bg)| fg.is_some() && bg.is_some()),
            "{backdrop:?} paints every cell of its sky"
        );
        let mut shades: Vec<Color> = sky.iter().filter_map(|c| c.1).collect();
        shades.sort_by_key(|c| format!("{c:?}"));
        shades.dedup();
        assert!(
            shades.len() > 30,
            "{backdrop:?} is a room with depth in it, not a flat colour: {} shades",
            shades.len()
        );
    }
}

/// The two rooms are lit and coloured for their own game: the maze is
/// bright at the far end of a hall, the base has gone dark by then.
#[test]
fn the_doom_base_is_darker_than_the_wolfenstein_maze() {
    let brightness = |backdrop| {
        let sky = sky_cells_after_a_wait(backdrop);
        let total: f32 = sky
            .iter()
            .filter_map(|c| c.1)
            .map(|c| match c {
                Color::Rgb(r, g, b) => (r as f32 + g as f32 + b as f32) / 3.0,
                _ => 0.0,
            })
            .sum();
        total / sky.len() as f32
    };
    assert!(
        brightness(Backdrop::Doom) < brightness(Backdrop::Wolf3D),
        "the base is the darker of the two"
    );
}

#[test]
fn the_snake_backdrop_has_a_snake_and_food_above_the_mascot() {
    let sky = sky_after_a_wait(Backdrop::Snake);
    assert!(sky.contains('@'), "the snake is out:\n{sky}");
    assert!(
        sky.contains(['$', '*', '+']),
        "and there is food about:\n{sky}"
    );
    assert!(
        !sky.contains(['>', '<', '^', 'v']),
        "no light cycles in snake:\n{sky}"
    );
}
