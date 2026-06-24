//! GPS Time week-number / seconds-of-week representation.
//!
//! GNSS receivers report time as a `(week number, seconds of week)` pair rather
//! than a Julian Date. These newtypes carry the invariants that bare `u32` /
//! `f64` would lose:
//!
//! - [`GpsWeek`] is the **continuous** week count from the GPS epoch
//!   (1980-01-06), with no 1024-week rollover. The value broadcast in the
//!   navigation message is only 10 bits and *does* roll over; resolve such a
//!   value with [`GpsWeek::from_broadcast`].
//! - [`SecondsOfWeek`] is range-checked to `[0, 604800)`.
//!
//! Modelling these as types — rather than asking [`Epoch::<Gps>::from_week_seconds`]
//! to pick a rollover convention — keeps the constructor unambiguous and makes
//! broadcast de-rolling an explicit, separately testable step.

use super::{Epoch, GPS_EPOCH_JD, Gps, Precision};
// `floor`/`round` resolve to inherent std methods under `std`; this import
// provides them for `no_std` (libm) and is unused otherwise.
#[allow(unused_imports)]
use crate::math::F64Ext;

/// Seconds in one GPS week (`7 × 86400`).
const SECONDS_PER_WEEK: f64 = 604_800.0;
/// Width of the broadcast (10-bit) week-number field.
const BROADCAST_WEEK_MODULUS: i64 = 1024;

/// A continuous GPS week number, counted from the GPS epoch (1980-01-06) with
/// **no** 1024-week rollover.
///
/// The 10-bit value broadcast in the GPS navigation message rolls over every
/// 1024 weeks (~19.6 years; last rollovers 1999-08-22 and 2019-04-06). Use
/// [`from_broadcast`](Self::from_broadcast) to lift such a raw value into a
/// continuous week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GpsWeek(u32);

impl GpsWeek {
    /// Wrap a continuous (already de-rolled) week number.
    pub const fn new(continuous_week: u32) -> Self {
        Self(continuous_week)
    }

    /// The continuous week number.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Resolve a broadcast week into a continuous week, choosing the 1024-week
    /// era whose week is nearest `reference`.
    ///
    /// Only the low 10 bits of `raw10` are used (the navigation message field is
    /// 10 bits); any higher bits are reduced modulo 1024. `reference` only needs
    /// to be within ~9.8 years (half a rollover period) of the true epoch for
    /// the resolution to be correct — e.g. the receiver's build date or a coarse
    /// current time. A non-finite / pre-epoch `reference` is treated as era 0.
    pub fn from_broadcast<P: Precision>(raw10: u16, reference: &Epoch<Gps, P>) -> Self {
        let raw = (raw10 as i64) % BROADCAST_WEEK_MODULUS;
        let r = reference
            .to_week_seconds()
            .map(|(w, _)| w.get() as i64)
            .unwrap_or(0);
        // Pick the era (multiple of 1024) that lands W ≡ raw nearest to r.
        let era = ((r - raw) as f64 / BROADCAST_WEEK_MODULUS as f64).round() as i64;
        let w = raw + era * BROADCAST_WEEK_MODULUS;
        Self(w.max(0) as u32)
    }
}

/// Seconds elapsed since the start of the GPS week (Sunday 00:00:00), in the
/// half-open range `[0, 604800)`. GPS Time has no leap seconds, so this is
/// uniform within a week.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct SecondsOfWeek(f64);

impl SecondsOfWeek {
    /// Range-checked constructor. Returns `None` unless `sow ∈ [0, 604800)`
    /// (matching the `Option`-returning style of [`Epoch::<Utc>::from_iso8601`]:
    /// the caller decides how to handle out-of-range telemetry).
    pub fn new(sow: f64) -> Option<Self> {
        // Range `contains` also rejects NaN (no membership), unlike `<`/`>=`.
        (0.0..SECONDS_PER_WEEK).contains(&sow).then_some(Self(sow))
    }

    /// The seconds-of-week value.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Epoch<Gps> {
    /// Construct a GPS epoch from a continuous week and seconds-of-week.
    pub fn from_week_seconds(week: GpsWeek, sow: SecondsOfWeek) -> Self {
        Self::from_jd_gps(GPS_EPOCH_JD + week.get() as f64 * 7.0 + sow.get() / 86400.0)
    }
}

impl<P: Precision> Epoch<Gps, P> {
    /// Decompose into `(continuous week, seconds-of-week)`.
    ///
    /// Returns `None` unless this is a valid GPS instant — finite and at or
    /// after the GPS epoch. On that domain it is the inverse of
    /// [`from_week_seconds`](Self::from_week_seconds), up to the f64 JD precision
    /// floor (~tens of µs at modern epochs). Returning `Option` keeps the
    /// `SecondsOfWeek` `[0, 604800)` invariant intact for non-finite `jd()`
    /// (NaN/±∞), which would otherwise slip past the float guards below.
    pub fn to_week_seconds(&self) -> Option<(GpsWeek, SecondsOfWeek)> {
        let days = self.jd() - GPS_EPOCH_JD;
        // Reject non-finite (NaN/±∞) and pre-epoch instants — both lie outside
        // from_week_seconds's range and would escape the float guards below.
        if !days.is_finite() || days < 0.0 {
            return None;
        }
        let mut week = (days / 7.0).floor();
        let mut sow = (days - week * 7.0) * 86400.0;
        // days − week·7 ∈ [0, 7) analytically; guard the SecondsOfWeek
        // invariant ([0, 604800)) against f64 rounding at the week boundary.
        if sow < 0.0 {
            sow = 0.0;
        } else if sow >= SECONDS_PER_WEEK {
            week += 1.0;
            sow = 0.0;
        }
        Some((GpsWeek(week as u32), SecondsOfWeek(sow)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Epoch, GPS_EPOCH_JD, Gps, GpsWeek, SecondsOfWeek, Utc};

    #[test]
    fn gps_epoch_jd_matches_1980_01_06() {
        // GPS epoch is 1980-01-06 00:00:00 UTC; GPS = UTC at that instant.
        let utc = Epoch::<Utc>::from_gregorian(1980, 1, 6, 0, 0, 0.0);
        assert_eq!(utc.jd().to_bits(), GPS_EPOCH_JD.to_bits());
    }

    #[test]
    fn seconds_of_week_range_check() {
        assert!(SecondsOfWeek::new(0.0).is_some());
        assert!(SecondsOfWeek::new(604_799.999).is_some());
        assert!(SecondsOfWeek::new(604_800.0).is_none()); // exclusive upper bound
        assert!(SecondsOfWeek::new(-1.0).is_none());
        assert!(SecondsOfWeek::new(f64::NAN).is_none());
    }

    #[test]
    fn week_seconds_at_gps_epoch() {
        let e = Epoch::<Gps>::from_week_seconds(GpsWeek::new(0), SecondsOfWeek::new(0.0).unwrap());
        assert_eq!(e.jd().to_bits(), GPS_EPOCH_JD.to_bits());
    }

    #[test]
    fn week_seconds_roundtrip() {
        // Wednesday 12:00 of week 1303 (an arbitrary in-week instant).
        let week = GpsWeek::new(1303);
        let sow = SecondsOfWeek::new(3.5 * 86400.0).unwrap();
        let (w2, s2) = Epoch::<Gps>::from_week_seconds(week, sow)
            .to_week_seconds()
            .unwrap();
        assert_eq!(w2, week);
        assert!(
            (s2.get() - sow.get()).abs() < 1e-3,
            "sow roundtrip: expected {}, got {}",
            sow.get(),
            s2.get()
        );
    }

    #[test]
    fn week_seconds_matches_known_utc_date() {
        // 2024-03-20 12:00:00 UTC. GPS − UTC = 18 s in 2024, so the GPS instant
        // is 18 s later; decomposed week/sow must point back to that day.
        let utc = Epoch::<Utc>::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let (week, sow) = utc.to_gps().to_week_seconds().unwrap();
        // 2024-03-20 is a Wednesday → day-of-week index 3 (Sun=0).
        let dow = (sow.get() / 86400.0).floor() as u32;
        assert_eq!(dow, 3, "expected Wednesday (index 3), got {dow}");
        // Reconstructing the GPS epoch must recover the same UTC instant.
        let utc2 = Epoch::<Gps>::from_week_seconds(week, sow).to_utc();
        assert!((utc2.jd() - utc.jd()).abs() < 1e-8);
    }

    #[test]
    fn to_week_seconds_rejects_non_finite_and_pre_epoch() {
        // Non-finite GPS JDs must not yield a SecondsOfWeek(NaN) — they map to None.
        assert!(
            Epoch::<Gps>::from_jd_gps(f64::NAN)
                .to_week_seconds()
                .is_none()
        );
        assert!(
            Epoch::<Gps>::from_jd_gps(f64::INFINITY)
                .to_week_seconds()
                .is_none()
        );
        assert!(
            Epoch::<Gps>::from_jd_gps(f64::NEG_INFINITY)
                .to_week_seconds()
                .is_none()
        );
        // Before the GPS epoch is outside from_week_seconds's range → None.
        assert!(
            Epoch::<Gps>::from_jd_gps(GPS_EPOCH_JD - 1.0)
                .to_week_seconds()
                .is_none()
        );
        // The epoch instant itself is valid (week 0, sow 0).
        let (w, s) = Epoch::<Gps>::from_jd_gps(GPS_EPOCH_JD)
            .to_week_seconds()
            .unwrap();
        assert_eq!(w, GpsWeek::new(0));
        assert_eq!(s.get(), 0.0);
    }

    #[test]
    fn from_broadcast_resolves_rollover_era() {
        // A continuous week 2200 (year ~2022) has broadcast value 2200 − 2*1024
        // = 152. Resolving 152 against a 2022-ish reference must recover 2200.
        let reference =
            Epoch::<Gps>::from_week_seconds(GpsWeek::new(2190), SecondsOfWeek::new(0.0).unwrap());
        let resolved = GpsWeek::from_broadcast(152, &reference);
        assert_eq!(resolved, GpsWeek::new(2200));
    }

    #[test]
    fn from_broadcast_same_value_resolves_per_reference_era() {
        // The same broadcast value 100 must resolve to different continuous
        // weeks depending on which era the reference points at.
        let near_100 =
            Epoch::<Gps>::from_week_seconds(GpsWeek::new(90), SecondsOfWeek::new(0.0).unwrap());
        let near_1124 =
            Epoch::<Gps>::from_week_seconds(GpsWeek::new(1130), SecondsOfWeek::new(0.0).unwrap());
        assert_eq!(GpsWeek::from_broadcast(100, &near_100), GpsWeek::new(100));
        assert_eq!(GpsWeek::from_broadcast(100, &near_1124), GpsWeek::new(1124));
    }
}
