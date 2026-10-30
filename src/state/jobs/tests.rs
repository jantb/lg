use super::generation::{FINISH_MS, MAX_WORD_GAP_MS, START_WAIT_MS};
use super::*;

const FRAME_MS: u64 = crate::config::ANIMATION_FRAME_MS;

fn generation() -> Generation {
    let (_tx, rx) = std::sync::mpsc::channel();
    Generation::new(rx, std::thread::spawn(|| {}), Default::default())
}

/// Run the clock from `from` to `to` a frame at a time, as the loop does.
fn run(g: &mut Generation, from: u64, to: u64) {
    let mut now = from;
    while now <= to {
        g.release(now, true);
        now += FRAME_MS;
    }
}

fn finish(g: &mut Generation) {
    g.finished = Some((String::new(), Default::default()));
}

/// The words in flight at `now`, as they read in the output.
fn in_flight(g: &Generation, now: u64) -> Vec<String> {
    g.arrivals
        .iter()
        .filter(|a| now.saturating_sub(a.at_ms) < crate::panel::commit_art::FLIGHT_MS)
        .map(|a| g.output.chars().skip(a.start).take(a.len).collect())
        .collect()
}

/// A model that sends a line at once has it written out a word at a
/// time, the way one sending a word at a time would.
#[test]
fn a_line_from_the_model_comes_out_a_word_at_a_time() {
    let mut g = generation();
    g.receive("feat: accept a name argument");
    finish(&mut g);

    g.release(1_000, true);
    assert_eq!(g.output, "feat: ");

    run(&mut g, 1_000, 3_000);
    assert_eq!(g.output, "feat: accept a name argument");
}

/// Half a word in one chunk and the rest in the next fly as one word.
#[test]
fn a_word_split_across_chunks_flies_whole() {
    let mut g = generation();
    g.receive("word-fre");
    g.receive("quency tool");
    finish(&mut g);
    run(&mut g, 1_000, 1_000 + 2 * MAX_WORD_GAP_MS);

    let flown: Vec<String> = g
        .arrivals
        .iter()
        .map(|a| g.output.chars().skip(a.start).take(a.len).collect())
        .collect();
    assert!(
        flown.iter().any(|w| w.trim() == "word-frequency"),
        "{flown:?}"
    );
}

/// Until the model says more, the word it stopped on may not be whole.
#[test]
fn the_last_word_waits_for_more_until_the_model_finishes() {
    let mut g = generation();
    g.receive("add a READ");
    run(&mut g, 1_000, 3_000);
    assert_eq!(g.output, "add a ");

    g.receive("ME file");
    finish(&mut g);
    run(&mut g, 3_000, 5_000);
    assert_eq!(g.output, "add a README file");
}

/// The editor takes the message over only once nothing is left in the
/// air, so no word is cut off on its way into the text.
#[test]
fn a_finished_message_settles_once_its_last_word_has_landed() {
    let mut g = generation();
    g.receive("fix: handle empty input");
    finish(&mut g);
    assert!(!g.settled(1_000), "nothing has gone out yet");

    let mut now = 1_000;
    while !g.queued.is_empty() {
        g.release(now, true);
        now += FRAME_MS;
    }
    assert!(!in_flight(&g, now).is_empty());
    assert!(!g.settled(now), "the last word is still flying");

    now += crate::panel::commit_art::FLIGHT_MS;
    assert!(in_flight(&g, now).is_empty());
    assert!(g.settled(now));
}

/// A long message sent in one go does not keep the reader waiting long
/// after the model is done.
#[test]
fn a_long_backlog_is_written_out_quickly() {
    let mut g = generation();
    g.receive(&"word ".repeat(120));
    finish(&mut g);
    run(&mut g, 1_000, 1_000 + 2 * FINISH_MS);
    assert!(g.queued.is_empty());
    assert_eq!(g.output, "word ".repeat(120));
}

/// After a pause the words do not all go at once to make up the time.
#[test]
fn words_after_a_pause_still_go_one_at_a_time() {
    let mut g = generation();
    g.receive("one ");
    g.receive("two");
    run(&mut g, 1_000, 1_100);
    g.receive(" three four five six");
    finish(&mut g);
    g.release(10_000, true);
    assert_eq!(in_flight(&g, 10_000).len(), 1);
}

/// Claude can go quiet for over a second mid-message and then send the
/// rest at once. What is already queued is spread over the quiet, so
/// the text keeps moving instead of stopping dead until the burst.
#[test]
fn the_text_keeps_moving_through_a_pause_in_the_stream() {
    let mut g = generation();
    g.receive(&"word ".repeat(20));
    let quiet = 1_400;
    let mut last_word_at = 0;
    let mut longest_still = 0;
    let mut now = 1_000;
    while now <= 1_000 + quiet {
        let before = g.output.len();
        g.release(now, true);
        if g.output.len() > before {
            longest_still = longest_still.max(now - last_word_at.max(1_000));
            last_word_at = now;
        }
        now += FRAME_MS;
    }
    longest_still = longest_still.max(1_000 + quiet - last_word_at);
    assert!(
        longest_still <= 300,
        "the text stood still for {longest_still} ms"
    );

    g.receive(&"more ".repeat(60));
    finish(&mut g);
    run(&mut g, now, now + FINISH_MS + MAX_WORD_GAP_MS + FRAME_MS);
    assert!(g.queued.is_empty(), "the burst is caught up quickly");
}

/// Claude sends the opening word or two, goes quiet for about a second,
/// then sends the rest. The text does not start on the opening alone and
/// then stand still: once it starts it keeps moving to the end.
#[test]
fn an_opening_followed_by_a_pause_does_not_stall_the_text() {
    let mut g = generation();
    g.receive("feat(commit): fly");
    let mut now = 1_000;
    while now < 2_200 {
        g.release(now, true);
        now += FRAME_MS;
    }
    g.receive(&" streamed words in one at a time".repeat(4));
    finish(&mut g);

    let mut last_word_at = None;
    let mut longest_still = 0;
    let mut seen = 0;
    while !g.settled(now) {
        let before = g.output.len();
        g.release(now, true);
        if g.output.len() > before {
            if seen > 0
                && let Some(at) = last_word_at
            {
                longest_still = std::cmp::max(longest_still, now - at);
            }
            seen = g.output.len();
            last_word_at = Some(now);
        }
        now += FRAME_MS;
    }
    assert!(
        longest_still <= 300,
        "the text stood still for {longest_still} ms"
    );
}

/// A short message that never gets going still shows up.
#[test]
fn a_lone_opening_is_shown_after_a_while() {
    let mut g = generation();
    g.receive("fix: typo ");
    run(
        &mut g,
        1_000,
        1_000 + START_WAIT_MS + MAX_WORD_GAP_MS + FRAME_MS,
    );
    assert_eq!(g.output, "fix: typo ");
}

#[test]
fn with_animations_off_everything_shows_at_once() {
    let mut g = generation();
    g.receive("docs: explain the flag");
    finish(&mut g);
    g.release(1_000, false);
    assert_eq!(g.output, "docs: explain the flag");
    assert!(g.queued.is_empty());
}

#[test]
fn reasoning_taken_back_leaves_nothing_queued() {
    let mut g = generation();
    g.receive("thinking about it ");
    run(&mut g, 1_000, 1_100);
    g.restart();
    finish(&mut g);
    run(&mut g, 1_100, 2_000);
    assert!(g.output.is_empty());
}
