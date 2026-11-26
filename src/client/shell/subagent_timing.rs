//! Timing text and redraw clock for subagent rows in the spaces tree.
//!
//! Rendering stays pure: the wall clock is read once per composed frame
//! ([`SubagentClock::begin_compose`]) and the row renderer only reads the
//! frame's `now_ms`. The clock can be pinned for tests.

use std::cell::Cell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::api::schema::SubagentStatus;

/// How often the client repaints while a running subagent row is visible.
pub(super) const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// Elapsed time with second precision under an hour: `45s`, `2m14s`, `1h3m`,
/// `2d4h`.
pub(super) fn format_elapsed(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{}s", secs / 60, secs % 60),
        _ => format_coarse(secs),
    }
}

/// Coarse age: `45s`, `3m`, `1h3m`, `2d4h`.
pub(super) fn format_ago(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format_coarse(secs),
    }
}

fn format_coarse(secs: u64) -> String {
    let minutes = secs / 60;
    if minutes < 24 * 60 {
        format!("{}h{}m", minutes / 60, minutes % 60)
    } else {
        let hours = minutes / 60;
        format!("{}d{}h", hours / 24, hours % 24)
    }
}

/// The right-aligned timing text of a subagent row, if its times are known.
///
/// Running: elapsed since start. Done and interrupted: `<ran>·<ago> ago`
/// (`ended_ms` is the finish time or, for interrupted, the last activity), or
/// just `<ago> ago` when the start is unknown.
pub(super) fn row_timing(
    state: SubagentStatus,
    started_ms: Option<u64>,
    ended_ms: Option<u64>,
    now_ms: u64,
) -> Option<String> {
    match state {
        SubagentStatus::Running => Some(format_elapsed(now_ms.saturating_sub(started_ms?) / 1000)),
        SubagentStatus::Done | SubagentStatus::Interrupted => {
            let ended = ended_ms?;
            let ago = format_ago(now_ms.saturating_sub(ended) / 1000);
            Some(match started_ms {
                Some(started) => format!(
                    "{}·{ago} ago",
                    format_elapsed(ended.saturating_sub(started) / 1000)
                ),
                None => format!("{ago} ago"),
            })
        }
    }
}

/// Frame time and redraw scheduling for subagent timing.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubagentClock {
    /// Wall-clock time of the frame being composed, Unix milliseconds.
    pub(super) now_ms: u64,
    /// Pins the frame time instead of reading the system clock (tests).
    pub(super) fixed_ms: Option<u64>,
    /// Set while composing when a running subagent row with timing is
    /// drawn, so the tick only runs while one is visible.
    drew_running: Cell<bool>,
    last_tick: Option<Instant>,
}

impl SubagentClock {
    /// Latches the frame time and clears the visibility flag. Called once per
    /// composed frame, before rendering.
    pub(super) fn begin_compose(&mut self) {
        self.now_ms = self.fixed_ms.unwrap_or_else(system_unix_ms);
        self.drew_running.set(false);
    }

    /// Records that a running row with a live timer was drawn.
    pub(super) fn mark_running_drawn(&self) {
        self.drew_running.set(true);
    }

    /// When the next repaint is due, if a running row is on screen.
    pub(super) fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.drew_running.get() {
            return None;
        }
        Some(
            self.last_tick
                .map_or(now + TICK_INTERVAL, |tick| tick + TICK_INTERVAL),
        )
    }

    /// Returns true when the visible timers need a repaint.
    pub(super) fn tick(&mut self, now: Instant) -> bool {
        if !self.drew_running.get() {
            self.last_tick = None;
            return false;
        }
        match self.last_tick {
            Some(last) if now.saturating_duration_since(last) >= TICK_INTERVAL => {
                self.last_tick = Some(now);
                true
            }
            Some(_) => false,
            None => {
                self.last_tick = Some(now);
                false
            }
        }
    }
}

fn system_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_formats() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(45), "45s");
        assert_eq!(format_elapsed(60), "1m0s");
        assert_eq!(format_elapsed(134), "2m14s");
        assert_eq!(format_elapsed(3599), "59m59s");
        assert_eq!(format_elapsed(3780), "1h3m");
        assert_eq!(format_elapsed(26 * 3600 + 120), "1d2h");
    }

    #[test]
    fn ago_formats_are_coarse() {
        assert_eq!(format_ago(12), "12s");
        assert_eq!(format_ago(180), "3m");
        assert_eq!(format_ago(194), "3m");
        assert_eq!(format_ago(3780), "1h3m");
    }

    #[test]
    fn row_timing_per_state() {
        let ms = |s: u64| s * 1000;
        assert_eq!(
            row_timing(SubagentStatus::Running, Some(ms(100)), None, ms(234)),
            Some("2m14s".into())
        );
        assert_eq!(
            row_timing(SubagentStatus::Running, None, None, ms(234)),
            None
        );
        assert_eq!(
            row_timing(SubagentStatus::Done, Some(ms(100)), Some(ms(112)), ms(292)),
            Some("12s·3m ago".into())
        );
        assert_eq!(
            row_timing(
                SubagentStatus::Interrupted,
                Some(ms(0)),
                Some(ms(75)),
                ms(75 + 3700)
            ),
            Some("1m15s·1h1m ago".into())
        );
        assert_eq!(
            row_timing(SubagentStatus::Done, None, Some(ms(10)), ms(20)),
            Some("10s ago".into())
        );
        assert_eq!(
            row_timing(SubagentStatus::Done, Some(ms(1)), None, ms(20)),
            None
        );
        // A clock behind the recorded times never underflows.
        assert_eq!(
            row_timing(SubagentStatus::Running, Some(ms(500)), None, ms(100)),
            Some("0s".into())
        );
    }

    #[test]
    fn clock_ticks_only_while_a_running_row_is_drawn() {
        let mut clock = SubagentClock::default();
        let start = Instant::now();
        clock.begin_compose();
        assert_eq!(clock.next_deadline(start), None);
        assert!(!clock.tick(start + TICK_INTERVAL * 5));

        clock.begin_compose();
        clock.mark_running_drawn();
        assert_eq!(clock.next_deadline(start), Some(start + TICK_INTERVAL));
        assert!(!clock.tick(start)); // arms the timer
        assert_eq!(clock.next_deadline(start), Some(start + TICK_INTERVAL));
        assert!(!clock.tick(start + TICK_INTERVAL / 2));
        assert!(clock.tick(start + TICK_INTERVAL));
        assert_eq!(
            clock.next_deadline(start + TICK_INTERVAL),
            Some(start + TICK_INTERVAL * 2)
        );

        clock.begin_compose(); // next frame draws no running row
        assert!(!clock.tick(start + TICK_INTERVAL * 3));
        assert_eq!(clock.next_deadline(start), None);
    }

    #[test]
    fn begin_compose_uses_the_pinned_time() {
        let mut clock = SubagentClock {
            fixed_ms: Some(42_000),
            ..SubagentClock::default()
        };
        clock.begin_compose();
        assert_eq!(clock.now_ms, 42_000);
        clock.fixed_ms = None;
        clock.begin_compose();
        assert!(clock.now_ms > 1_000_000_000_000);
    }
}
