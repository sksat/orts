//! IERS leap second table and the `TAI - UTC` lookup.

/// IERS leap second table: (MJD_start, TAI - UTC [s]).
///
/// Each entry is the MJD of the UTC day when the cumulative TAI-UTC offset became
/// the listed value. Updates are announced ~6 months ahead by IERS Bulletin C.
/// As of 2024 the last leap second was introduced at 2017-01-01 (TAI-UTC = 37 s).
pub(super) const LEAP_SECONDS: &[(f64, f64)] = &[
    (41317.0, 10.0), // 1972-01-01
    (41499.0, 11.0), // 1972-07-01
    (41683.0, 12.0), // 1973-01-01
    (42048.0, 13.0), // 1974-01-01
    (42413.0, 14.0), // 1975-01-01
    (42778.0, 15.0), // 1976-01-01
    (43144.0, 16.0), // 1977-01-01
    (43509.0, 17.0), // 1978-01-01
    (43874.0, 18.0), // 1979-01-01
    (44239.0, 19.0), // 1980-01-01
    (44786.0, 20.0), // 1981-07-01
    (45151.0, 21.0), // 1982-07-01
    (45516.0, 22.0), // 1983-07-01
    (46247.0, 23.0), // 1985-07-01
    (47161.0, 24.0), // 1988-01-01
    (47892.0, 25.0), // 1990-01-01
    (48257.0, 26.0), // 1991-01-01
    (48804.0, 27.0), // 1992-07-01
    (49169.0, 28.0), // 1993-07-01
    (49534.0, 29.0), // 1994-07-01
    (50083.0, 30.0), // 1996-01-01
    (50630.0, 31.0), // 1997-07-01
    (51179.0, 32.0), // 1999-01-01
    (53736.0, 33.0), // 2006-01-01
    (54832.0, 34.0), // 2009-01-01
    (56109.0, 35.0), // 2012-07-01
    (57204.0, 36.0), // 2015-07-01
    (57754.0, 37.0), // 2017-01-01
];

/// TAI - UTC [seconds] at the given UTC MJD.
///
/// Before 1972-01-01 (MJD 41317) returns 10.0 (the value at the introduction of
/// the modern leap-second regime). After the last table entry returns the final
/// listed offset (currently 37.0).
///
/// # Pre-1972 limitation
///
/// Pre-1972 UTC used a different definition based on "rubber seconds" and
/// stepped frequency offsets, with ~50 distinct entries from 1960 to 1971.
/// The actual TAI − UTC offset during that era varied from ~1.4 s (1961) to
/// ~9.9 s (late 1971), NOT a constant 10.0 s. For example, Apollo 11 epoch
/// (1969-07-20) had TAI − UTC ≈ 8.0 s, so this function over-estimates by
/// about 2 s for that date.
///
/// At lunar distances (Moon velocity ~1 km/s), a 2 s time scale error
/// translates to ~2 km Meeus ephemeris offset. This is a strict improvement
/// over the pre-refactor behavior which did NOT convert UTC → TDB at all
/// (yielding a ~40 s / ~40 km error at Apollo epochs), but it is not
/// perfectly correct for pre-1972 dates.
///
/// A full pre-1972 UTC rate-offset table is deferred to a later phase of the
/// arika redesign. Callers requiring bit-accurate pre-1972 ephemerides
/// should use [`Epoch::<Tdb>::from_jd_tdb`](super::Epoch::from_jd_tdb) directly
/// with an externally computed TDB Julian Date.
pub(crate) fn tai_minus_utc_at_mjd(utc_mjd: f64) -> f64 {
    let mut offset = 10.0;
    for &(mjd_start, val) in LEAP_SECONDS {
        if utc_mjd >= mjd_start {
            offset = val;
        } else {
            break;
        }
    }
    offset
}
