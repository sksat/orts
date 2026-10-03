//! Wall-clock pacing of the serve loop.
//!
//! [`Pacing`] says how a simulation advances against the wall clock;
//! [`RealtimeClock`] and [`LagWarnings`] are the arithmetic of the realtime
//! mode, kept free of the loop's channels so they can be tested on their own.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

/// How the serve loop paces a simulation against the wall clock.
///
/// On the wire as `"accelerated"` / `"realtime"`: a `start_simulation` may ask
/// for one, `info` says which one the simulation runs at, and an idle
/// `status` says which one the server starts a simulation at when not asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Pacing {
    /// Faster than real time: chunks of output intervals, each sent at a rate
    /// derived from `dt` and `stream_interval` (100x at the default `dt`).
    #[default]
    Accelerated,
    /// 1 sim s = 1 wall s (`--realtime`, a `start_simulation` asking for it,
    /// or any simulation with stream-io streams wired).
    Realtime,
}

impl Pacing {
    /// The pacing a fleet asked to run at `self` actually runs at: stream-io
    /// streams always run in real time, since the byte protocols on the other
    /// side of them assume wall-clock time.
    pub(super) fn for_fleet(self, has_streams: bool) -> Self {
        if has_streams { Self::Realtime } else { self }
    }
}

/// How far the simulation may fall behind the wall clock and still catch up.
///
/// A hiccup shorter than this (a slow interval, a stalled blocking pool) is
/// made up by stepping the next intervals without waiting. Beyond it the
/// clock is re-anchored instead: catching up would step and send many
/// intervals back to back, and a stream-io peer on the other side expects
/// wall-clock time, not a burst.
pub(super) const MAX_CATCH_UP: Duration = Duration::from_secs(1);

/// The shortest wall time between two "fell behind" warnings. A simulation
/// that cannot keep up re-anchors every time it falls [`MAX_CATCH_UP`]
/// behind; one line per occurrence would flood the log.
pub(super) const LAG_WARN_INTERVAL: Duration = Duration::from_secs(10);

/// Maps simulation time to the wall-clock instant it is due at.
///
/// Anchored once (`wall_origin` ↔ `sim_origin`) instead of per interval, so
/// the time the loop spends between intervals — timer rounding, command
/// handling, sending — does not accumulate into a drift.
#[derive(Debug, Clone, Copy)]
pub(super) struct RealtimeClock {
    wall_origin: Instant,
    sim_origin: f64,
}

impl RealtimeClock {
    /// A clock on which `sim_t` is due at `now`.
    pub(super) fn anchor(now: Instant, sim_t: f64) -> Self {
        Self {
            wall_origin: now,
            sim_origin: sim_t,
        }
    }

    /// The wall instant at which `sim_t` is due.
    ///
    /// A time at or before the origin, or one whose offset is not a
    /// representable duration (`NaN`, `±∞`), is due at the origin: the loop
    /// steps on instead of waiting forever.
    pub(super) fn due(&self, sim_t: f64) -> Instant {
        let offset = Duration::try_from_secs_f64(sim_t - self.sim_origin).unwrap_or(Duration::ZERO);
        self.wall_origin + offset
    }

    /// Re-anchor if `sim_t`, reached at `now`, is more than [`MAX_CATCH_UP`]
    /// behind the wall clock. Returns the lag that was dropped, if any.
    pub(super) fn drop_excess_lag(&mut self, now: Instant, sim_t: f64) -> Option<Duration> {
        let lag = now.saturating_duration_since(self.due(sim_t));
        if lag > MAX_CATCH_UP {
            *self = Self::anchor(now, sim_t);
            Some(lag)
        } else {
            None
        }
    }
}

/// Rate-limits the warnings for lag that [`RealtimeClock`] dropped.
#[derive(Debug, Default)]
pub(super) struct LagWarnings {
    last_warned: Option<Instant>,
    /// Lag dropped since the last warning, and how many times.
    pending: Duration,
    pending_count: u32,
}

/// What one warning reports: the lag dropped since the previous warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LagReport {
    pub dropped: Duration,
    pub times: u32,
}

impl LagWarnings {
    /// Record `dropped` lag at `now`. Returns a report to log when the first
    /// drop happens or [`LAG_WARN_INTERVAL`] has passed since the last one;
    /// otherwise the drop is folded into the next report.
    pub(super) fn record(&mut self, now: Instant, dropped: Duration) -> Option<LagReport> {
        self.pending += dropped;
        self.pending_count += 1;
        let due = self
            .last_warned
            .is_none_or(|last| now.saturating_duration_since(last) >= LAG_WARN_INTERVAL);
        if !due {
            return None;
        }
        self.last_warned = Some(now);
        let report = LagReport {
            dropped: self.pending,
            times: self.pending_count,
        };
        self.pending = Duration::ZERO;
        self.pending_count = 0;
        Some(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    /// Streams force realtime; without them the asked-for pacing stands.
    #[test]
    fn streams_force_realtime_and_nothing_else_does() {
        for asked in [Pacing::Accelerated, Pacing::Realtime] {
            assert_eq!(asked.for_fleet(true), Pacing::Realtime, "{asked:?}");
            assert_eq!(asked.for_fleet(false), asked);
        }
    }

    /// The wire spelling the viewer sends and reads.
    #[test]
    fn pacing_is_lowercase_on_the_wire() {
        for (pacing, wire) in [
            (Pacing::Accelerated, "\"accelerated\""),
            (Pacing::Realtime, "\"realtime\""),
        ] {
            assert_eq!(serde_json::to_string(&pacing).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Pacing>(wire).unwrap(), pacing);
        }
    }

    /// Sim time maps onto the wall clock 1:1 from the anchor, including an
    /// anchor at a sim time other than zero (a resumed run).
    #[test]
    fn sim_time_is_due_one_to_one_from_the_anchor() {
        let base = Instant::now();
        let clock = RealtimeClock::anchor(base, 120.0);
        assert_eq!(clock.due(120.0), base);
        assert_eq!(clock.due(130.0), base + secs(10.0));
        assert_eq!(clock.due(120.5), base + secs(0.5));
    }

    /// The due instant is computed from the anchor, not by adding up steps,
    /// so a thousand 0.1 s intervals land on 100 s to the nanosecond.
    #[test]
    fn due_does_not_accumulate_per_step_error() {
        let base = Instant::now();
        let clock = RealtimeClock::anchor(base, 0.0);
        assert_eq!(clock.due(1000.0 * 0.1), base + secs(100.0));
    }

    /// Times that are not ahead of the origin are due at the origin, so the
    /// loop never waits forever or panics on a time it cannot represent.
    #[test]
    fn times_not_ahead_of_the_origin_are_due_at_the_origin() {
        let base = Instant::now();
        let clock = RealtimeClock::anchor(base, 10.0);
        for t in [5.0, 10.0, f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
            assert_eq!(clock.due(t), base, "sim_t = {t}");
        }
    }

    /// Lag up to the catch-up limit is kept: the anchor stays, and the next
    /// intervals are due in the past, so the loop steps them without waiting.
    #[test]
    fn lag_within_the_limit_is_caught_up() {
        let base = Instant::now();
        let mut clock = RealtimeClock::anchor(base, 0.0);
        let now = base + secs(10.0) + MAX_CATCH_UP;
        assert_eq!(clock.drop_excess_lag(now, 10.0), None);
        assert_eq!(clock.due(10.0), base + secs(10.0), "anchor unchanged");
    }

    /// Lag past the limit is dropped: the sim time reached is due now, and the
    /// run goes on at 1:1 from there instead of bursting to catch up.
    #[test]
    fn lag_past_the_limit_re_anchors_at_now() {
        let base = Instant::now();
        let mut clock = RealtimeClock::anchor(base, 0.0);
        let now = base + secs(13.0);
        assert_eq!(clock.drop_excess_lag(now, 10.0), Some(secs(3.0)));
        assert_eq!(clock.due(10.0), now);
        assert_eq!(clock.due(11.0), now + secs(1.0));
    }

    /// A sim ahead of the wall clock has no lag to drop.
    #[test]
    fn being_ahead_is_not_lag() {
        let base = Instant::now();
        let mut clock = RealtimeClock::anchor(base, 0.0);
        assert_eq!(clock.drop_excess_lag(base, 10.0), None);
        assert_eq!(clock.due(10.0), base + secs(10.0));
    }

    /// The first drop is reported at once; drops within the interval after it
    /// are folded into the next report, which carries their total and count.
    #[test]
    fn lag_warnings_are_rate_limited_and_summed() {
        let base = Instant::now();
        let mut warnings = LagWarnings::default();
        assert_eq!(
            warnings.record(base, secs(2.0)),
            Some(LagReport {
                dropped: secs(2.0),
                times: 1
            })
        );
        assert_eq!(warnings.record(base + secs(3.0), secs(1.5)), None);
        assert_eq!(warnings.record(base + secs(6.0), secs(1.5)), None);
        assert_eq!(
            warnings.record(base + LAG_WARN_INTERVAL, secs(2.0)),
            Some(LagReport {
                dropped: secs(5.0),
                times: 3
            })
        );
    }
}
