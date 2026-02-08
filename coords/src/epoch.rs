use std::f64::consts::TAU;

/// Julian Date of J2000.0 epoch (2000-01-01 12:00:00 TT).
pub const J2000_JD: f64 = 2451545.0;

/// Offset between Julian Date and Modified Julian Date.
const MJD_OFFSET: f64 = 2400000.5;

/// Julian century in days.
const JULIAN_CENTURY: f64 = 36525.0;

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
    ///
    /// Returns `(year, month, day, hour, minute, second)`.
    pub fn to_gregorian(&self) -> (i32, u32, u32, u32, u32, f64) {
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

        (year, month, day, hour, min, sec)
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
        let (year, month, day, hour, min, sec) = original.to_gregorian();
        assert_eq!(year, 2024);
        assert_eq!(month, 6);
        assert_eq!(day, 21);
        assert_eq!(hour, 15);
        assert_eq!(min, 30);
        assert!((sec - 45.0).abs() < 0.01, "sec: expected 45.0, got {sec}");
    }

    #[test]
    fn gregorian_roundtrip_january() {
        // Jan and Feb use different month adjustment in JD algorithm
        let original = Epoch::from_gregorian(2024, 1, 15, 0, 0, 0.0);
        let (year, month, day, hour, min, _sec) = original.to_gregorian();
        assert_eq!(year, 2024);
        assert_eq!(month, 1);
        assert_eq!(day, 15);
        assert_eq!(hour, 0);
        assert_eq!(min, 0);
    }

    #[test]
    fn gregorian_roundtrip_february() {
        let original = Epoch::from_gregorian(2024, 2, 29, 6, 0, 0.0);
        let (year, month, day, hour, _, _) = original.to_gregorian();
        assert_eq!(year, 2024);
        assert_eq!(month, 2);
        assert_eq!(day, 29);
        assert_eq!(hour, 6);
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

    #[test]
    fn gmst_works_with_eci_ecef() {
        // Verify that Epoch::gmst() produces valid angles for ECI↔ECEF conversion
        use crate::Eci;
        let epoch = Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0);
        let gmst = epoch.gmst();

        let eci = Eci(nalgebra::Vector3::new(7000.0, 1000.0, 500.0));
        let ecef = eci.to_ecef(gmst);
        let roundtrip = ecef.to_eci(gmst);

        let eps = 1e-10;
        assert!((roundtrip.0.x - eci.0.x).abs() < eps);
        assert!((roundtrip.0.y - eci.0.y).abs() < eps);
        assert!((roundtrip.0.z - eci.0.z).abs() < eps);
    }
}
