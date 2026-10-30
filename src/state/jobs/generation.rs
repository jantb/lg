//! A model's answer as it arrives: the messages a generation sends, the commit
//! draft it fills, and the words played out at a readable pace.

use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

#[derive(Debug)]
pub enum GenMsg {
    Thinking(String),
    Output(String),
    /// What has been sent as output turned out to be reasoning after all — a
    /// closing think tag arrived with no opening one. Consumers drop whatever
    /// they have accumulated and start over.
    Reset,
    /// The finished answer, with what the server said it cost.
    ///
    /// The stats travel with the answer rather than ahead of it because a
    /// consumer polls this channel in batches: sent separately, the stats and
    /// the answer they describe can land on different passes, and the pass that
    /// takes delivery of the answer is the one that has to know whether it was
    /// cut off.
    Done {
        text: String,
        stats: crate::llm::GenStats,
    },
    Error(String),
}

/// A commit message being written, or written, for one checkout. There is a
/// draft per checkout rather than one for the whole app: a message can be
/// left generating in the background and another started in the next
/// checkout, and the two must not be mistaken for each other. While the
/// modal is closed the draft is the sub-line under its checkout in the
/// workspace tree that leads back to it.
#[derive(Debug)]
pub struct CommitDraft {
    /// The checkout the message describes.
    pub dir: String,
    /// The model writing it, gone once it has finished or been cancelled.
    pub generation: Option<Generation>,
    /// The finished message, held until the checkout's modal is opened on
    /// it. Empty while the model is still writing: what has arrived so far
    /// lives in the generation, where the scene can see it stream in.
    pub text: String,
    /// The model has finished and nobody has looked at the result yet.
    pub ready: bool,
}

impl CommitDraft {
    pub fn generating(&self) -> bool {
        self.generation.is_some()
    }
}

#[derive(Debug)]
pub struct Generation {
    pub rx: Receiver<GenMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub output: String,
    pub spinner: usize,
    /// Which of the waiting scenes this generation shows, chosen when it
    /// started so the picture does not change under the reader.
    pub scene: usize,
    /// The words that have lately left the network, still flying in the
    /// scene to their place in the text.
    pub arrivals: Vec<Arrival>,
    /// When the first word left the network on the animation clock; the
    /// scene makes an event of the wait paying off.
    pub first_output_ms: Option<u64>,
    /// The diff that went to the model, prepared for the scene: it is what
    /// flows down the stream into the network.
    pub feed: crate::panel::commit_art::Feed,
    /// What the model has sent that has not left the network yet, a word to
    /// an entry. A model may send a word at a time or a line at a time;
    /// either way the words fly out one after another.
    pub queued: VecDeque<String>,
    /// When the next queued word may leave, on the animation clock.
    pub next_word_ms: u64,
    /// The gap between words once the model has finished, fixed then so
    /// what is left goes out at one steady pace.
    pub finish_gap_ms: Option<u64>,
    /// When the first of the message came from the model, on the animation
    /// clock: the words are held back for a while from then.
    pub first_chunk_ms: Option<u64>,
    /// The finished message, held back until the last word has landed so
    /// the editor does not take the text out from under the words in flight.
    pub finished: Option<(String, crate::llm::GenStats)>,
}

/// How long the words waiting are spread over while the model is still
/// writing. A model can go quiet for a second or more mid-message and then
/// send the rest at once; spreading what is queued slows the words down as
/// the queue runs low, so the text keeps moving through the quiet rather
/// than stopping dead, and a burst is caught up quickly.
const WORD_SPREAD_MS: u64 = 1_200;

/// The longest gap between words, with one word left waiting.
pub(super) const MAX_WORD_GAP_MS: u64 = 220;

/// How long what is left is given to go out once the model has finished.
pub(super) const FINISH_MS: u64 = 1_200;

/// The longest gap between words once the model has finished.
const FINISH_WORD_GAP_MS: u64 = 80;

/// The shortest gap between words, however far behind they are.
const MIN_WORD_GAP_MS: u64 = 10;

/// Words queued before the first of them goes out. Claude sends the opening
/// word or two, goes quiet for up to a second, then sends the rest; starting
/// on the opening alone would show it and then stand still. Held back, the
/// quiet is spent in the scene, which looks like the network still working.
const START_WORDS: usize = 8;

/// How long the opening is held back at most, for a message that is short or
/// slow to come.
pub(super) const START_WAIT_MS: u64 = 1_500;

impl Generation {
    pub fn new(
        rx: Receiver<GenMsg>,
        handle: JoinHandle<()>,
        feed: crate::panel::commit_art::Feed,
    ) -> Self {
        Self {
            rx,
            handle: Some(handle),
            output: String::new(),
            spinner: 0,
            scene: crate::panel::commit_art::fresh_seed(),
            arrivals: Vec::new(),
            first_output_ms: None,
            feed,
            queued: VecDeque::new(),
            next_word_ms: 0,
            finish_gap_ms: None,
            first_chunk_ms: None,
            finished: None,
        }
    }

    /// Take in a chunk of the message. The last queued word may be the
    /// first half of one this chunk finishes, so it is cut up again along
    /// with it.
    pub fn receive(&mut self, chunk: &str) {
        let mut text = self.queued.pop_back().unwrap_or_default();
        text.push_str(chunk);
        self.queued.extend(words(&text));
    }

    /// Forget everything received: what came was reasoning, not the message.
    pub fn restart(&mut self) {
        self.output.clear();
        self.arrivals.clear();
        self.queued.clear();
        self.first_output_ms = None;
        self.finish_gap_ms = None;
        self.first_chunk_ms = None;
    }

    /// Let out the words that are due by `now`. A last queued word with no
    /// space after it waits for more, or for the model to finish, since the
    /// rest of it may be on the way. With `animate` off, everything ready
    /// goes at once.
    pub fn release(&mut self, now: u64, animate: bool) {
        if !self.queued.is_empty() {
            self.first_chunk_ms.get_or_insert(now);
        }
        let starting = self.first_output_ms.is_none()
            && self.finished.is_none()
            && self.queued.len() < START_WORDS
            && self
                .first_chunk_ms
                .is_some_and(|first| now.saturating_sub(first) < START_WAIT_MS);
        if animate && starting {
            return;
        }
        let whole = self.finished.is_some()
            || self
                .queued
                .back()
                .is_some_and(|word| word.ends_with(char::is_whitespace));
        let mut ready = if whole {
            self.queued.len()
        } else {
            self.queued.len().saturating_sub(1)
        };
        if self.finished.is_some() && self.finish_gap_ms.is_none() {
            self.finish_gap_ms =
                Some((FINISH_MS / ready.max(1) as u64).clamp(MIN_WORD_GAP_MS, FINISH_WORD_GAP_MS));
        }
        // After a pause the clock has run on; the words that come next start
        // from now rather than all leaving at once to make up the time.
        let mut at = if now.saturating_sub(self.next_word_ms) > FINISH_WORD_GAP_MS {
            now
        } else {
            self.next_word_ms
        };
        self.arrivals
            .retain(|a| now.saturating_sub(a.at_ms) < crate::panel::commit_art::FLIGHT_TOTAL_MS);
        while ready > 0 && (!animate || at <= now) {
            let Some(word) = self.queued.pop_front() else {
                break;
            };
            let at_ms = if animate { at } else { now };
            self.arrivals.push(Arrival {
                start: self.output.chars().count(),
                len: word.chars().count(),
                at_ms,
            });
            self.output.push_str(&word);
            self.first_output_ms.get_or_insert(at_ms);
            at += self.finish_gap_ms.unwrap_or_else(|| {
                (WORD_SPREAD_MS / ready as u64).clamp(MIN_WORD_GAP_MS, MAX_WORD_GAP_MS)
            });
            ready -= 1;
        }
        self.next_word_ms = at;
    }

    /// Whether every word has left the network and landed in the text.
    pub fn settled(&self, now: u64) -> bool {
        self.queued.is_empty()
            && self
                .arrivals
                .iter()
                .all(|a| now.saturating_sub(a.at_ms) >= crate::panel::commit_art::FLIGHT_MS)
    }
}

/// `text` cut after each word, so every piece is one word with the space
/// that follows it.
pub(super) fn words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut after_space = false;
    for c in text.chars() {
        let starts_word = !c.is_whitespace()
            && after_space
            && words
                .last()
                .is_some_and(|word| word.chars().any(|c| !c.is_whitespace()));
        match words.last_mut() {
            Some(word) if !starts_word => word.push(c),
            _ => words.push(c.to_string()),
        }
        after_space = c.is_whitespace();
    }
    words
}

/// One word of the message as it left the model: where it sits in the
/// output, and when it left on the animation clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrival {
    /// First character of the word in the output.
    pub start: usize,
    pub len: usize,
    pub at_ms: u64,
}
