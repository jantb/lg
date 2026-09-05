//! What the model server did with a request, and what it is doing right now.
//!
//! Throughput is a property of the server rather than of any one panel's job,
//! so it is recorded here once by the streaming layer and read from here by
//! whatever wants to draw it. That keeps the numbers out of every job type's
//! message enum, and means a task that consumes its own stream on a worker
//! thread reports the same way one streamed into a pane does.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// What one finished request cost, as the server measured it.
///
/// Every field is what the server reported, not what lg guessed: token counts
/// are the server's tokenizer, and the rates are its own clocks. A field the
/// server said nothing about stays zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenStats {
    pub prompt_tokens: u64,
    /// Prompt tokens served from the server's cache rather than read again.
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    /// Tokens per second reading the prompt.
    pub prefill_tps: f64,
    /// Tokens per second writing the answer.
    pub decode_tps: f64,
    /// The answer stopped because it ran out of budget, not because it ended.
    /// The one field a caller acts on rather than displays.
    pub truncated: bool,
    /// The model that actually answered, which need not be the one asked for.
    pub served_model: Option<String>,
}

impl GenStats {
    /// Whether the server reported throughput at all. A request that ended
    /// without counters has stats worth nothing to a reader.
    pub fn is_measured(&self) -> bool {
        self.prefill_tps > 0.0 || self.decode_tps > 0.0 || self.completion_tokens > 0
    }

    /// Both rates as one figure, prefill first. They differ by roughly an
    /// order of magnitude, so which is which is never in doubt once both are
    /// shown. `None` when the server reported neither.
    pub fn rates(&self) -> Option<String> {
        (self.prefill_tps > 0.0 || self.decode_tps > 0.0)
            .then(|| format!("{:.0}/{:.1} tok/s", self.prefill_tps, self.decode_tps))
    }

    /// The request in one line, for a status message: what went in, how much
    /// of it was already cached, what came out, and how fast both halves ran.
    /// `None` when the server measured nothing worth printing.
    pub fn summary(&self) -> Option<String> {
        if !self.is_measured() {
            return None;
        }
        let mut parts = Vec::new();
        if self.prompt_tokens > 0 {
            let cached = if self.cached_tokens > 0 {
                format!(" ({} cached)", self.cached_tokens)
            } else {
                String::new()
            };
            parts.push(format!("{} in{cached}", self.prompt_tokens));
        }
        if self.completion_tokens > 0 {
            parts.push(format!("{} out", self.completion_tokens));
        }
        parts.extend(self.rates());
        Some(parts.join(" \u{b7} "))
    }
}

/// What the server is doing for a request that has not finished yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmPhase {
    /// Sent, nothing back yet: the server is still reading the prompt, which
    /// was this many bytes long.
    Prefill {
        elapsed: Duration,
        prompt_bytes: usize,
    },
    /// Tokens are arriving: how long for, and how many so far.
    Decode { elapsed: Duration, tokens: u64 },
}

impl LlmPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prefill { .. } => "prefill",
            Self::Decode { .. } => "decode",
        }
    }

    pub fn elapsed(self) -> Duration {
        match self {
            Self::Prefill { elapsed, .. } | Self::Decode { elapsed, .. } => elapsed,
        }
    }

    /// Tokens per second so far, measured by lg rather than by the server, so
    /// it is available while the request is still running, which the
    /// server's own figure is not. Decoding counts the chunks that have
    /// arrived, roughly one token each. Prefill has nothing to count, so it
    /// is estimated from the prompt's size at `BYTES_PER_TOKEN` and marked
    /// as an estimate. `None` until the phase has run long enough for the
    /// figure to mean something.
    pub fn live_tps(self) -> Option<LiveRate> {
        if self.elapsed() < Duration::from_millis(500) {
            return None;
        }
        let secs = self.elapsed().as_secs_f64();
        Some(match self {
            Self::Decode { tokens, .. } => LiveRate {
                tps: tokens as f64 / secs,
                estimated: false,
            },
            Self::Prefill { prompt_bytes, .. } => LiveRate {
                tps: prompt_bytes as f64 / BYTES_PER_TOKEN / secs,
                estimated: true,
            },
        })
    }

    /// The phase as a reader wants it: what is happening, for how long, and
    /// how fast.
    pub fn describe(self) -> String {
        let mut text = format!("{} {}", self.label(), compact_duration(self.elapsed()));
        if let Some(rate) = self.live_tps() {
            text.push_str(&format!(" \u{b7} {rate}"));
        }
        text
    }
}

/// How many bytes of prompt one token stands for, on average, for English
/// text and code. Rough, and said to be: the estimate is for watching the
/// prefill move, not for comparing servers.
const BYTES_PER_TOKEN: f64 = 4.0;

/// A throughput lg measured itself while a request runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveRate {
    pub tps: f64,
    /// Whether the token count behind it was estimated rather than counted.
    pub estimated: bool,
}

impl std::fmt::Display for LiveRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.estimated {
            write!(f, "~{:.0} tok/s", self.tps)
        } else {
            write!(f, "{:.1} tok/s", self.tps)
        }
    }
}

/// A duration short enough to glance at.
pub fn compact_duration(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{:.1}s", elapsed.as_secs_f64())
    }
}

/// One line about the request in flight and the one before it: the live phase
/// with its running speed, then the last request's measured prefill and
/// decode rates for comparison. `None` when nothing is in flight.
pub fn progress() -> Option<String> {
    let mut text = phase()?.describe();
    if let Some(rates) = last_stats().and_then(|stats| stats.rates()) {
        text.push_str(&format!(" \u{b7} last {rates}"));
    }
    Some(text)
}

struct InFlight {
    id: u64,
    started: Instant,
    /// How long the prompt was, for estimating prefill speed before the
    /// server has said anything.
    prompt_bytes: usize,
    first_token: Option<Instant>,
    /// Chunks seen so far, standing in for tokens.
    tokens: u64,
}

#[derive(Default)]
struct Registry {
    in_flight: Vec<InFlight>,
    last: Option<GenStats>,
    next_id: u64,
}

impl Registry {
    const fn new() -> Self {
        Self {
            in_flight: Vec::new(),
            last: None,
            next_id: 0,
        }
    }

    fn begin(&mut self, started: Instant, prompt_bytes: usize) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.in_flight.push(InFlight {
            id,
            started,
            prompt_bytes,
            first_token: None,
            tokens: 0,
        });
        id
    }

    /// One more chunk has arrived; the first also ends the prefill phase.
    fn note_token(&mut self, id: u64, at: Instant) {
        if let Some(entry) = self.in_flight.iter_mut().find(|entry| entry.id == id) {
            entry.first_token.get_or_insert(at);
            entry.tokens += 1;
        }
    }

    /// Retire a request. Stats worth reading replace what the last one left;
    /// a request that ended without counters leaves the previous numbers up
    /// rather than blanking the readout.
    fn finish(&mut self, id: u64, stats: Option<GenStats>) {
        self.in_flight.retain(|entry| entry.id != id);
        if let Some(stats) = stats.filter(GenStats::is_measured) {
            self.last = Some(stats);
        }
    }

    /// The phase of the request that has been waiting longest, which is the one
    /// a reader is actually waiting on.
    fn phase_at(&self, now: Instant) -> Option<LlmPhase> {
        let oldest = self.in_flight.iter().min_by_key(|entry| entry.started)?;
        Some(match oldest.first_token {
            Some(at) => LlmPhase::Decode {
                elapsed: now.saturating_duration_since(at),
                tokens: oldest.tokens,
            },
            None => LlmPhase::Prefill {
                elapsed: now.saturating_duration_since(oldest.started),
                prompt_bytes: oldest.prompt_bytes,
            },
        })
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

/// A poisoned registry is still a usable one: throughput numbers are a readout,
/// and losing them is never a reason to take the app down with it.
fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|err| err.into_inner())
}

/// Registers one request for as long as it is in flight.
///
/// Dropping it retires the request, so a path that bails out early — a refused
/// connection, a receiver that went away mid-stream — cannot leave the readout
/// stuck reporting work that is no longer happening.
pub(super) struct Tracked {
    id: u64,
    stats: Option<GenStats>,
}

impl Tracked {
    /// Register a request whose prompt is `prompt_bytes` long.
    pub(super) fn new(prompt_bytes: usize) -> Self {
        Self {
            id: registry().begin(Instant::now(), prompt_bytes),
            stats: None,
        }
    }

    /// A chunk of the answer has arrived. The first one ends the prefill
    /// phase; every one counts towards the live decode rate.
    pub(super) fn note_token(&mut self) {
        registry().note_token(self.id, Instant::now());
    }

    /// What the server said this request cost, reported when the request
    /// retires.
    pub(super) fn report(&mut self, stats: GenStats) {
        self.stats = Some(stats);
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        registry().finish(self.id, self.stats.take());
    }
}

/// What the server is doing right now, or `None` when nothing is in flight.
pub fn phase() -> Option<LlmPhase> {
    registry().phase_at(Instant::now())
}

/// What the last measured request cost.
pub fn last_stats() -> Option<GenStats> {
    registry().last.clone()
}

/// Drop the last request's numbers. They name the model that answered, and
/// once a different one has been chosen that name is a fact about the past.
pub fn forget_last_stats() {
    registry().last = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(decode_tps: f64) -> GenStats {
        GenStats {
            completion_tokens: 32,
            decode_tps,
            ..GenStats::default()
        }
    }

    #[test]
    fn a_summary_names_what_went_in_what_came_out_and_how_fast() {
        let stats = GenStats {
            prompt_tokens: 1456,
            cached_tokens: 1455,
            completion_tokens: 10,
            prefill_tps: 13319.2,
            decode_tps: 30.9,
            ..GenStats::default()
        };
        let text = stats.summary().unwrap();
        for needle in ["1456", "1455", "10", "13319", "30.9"] {
            assert!(text.contains(needle), "{text} lacks {needle}");
        }
        assert!(GenStats::default().summary().is_none());
    }

    #[test]
    fn a_request_with_nothing_back_yet_reads_as_prefill() {
        let mut registry = Registry::new();
        let start = Instant::now();
        registry.begin(start, 4_000);

        let phase = registry.phase_at(start + Duration::from_secs(3)).unwrap();

        assert_eq!(phase.label(), "prefill");
        assert_eq!(phase.elapsed(), Duration::from_secs(3));
    }

    #[test]
    fn the_first_token_moves_a_request_into_decode() {
        let mut registry = Registry::new();
        let start = Instant::now();
        let id = registry.begin(start, 4_000);
        registry.note_token(id, start + Duration::from_secs(4));

        let phase = registry.phase_at(start + Duration::from_secs(9)).unwrap();

        assert_eq!(phase.label(), "decode");
        assert_eq!(
            phase.elapsed(),
            Duration::from_secs(5),
            "decode is timed from the first token, not from the request"
        );
    }

    #[test]
    fn a_finished_request_is_no_longer_in_flight() {
        let mut registry = Registry::new();
        let start = Instant::now();
        let id = registry.begin(start, 4_000);
        registry.finish(id, Some(measured(16.9)));

        assert!(registry.phase_at(start).is_none());
        assert_eq!(registry.last.as_ref().unwrap().decode_tps, 16.9);
    }

    /// Two tasks can overlap, and the one a reader is waiting on is whichever
    /// has been waiting longest.
    #[test]
    fn the_longest_running_request_is_the_one_reported() {
        let mut registry = Registry::new();
        let start = Instant::now();
        let first = registry.begin(start, 4_000);
        registry.begin(start + Duration::from_secs(1), 4_000);
        registry.note_token(first, start + Duration::from_secs(2));

        assert_eq!(
            registry
                .phase_at(start + Duration::from_secs(3))
                .unwrap()
                .label(),
            "decode"
        );
    }

    #[test]
    fn an_unmeasured_end_leaves_the_last_numbers_alone() {
        let mut registry = Registry::new();
        let kept = registry.begin(Instant::now(), 4_000);
        registry.finish(kept, Some(measured(16.9)));

        let bare = registry.begin(Instant::now(), 4_000);
        registry.finish(bare, Some(GenStats::default()));

        assert_eq!(
            registry.last.as_ref().unwrap().decode_tps,
            16.9,
            "a request the server sent no counters for must not blank the readout"
        );
    }

    #[test]
    fn the_live_rate_counts_chunks_over_decode_time() {
        let mut registry = Registry::new();
        let start = Instant::now();
        let id = registry.begin(start, 4_000);
        for i in 0..=40 {
            registry.note_token(id, start + Duration::from_millis(1_000 + i * 100));
        }
        // 41 chunks, the first at 1.0s, read at 5.0s: four seconds of decode.
        let phase = registry.phase_at(start + Duration::from_secs(5)).unwrap();
        let rate = phase.live_tps().expect("decoding long enough to measure");
        assert!((rate.tps - 10.25).abs() < 0.01, "{rate}");
        assert!(!rate.estimated, "decoded chunks are counted, not guessed");
        assert!(phase.describe().contains("tok/s"), "{}", phase.describe());
        assert!(
            LlmPhase::Prefill {
                elapsed: Duration::from_secs(2),
                prompt_bytes: 8_000
            }
            .live_tps()
            .is_some_and(|rate| rate.estimated && (rate.tps - 1_000.0).abs() < 1.0),
            "prefill speed is estimated from the prompt's size"
        );
    }
}
