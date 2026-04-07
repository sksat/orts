use std::f64::consts::TAU;

/// Julian Date of J2000.0 epoch (2000-01-01 12:00:00 TT).
pub const J2000_JD: f64 = 2451545.0;

/// Offset between Julian Date and Modified Julian Date.
const MJD_OFFSET: f64 = 2400000.5;

/// Julian century in days.
const JULIAN_CENTURY: f64 = 36525.0;

/// A Gregorian calendar date and time (UTC).
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

impl std::fmt::Display for DateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// An astronomical epoch represented as Julian Date (JD).
///
/// Provides conversions between JD, MJD, Gregorian calendar, and ISO 8601,
/// as well as derived quantities like GMST.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Epoch {
    jd: f64,
}

impl Epoch {
    /// Create an epoch from a Julian Date value.
    pub fn from_jd(jd: f64) -> Self {
        Epoch { jd }
    }

    /// Create an epoch from a Modified Julian Date value.
    pub fn from_mjd(mjd: f64) -> Self {
        Epoch {
            jd: mjd + MJD_OFFSET,
        }
    }

    /// The J2000.0 epoch (2000-01-01 12:00:00 TT).
    pub fn j2000() -> Self {
        Epoch { jd: J2000_JD }
    }

    /// Create an epoch from a [`DateTime`] value.
    pub fn from_datetime(dt: &DateTime) -> Self {
        Self::from_gregorian(dt.year, dt.month, dt.day, dt.hour, dt.min, dt.sec)
    }

    /// Create an epoch from Gregorian calendar date and time (UTC).
    ///
    /// Uses the standard Julian Date algorithm valid for dates after
    /// the Gregorian calendar reform (1582-10-15).
    pub fn from_gregorian(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: f64) -> Self {
        // Adjust year and month for the algorithm (Jan/Feb are months 13/14 of prev year)
        let (y, m) = if month <= 2 {
            (year - 1, month + 12)
        } else {
            (year, month)
        };

        let a = y / 100;
        let b = 2 - a + a / 4;

        let jd = (365.25 * (y + 4716) as f64).floor()
            + (30.6001 * (m + 1) as f64).floor()
            + day as f64
            + b as f64
            - 1524.5
            + (hour as f64 + min as f64 / 60.0 + sec / 3600.0) / 24.0;

        Epoch { jd }
    }

    /// Parse an epoch from ISO 8601 format: `YYYY-MM-DDTHH:MM:SSZ`.
    ///
    /// Only UTC (Z suffix) is supported. Returns `None` if parsing fails.
    pub fn from_iso8601(s: &str) -> Option<Self> {
        let s = s.trim();
        if !s.ends_with('Z') {
            return None;
        }
        let s = &s[..s.len() - 1]; // strip 'Z'
        let parts: Vec<&str> = s.split('T').collect();
        if parts.len() != 2 {
            return None;
        }

        let date_parts: Vec<&str> = parts[0].split('-').collect();
        if date_parts.len() != 3 {
            return None;
        }
        let year: i32 = date_parts[0].parse().ok()?;
        let month: u32 = date_parts[1].parse().ok()?;
        let day: u32 = date_parts[2].parse().ok()?;

        let time_parts: Vec<&str> = parts[1].split(':').collect();
        if time_parts.len() != 3 {
            return None;
        }
        let hour: u32 = time_parts[0].parse().ok()?;
        let min: u32 = time_parts[1].parse().ok()?;
        let sec: f64 = time_parts[2].parse().ok()?;

        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || min > 59
            || sec >= 60.0
        {
            return None;
        }

        Some(Self::from_gregorian(year, month, day, hour, min, sec))
    }

    /// Create an epoch from the current system time (UTC).
    pub fn now() -> Self {
        // Unix epoch (1970-01-01 00:00:00 UTC) = JD 2440587.5
        const UNIX_EPOCH_JD: f64 = 2440587.5;
        let unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_secs_f64();
        Epoch {
            jd: UNIX_EPOCH_JD + unix_secs / 86400.0,
        }
    }

    /// Return the Julian Date value.
    pub fn jd(&self) -> f64 {
        self.jd
    }

    /// Return the Modified Julian Date value.
    pub fn mjd(&self) -> f64 {
        self.jd - MJD_OFFSET
    }

    /// Return Julian centuries since J2000.0.
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd - J2000_JD) / JULIAN_CENTURY
    }

    /// Create a new epoch advanced by `dt` seconds.
    pub fn add_seconds(&self, dt: f64) -> Self {
        Epoch {
            jd: self.jd + dt / 86400.0,
        }
    }

    /// Convert to Gregorian calendar date and time (UTC).
    pub fn to_datetime(&self) -> DateTime {
        // Meeus, "Astronomical Algorithms", Chapter 7
        let jd = self.jd + 0.5;
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

    /// Create an epoch from a TLE epoch (2-digit year + fractional day of year).
    ///
    /// 2-digit year convention (NORAD): 57-99 → 1957-1999, 00-56 → 2000-2056.
    pub fn from_tle_epoch(year_2digit: u32, day_of_year: f64) -> Self {
        let year = if year_2digit >= 57 {
            1900 + year_2digit as i32
        } else {
            2000 + year_2digit as i32
        };
        // JD of January 0.0 of that year = JD of Dec 31 of previous year at 0h
        let jan1 = Self::from_gregorian(year, 1, 1, 0, 0, 0.0);
        // day_of_year: 1.0 = Jan 1 00:00, 1.5 = Jan 1 12:00, etc.
        Epoch {
            jd: jan1.jd + (day_of_year - 1.0),
        }
    }

    /// Greenwich Mean Sidereal Time (GMST) in radians.
    ///
    /// Uses the Earth Rotation Angle (ERA) formula from IERS 2003.
    /// Assumes UT1 ≈ UTC (sufficient for visualization purposes).
    pub fn gmst(&self) -> f64 {
        let du = self.jd - J2000_JD;
        // Earth Rotation Angle (IERS 2003)
        let era = TAU * (0.7790572732640 + 1.002_737_811_911_354_6 * du);
        // Normalize to [0, 2π)
        let gmst = era % TAU;
        if gmst < 0.0 { gmst + TAU } else { gmst }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // --- Epoch construction and accessors ---

    #[test]
    fn j2000_constant() {
        let epoch = Epoch::j2000();
        assert_eq!(epoch.jd(), J2000_JD);
        assert_eq!(epoch.jd(), 2451545.0);
    }

    #[test]
    fn from_jd_roundtrip() {
        let jd = 2460389.0;
        let epoch = Epoch::from_jd(jd);
        assert_eq!(epoch.jd(), jd);
    }

    #[test]
    fn mjd_roundtrip() {
        let mjd = 60388.5;
        let epoch = Epoch::from_mjd(mjd);
        assert!((epoch.mjd() - mjd).abs() < 1e-12);
    }

    #[test]
    fn mjd_jd_relationship() {
        let epoch = Epoch::from_jd(2451545.0);
        assert!((epoch.mjd() - 51544.5).abs() < 1e-12);
    }

    // --- Gregorian conversions ---

    #[test]
    fn j2000_gregorian() {
        // J2000.0 = 2000-01-01 12:00:00
        let epoch = Epoch::from_gregorian(2000, 1, 1, 12, 0, 0.0);
        assert!(
            (epoch.jd() - J2000_JD).abs() < 1e-6,
            "J2000 JD: expected {}, got {}",
            J2000_JD,
            epoch.jd()
        );
    }

    #[test]
    fn known_date_2024_march_equinox() {
        // 2024-03-20 12:00:00 UTC
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let expected_jd = 2460390.0;
        assert!(
            (epoch.jd() - expected_jd).abs() < 0.01,
            "2024-03-20 JD: expected ~{}, got {}",
            expected_jd,
            epoch.jd()
        );
    }

    #[test]
    fn gregorian_roundtrip() {
        let original = Epoch::from_gregorian(2024, 6, 21, 15, 30, 45.0);
        let dt = original.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 6);
        assert_eq!(dt.day, 21);
        assert_eq!(dt.hour, 15);
        assert_eq!(dt.min, 30);
        assert!(
            (dt.sec - 45.0).abs() < 0.01,
            "sec: expected 45.0, got {}",
            dt.sec
        );
    }

    #[test]
    fn gregorian_roundtrip_january() {
        // Jan and Feb use different month adjustment in JD algorithm
        let original = Epoch::from_gregorian(2024, 1, 15, 0, 0, 0.0);
        let dt = original.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 15);
        assert_eq!(dt.hour, 0);
        assert_eq!(dt.min, 0);
    }

    #[test]
    fn gregorian_roundtrip_february() {
        let original = Epoch::from_gregorian(2024, 2, 29, 6, 0, 0.0);
        let dt = original.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 2);
        assert_eq!(dt.day, 29);
        assert_eq!(dt.hour, 6);
    }

    #[test]
    fn datetime_display() {
        let dt = DateTime::new(2024, 3, 20, 12, 0, 0.0);
        assert_eq!(dt.to_string(), "2024-03-20T12:00:00Z");
    }

    #[test]
    fn from_datetime_roundtrip() {
        let dt = DateTime::new(2024, 6, 21, 15, 30, 45.0);
        let epoch = Epoch::from_datetime(&dt);
        let rt = epoch.to_datetime();
        assert_eq!(rt.year, dt.year);
        assert_eq!(rt.month, dt.month);
        assert_eq!(rt.day, dt.day);
        assert_eq!(rt.hour, dt.hour);
        assert_eq!(rt.min, dt.min);
        assert!((rt.sec - dt.sec).abs() < 0.01);
    }

    // --- add_seconds ---

    #[test]
    fn add_seconds_one_day() {
        let epoch = Epoch::j2000();
        let next_day = epoch.add_seconds(86400.0);
        assert!(
            (next_day.jd() - (J2000_JD + 1.0)).abs() < 1e-12,
            "add 86400s: expected JD {}, got {}",
            J2000_JD + 1.0,
            next_day.jd()
        );
    }

    #[test]
    fn now_returns_reasonable_jd() {
        let epoch = Epoch::now();
        // JD for 2025-01-01 ≈ 2460676, for 2030-01-01 ≈ 2462502
        // Any reasonable current date should be in this range
        assert!(
            epoch.jd() > 2460676.0 && epoch.jd() < 2462502.0,
            "Epoch::now() JD {} is outside 2025–2030 range",
            epoch.jd()
        );
        // Verify to_datetime year is plausible
        let dt = epoch.to_datetime();
        assert!(
            dt.year >= 2025 && dt.year <= 2030,
            "Epoch::now() year {} is outside expected range",
            dt.year
        );
    }

    #[test]
    fn add_seconds_one_hour() {
        let epoch = Epoch::j2000();
        let plus_hour = epoch.add_seconds(3600.0);
        let expected = J2000_JD + 1.0 / 24.0;
        assert!((plus_hour.jd() - expected).abs() < 1e-12);
    }

    #[test]
    fn centuries_since_j2000() {
        let epoch = Epoch::j2000();
        assert!((epoch.centuries_since_j2000() - 0.0).abs() < 1e-15);

        // One Julian century later
        let later = Epoch::from_jd(J2000_JD + JULIAN_CENTURY);
        assert!((later.centuries_since_j2000() - 1.0).abs() < 1e-12);
    }

    // --- ISO 8601 parsing ---

    #[test]
    fn iso8601_valid() {
        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let expected = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        assert!(
            (epoch.jd() - expected.jd()).abs() < 1e-10,
            "ISO parse mismatch"
        );
    }

    #[test]
    fn iso8601_with_seconds() {
        let epoch = Epoch::from_iso8601("2000-01-01T12:00:00Z").unwrap();
        assert!((epoch.jd() - J2000_JD).abs() < 1e-6);
    }

    #[test]
    fn iso8601_invalid_no_z() {
        assert!(Epoch::from_iso8601("2024-03-20T12:00:00").is_none());
    }

    #[test]
    fn iso8601_invalid_format() {
        assert!(Epoch::from_iso8601("not-a-date").is_none());
        assert!(Epoch::from_iso8601("2024-13-01T00:00:00Z").is_none()); // month 13
        assert!(Epoch::from_iso8601("2024-01-32T00:00:00Z").is_none()); // day 32
    }

    // --- GMST ---

    #[test]
    fn gmst_at_j2000() {
        let epoch = Epoch::j2000();
        let gmst = epoch.gmst();
        // At J2000.0, GMST ≈ 280.46° = 4.8949 rad
        // ERA at Du=0: 2π × 0.7790572732640 ≈ 4.8949 rad
        let expected = TAU * 0.7790572732640;
        assert!(
            (gmst - expected).abs() < 0.01,
            "GMST at J2000: expected {:.4} rad, got {:.4} rad",
            expected,
            gmst
        );
    }

    #[test]
    fn gmst_increases_one_sidereal_day() {
        // One sidereal day ≈ 86164.0905 seconds
        // After one solar day (86400s), GMST should increase by ~360.9856° ≈ ~2π + 0.0172 rad
        let epoch = Epoch::j2000();
        let gmst0 = epoch.gmst();
        let next_day = epoch.add_seconds(86400.0);
        let gmst1 = next_day.gmst();

        // The GMST difference over one solar day should be close to
        // 1.002_737_811_911_354_6 × 2π (one sidereal rotation plus the extra ~3.94 min)
        let delta = if gmst1 > gmst0 {
            gmst1 - gmst0
        } else {
            gmst1 + TAU - gmst0
        };
        let expected_delta = TAU * 1.002_737_811_911_354_6;
        let expected_delta_mod = expected_delta % TAU;
        assert!(
            (delta - expected_delta_mod).abs() < 0.001,
            "GMST daily increase: expected {:.6} rad, got {:.6} rad",
            expected_delta_mod,
            delta
        );
    }

    #[test]
    fn gmst_normalized() {
        // GMST should always be in [0, 2π)
        for days in [0.0, 0.5, 1.0, 100.0, 365.25, 3652.5] {
            let epoch = Epoch::j2000().add_seconds(days * 86400.0);
            let gmst = epoch.gmst();
            assert!(
                gmst >= 0.0 && gmst < TAU,
                "GMST at +{days} days: {gmst} not in [0, 2π)"
            );
        }
    }

    // --- TLE epoch ---

    #[test]
    fn tle_epoch_iss_2024() {
        // ISS TLE epoch: 24079.50000000 → 2024 day 79.5 → 2024-03-19 12:00:00 UTC
        let epoch = Epoch::from_tle_epoch(24, 79.5);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 3);
        assert_eq!(dt.day, 19);
        assert_eq!(dt.hour, 12);
    }

    #[test]
    fn tle_epoch_year_2000() {
        // Year 00 → 2000, day 1.0 → 2000-01-01 00:00:00
        let epoch = Epoch::from_tle_epoch(0, 1.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2000);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.hour, 0);
    }

    #[test]
    fn tle_epoch_year_1999() {
        // Year 99 → 1999, day 365.0 → 1999-12-31 00:00:00
        let epoch = Epoch::from_tle_epoch(99, 365.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 1999);
        assert_eq!(dt.month, 12);
        assert_eq!(dt.day, 31);
    }

    #[test]
    fn tle_epoch_year_57() {
        // Year 57 → 1957 (Sputnik era)
        let epoch = Epoch::from_tle_epoch(57, 1.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 1957);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
    }

    #[test]
    fn tle_epoch_year_56() {
        // Year 56 → 2056
        let epoch = Epoch::from_tle_epoch(56, 1.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2056);
    }

    #[test]
    fn tle_epoch_matches_iso8601() {
        // TLE epoch 24001.50000000 → 2024-01-01 12:00:00 UTC
        let tle_epoch = Epoch::from_tle_epoch(24, 1.5);
        let iso_epoch = Epoch::from_iso8601("2024-01-01T12:00:00Z").unwrap();
        assert!(
            (tle_epoch.jd() - iso_epoch.jd()).abs() < 1e-6,
            "TLE epoch {} vs ISO epoch {}",
            tle_epoch.jd(),
            iso_epoch.jd()
        );
    }

    // --- JD → UTC string end-to-end (mirrors deleted TS astro.test.ts) ---

    #[test]
    fn jd_to_utc_string_j2000() {
        let s = Epoch::from_jd(J2000_JD).to_datetime().to_string();
        assert_eq!(s, "2000-01-01T12:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_2024_march() {
        let s = Epoch::from_jd(2460390.0).to_datetime().to_string();
        assert_eq!(s, "2024-03-20T12:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_with_offset_1h() {
        // J2000 + 3600s = 2000-01-01T13:00:00Z
        let s = Epoch::from_jd(J2000_JD)
            .add_seconds(3600.0)
            .to_datetime()
            .to_string();
        assert_eq!(s, "2000-01-01T13:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_with_offset_1day() {
        // J2000 + 86400s = 2000-01-02T12:00:00Z
        let s = Epoch::from_jd(J2000_JD)
            .add_seconds(86400.0)
            .to_datetime()
            .to_string();
        assert_eq!(s, "2000-01-02T12:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_no_fractional_seconds() {
        // Fractional seconds should be truncated (format uses {:02.0})
        let s = Epoch::from_jd(J2000_JD)
            .add_seconds(0.5)
            .to_datetime()
            .to_string();
        assert!(
            s.ends_with("Z") && !s.contains('.'),
            "Should not contain fractional seconds: {s}"
        );
    }

    #[test]
    fn gmst_works_with_eci_ecef() {
        // Verify that Epoch::gmst() produces valid angles for ECI↔ECEF conversion
        use crate::Eci;
        let epoch = Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0);
        let gmst = epoch.gmst();

        let eci = Eci::new(7000.0, 1000.0, 500.0);
        let ecef = eci.to_ecef(gmst);
        let roundtrip = ecef.to_eci(gmst);

        let eps = 1e-10;
        assert!((roundtrip.x() - eci.x()).abs() < eps);
        assert!((roundtrip.y() - eci.y()).abs() < eps);
        assert!((roundtrip.z() - eci.z()).abs() < eps);
    }
}
