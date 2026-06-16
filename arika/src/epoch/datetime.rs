//! Gregorian [`DateTime`] (UTC) and Julian Date → calendar conversion.

#[allow(unused_imports)]
use crate::math::F64Ext;

/// A Gregorian calendar date and time (UTC).
///
/// 本 struct は UTC 暦表示専用。TAI / TT / TDB 等の dynamical time scale は
/// 直接 Gregorian で表現しない (`Epoch<Utc>` に変換してから `to_datetime` を使う)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: f64,
}

impl DateTime {
    pub fn new(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: f64) -> Self {
        DateTime {
            year,
            month,
            day,
            hour,
            min,
            sec,
        }
    }
}

impl core::fmt::Display for DateTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Round to integer seconds and normalize overflow (e.g. sec=59.999... → 60)
        let sec = self.sec.round() as u32;
        let (sec, carry) = if sec >= 60 { (0u32, 1u32) } else { (sec, 0) };
        let min = self.min + carry;
        let (min, carry) = if min >= 60 {
            (min - 60, 1u32)
        } else {
            (min, 0)
        };
        let hour = self.hour + carry;
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, hour, min, sec
        )
    }
}

/// Convert a Julian Date value to Gregorian calendar date and time.
/// Shared by `Epoch<Utc>::to_datetime` — kept at module scope so it can be
/// reused by future scale-specific display helpers.
pub(super) fn to_datetime_from_jd(jd: f64) -> DateTime {
    // Meeus, "Astronomical Algorithms", Chapter 7
    let jd = jd + 0.5;
    let z = jd.floor() as i64;
    let f = jd - z as f64;

    let a = if z < 2299161 {
        z
    } else {
        let alpha = ((z as f64 - 1867216.25) / 36524.25).floor() as i64;
        z + 1 + alpha - alpha / 4
    };

    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25).floor() as i64;
    let d = (365.25 * c as f64).floor() as i64;
    let e = ((b - d) as f64 / 30.6001).floor() as i64;

    let day = (b - d - (30.6001 * e as f64).floor() as i64) as u32;
    let month = if e < 14 { e - 1 } else { e - 13 } as u32;
    let year = if month > 2 { c - 4716 } else { c - 4715 } as i32;

    let hours_total = f * 24.0;
    let hour = hours_total.floor() as u32;
    let mins_total = (hours_total - hour as f64) * 60.0;
    let min = mins_total.floor() as u32;
    let sec = (mins_total - min as f64) * 60.0;

    DateTime {
        year,
        month,
        day,
        hour,
        min,
        sec,
    }
}
