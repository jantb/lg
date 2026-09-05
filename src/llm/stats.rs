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
}

/// What the server is doing for a request that has not finished yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmPhase {
    /// Sent, nothing back yet: the server is still reading the prompt.
    Prefill(Duration),
    /// Tokens are arriving.
    Decode(Duration),
}

impl LlmPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prefill(_) => "prefill",
            Self::Decode(_) => "decode",
        }
    }

    pub fn elapsed(self) -> Duration {
        match self {
            Self::Prefill(elapsed) | Self::Decode(elapsed) => elapsed,
        }
    }
}

struct InFlight {
    id: u64,
    started: Instant,
    first_token: Option<Instant>,
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

    fn begin(&mut self, started: Instant) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.in_flight.push(InFlight {
            id,
            started,
            first_token: None,
        });
        id
    }

    fn note_first_token(&mut self, id: u64, at: Instant) {
        if let Some(entry) = self.in_flight.iter_mut().find(|entry| entry.id == id)
            && entry.first_token.is_none()
        {
            entry.first_token = Some(at);
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
            Some(at) => LlmPhase::Decode(now.saturating_duration_since(at)),
            None => LlmPhase::Prefill(now.saturating_duration_since(oldest.started)),
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
    pub(super) fn new() -> Self {
        Self {
            id: registry().begin(Instant::now()),
            stats: None,
        }
    }

    /// The prompt has been read and the answer has started.
    pub(super) fn note_first_token(&self) {
        registry().note_first_token(self.id, Instant::now());
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
    fn a_request_with_nothing_back_yet_reads_as_prefill() {
        let mut registry = Registry::new();
        let start = Instant::now();
        registry.begin(start);

        let phase = registry.phase_at(start + Duration::from_secs(3)).unwrap();

        assert_eq!(phase.label(), "prefill");
        assert_eq!(phase.elapsed(), Duration::from_secs(3));
    }

    #[test]
    fn the_first_token_moves_a_request_into_decode() {
        let mut registry = Registry::new();
        let start = Instant::now();
        let id = registry.begin(start);
        registry.note_first_token(id, start + Duration::from_secs(4));

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
        let id = registry.begin(start);
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
        let first = registry.begin(start);
        registry.begin(start + Duration::from_secs(1));
        registry.note_first_token(first, start + Duration::from_secs(2));

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
        let kept = registry.begin(Instant::now());
        registry.finish(kept, Some(measured(16.9)));

        let bare = registry.begin(Instant::now());
        registry.finish(bare, Some(GenStats::default()));

        assert_eq!(
            registry.last.as_ref().unwrap().decode_tps,
            16.9,
            "a request the server sent no counters for must not blank the readout"
        );
    }
}
