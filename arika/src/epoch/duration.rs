//! Scale-invariant [`Duration`] measured in SI seconds.

/// Scale-invariant duration measured in SI (TAI) seconds.
///
/// Does not carry a scale tag because SI seconds tick uniformly regardless of
/// the reference time scale. UTC display arithmetic (e.g. "翌日同時刻") is NOT
/// provided — use [`Epoch::<Utc>::add_si_seconds`](super::Epoch::add_si_seconds)
/// which correctly handles leap second boundaries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duration {
    si_seconds: f64,
}

impl Duration {
    /// Construct from SI seconds.
    pub const fn from_si_seconds(s: f64) -> Self {
        Duration { si_seconds: s }
    }

    /// Construct from minutes (= 60 SI seconds).
    pub const fn from_minutes(m: f64) -> Self {
        Duration {
            si_seconds: m * 60.0,
        }
    }

    /// Construct from hours (= 3600 SI seconds).
    pub const fn from_hours(h: f64) -> Self {
        Duration {
            si_seconds: h * 3600.0,
        }
    }

    /// Return the duration in SI seconds.
    pub fn as_si_seconds(&self) -> f64 {
        self.si_seconds
    }
}
