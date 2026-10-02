//! When the loop redraws, and how long it waits for input in between.
//!
//! A frame is only drawn when something on screen could look different: input
//! was handled, a job or session delivered something, or an animation is due
//! its next step. Everything else the loop does — draining jobs, reading
//! sessions, checking timers — happens on wakes that draw nothing.

use std::time::{Duration, Instant};

use crate::{
    config::{
        ANIMATION_FRAME_MS, ANIMATION_STEP_MS, BACKGROUND_SESSION_TICK_MS, MIN_POLL_MS,
        SESSION_LIVE_MS, SESSION_QUIET_TICK_MS, SESSION_TICK_MS, TICK_MS,
    },
    state::AppState,
};

/// How fast the loop has to go for what is on screen right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pace {
    /// Redraw at least this often, for something moving on screen.
    pub(super) animate: Option<Duration>,
    /// Wake at least this often to look for work, drawn or not.
    pub(super) wake: Duration,
}

/// The pace for `state`. `shown_live` is whether the session on screen wrote
/// or was typed into a moment ago; `sessions_more` whether a session has output
/// left over from the last pass.
///
/// Decorative motion — the orbiting modal frame, a settling status line — is
/// drawn at [`ANIMATION_FRAME_MS`], and only with decorative animations on. A
/// running job's spinner steps every [`ANIMATION_STEP_MS`] either way, and is
/// drawn at that rate when nothing smoother is wanted. Sessions out of sight
/// are read every [`BACKGROUND_SESSION_TICK_MS`] but never raise the frame
/// rate; their breathing dot is redrawn at the spinner's rate.
pub(super) fn pace(state: &AppState, shown_live: bool, sessions_more: bool) -> Pace {
    let jobs = state.any_job_running();
    let decorative = state.decorative_animations;
    let animate = if state.decorative_motion() || (decorative && jobs) {
        Some(ANIMATION_FRAME_MS)
    } else if jobs || state.wants_animation() {
        Some(ANIMATION_STEP_MS)
    } else {
        None
    };
    let mut wake = if state.session_view().is_some() {
        if shown_live {
            SESSION_TICK_MS
        } else {
            SESSION_QUIET_TICK_MS
        }
    } else if jobs {
        // A job's result lands within a frame of arriving.
        ANIMATION_FRAME_MS
    } else if state.sessions.any_running() {
        BACKGROUND_SESSION_TICK_MS
    } else {
        TICK_MS
    };
    if sessions_more {
        wake = wake.min(SESSION_TICK_MS);
    }
    if let Some(animate) = animate {
        wake = wake.min(animate);
    }
    Pace {
        animate: animate.map(Duration::from_millis),
        wake: Duration::from_millis(wake),
    }
}

/// What the loop knows about the frames it has drawn and the ones it owes.
#[derive(Debug, Default)]
pub(super) struct FrameClock {
    last_draw: Option<Instant>,
    /// Input was handled: the next pass draws, whatever was drawn last.
    urgent: bool,
    /// Something changed without being asked to: drawn once a frame interval
    /// has passed since the last one, so a stream of changes is drawn at most
    /// every [`SESSION_TICK_MS`] rather than once per change.
    pending: bool,
    /// When the session on screen last wrote or was typed into.
    shown_live_at: Option<Instant>,
}

impl FrameClock {
    /// Input was handled; draw it straight away.
    pub(super) fn input(&mut self) {
        self.urgent = true;
    }

    /// Something worth drawing changed on its own.
    pub(super) fn changed(&mut self) {
        self.pending = true;
    }

    /// The session on screen wrote something, or was typed into.
    pub(super) fn shown_live(&mut self, now: Instant) {
        self.shown_live_at = Some(now);
    }

    /// Whether the session on screen is live, as of `now`.
    pub(super) fn is_shown_live(&self, now: Instant) -> bool {
        self.shown_live_at
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(SESSION_LIVE_MS))
    }

    /// Whether a frame is owed at `now`.
    pub(super) fn due(&self, now: Instant, pace: Pace) -> bool {
        let Some(last) = self.last_draw else {
            return true;
        };
        let since = now.duration_since(last);
        self.urgent
            || (self.pending && since >= Duration::from_millis(SESSION_TICK_MS))
            || pace.animate.is_some_and(|every| since >= every)
    }

    /// A frame was drawn at `now`.
    pub(super) fn drawn(&mut self, now: Instant) {
        self.last_draw = Some(now);
        self.urgent = false;
        self.pending = false;
    }

    /// How long to wait for input at `now`: until the next frame is owed or
    /// the pace says to look again, whichever comes first — or until
    /// `deadline`, when something else is due by then. Never less than
    /// [`MIN_POLL_MS`], so an overrunning frame cannot turn the loop into a
    /// spin.
    pub(super) fn timeout(&self, now: Instant, pace: Pace, deadline: Option<Instant>) -> Duration {
        let mut wait = pace.wake;
        if let Some(last) = self.last_draw {
            let since = now.duration_since(last);
            if self.urgent {
                wait = Duration::ZERO;
            }
            if self.pending {
                wait = wait.min(Duration::from_millis(SESSION_TICK_MS).saturating_sub(since));
            }
            if let Some(every) = pace.animate {
                wait = wait.min(every.saturating_sub(since));
            }
        } else {
            wait = Duration::ZERO;
        }
        if let Some(deadline) = deadline {
            wait = wait.min(deadline.saturating_duration_since(now));
        }
        wait.max(Duration::from_millis(MIN_POLL_MS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Modal;

    fn drawn_at(now: Instant) -> FrameClock {
        let mut clock = FrameClock::default();
        clock.drawn(now);
        clock
    }

    #[test]
    fn an_idle_screen_waits_for_input_and_draws_nothing() {
        let state = AppState::new();
        let now = Instant::now();
        let clock = drawn_at(now);
        let pace = pace(&state, false, false);

        assert_eq!(pace.animate, None);
        assert!(clock.timeout(now, pace, None) >= Duration::from_millis(TICK_MS));
        assert!(
            !clock.due(now + Duration::from_secs(5), pace),
            "nothing changed, so nothing is redrawn"
        );
    }

    #[test]
    fn the_first_frame_is_drawn_straight_away() {
        let state = AppState::new();
        assert!(FrameClock::default().due(Instant::now(), pace(&state, false, false)));
    }

    #[test]
    fn handled_input_is_drawn_on_the_next_pass() {
        let state = AppState::new();
        let now = Instant::now();
        let mut clock = drawn_at(now);
        clock.input();
        assert!(clock.due(now, pace(&state, false, false)));
    }

    #[test]
    fn a_stream_of_changes_is_drawn_at_most_once_per_frame_interval() {
        let state = AppState::new();
        let now = Instant::now();
        let mut clock = drawn_at(now);
        let pace = pace(&state, false, false);
        clock.changed();

        assert!(!clock.due(now + Duration::from_millis(2), pace));
        let wait = clock.timeout(now + Duration::from_millis(2), pace, None);
        assert!(
            wait <= Duration::from_millis(SESSION_TICK_MS),
            "the loop wakes for the frame it owes: {wait:?}"
        );
        assert!(clock.due(now + Duration::from_millis(SESSION_TICK_MS), pace));
    }

    #[test]
    fn an_open_modal_animates_at_about_thirty_frames_a_second() {
        let mut state = AppState::new();
        state.modal = Modal::Worktree;
        let every = pace(&state, false, false).animate.expect("the frame moves");
        assert!(
            (Duration::from_millis(30)..=Duration::from_millis(40)).contains(&every),
            "{every:?}"
        );
    }

    #[test]
    fn without_decorative_animations_an_open_modal_holds_still() {
        let mut state = AppState::new();
        state.decorative_animations = false;
        state.modal = Modal::Worktree;
        let pace = pace(&state, false, false);
        assert_eq!(pace.animate, None);
        assert_eq!(pace.wake, Duration::from_millis(TICK_MS));
    }

    #[test]
    fn an_overrunning_frame_never_turns_the_loop_into_a_spin() {
        let mut state = AppState::new();
        state.modal = Modal::Worktree;
        let now = Instant::now();
        let mut clock = drawn_at(now);
        clock.changed();
        let late = now + Duration::from_secs(1);
        let wait = clock.timeout(late, pace(&state, true, true), Some(now));
        assert!(wait >= Duration::from_millis(MIN_POLL_MS));
    }

    #[test]
    fn a_session_on_screen_counts_as_live_for_a_moment_after_it_writes() {
        let mut clock = FrameClock::default();
        let now = Instant::now();
        clock.shown_live(now);
        assert!(clock.is_shown_live(now + Duration::from_millis(10)));
        assert!(!clock.is_shown_live(now + Duration::from_millis(SESSION_LIVE_MS + 1)));
    }

    #[test]
    fn a_deadline_wakes_the_loop_in_time() {
        let state = AppState::new();
        let now = Instant::now();
        let clock = drawn_at(now);
        let wait = clock.timeout(
            now,
            pace(&state, false, false),
            Some(now + Duration::from_millis(40)),
        );
        assert!(wait <= Duration::from_millis(40));
    }
}
