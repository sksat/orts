//! Meeus analytical lunar ephemeris and `MoonEphemeris` trait abstraction.
//!
//! Implements the analytical model from Meeus "Astronomical Algorithms"
//! Chapter 47 with full periodic term tables, plus a trait abstraction that
//! lets callers swap between Meeus and JPL-Horizons-backed ephemerides.

use crate::epoch::Epoch;
use crate::frame::{self, Vec3};

/// Moon position vector in ECI (J2000) frame [km].
///
/// Uses the analytical model from Meeus "Astronomical Algorithms" Chapter 47
/// with full periodic term tables (60 terms for longitude/distance, 60 for latitude),
/// E correction factor, and planetary perturbation corrections.
///
/// Accuracy: ~10" in ecliptic longitude, ~1% in distance (~4,000 km).
///
/// # Time scale
///
/// Meeus ephemerides take a dynamical time argument (TDB). The public signature
/// accepts `&Epoch<Utc>` (the default alias) for backward compatibility; the
/// UTC epoch is converted to TDB internally via leap seconds + TT offset +
/// Fairhead-Bretagnon periodic correction.
pub fn moon_position_eci(epoch: &Epoch) -> Vec3<frame::Gcrs> {
    let t = epoch.to_tdb().centuries_since_j2000();
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;

    // Fundamental arguments (degrees), full polynomial
    let lp = 218.3164477 + 481267.88123421 * t - 0.0015786 * t2 + t3 / 538841.0 - t4 / 65194000.0;
    let d = 297.8501921 + 445267.1114034 * t - 0.0018819 * t2 + t3 / 545868.0 - t4 / 113065000.0;
    let m = 357.5291092 + 35999.0502909 * t - 0.0001535 * t2 + t3 / 24490000.0;
    let mp = 134.9633964 + 477198.8675055 * t + 0.0087414 * t2 + t3 / 69699.0 - t4 / 14712000.0;
    let f = 93.2720950 + 483202.0175233 * t - 0.0036539 * t2 - t3 / 3526000.0 + t4 / 863310000.0;

    // Additional arguments for planetary corrections
    let a1 = (119.75 + 131.849 * t).to_radians();
    let a2 = (53.09 + 479264.290 * t).to_radians();
    let a3 = (313.45 + 481266.484 * t).to_radians();

    let d_r = d.to_radians();
    let m_r = m.to_radians();
    let mp_r = mp.to_radians();
    let f_r = f.to_radians();
    let lp_r = lp.to_radians();

    // E correction for solar terms
    let e = 1.0 - 0.002516 * t - 0.0000074 * t2;
    let e2 = e * e;

    // Table 47.A: periodic terms for longitude (Σl) and distance (Σr)
    // Each row: (D, M, M', F, Σl, Σr)
    // Σl in units of 0.000001 degrees (multiply sin(arg))
    // Σr in units of 0.001 km (multiply cos(arg))
    #[rustfmt::skip]
    const TABLE_A: [(i32, i32, i32, i32, f64, f64); 60] = [
        ( 0,  0,  1,  0,  6288774.0, -20905355.0),
        ( 2,  0, -1,  0,  1274027.0,  -3699111.0),
        ( 2,  0,  0,  0,   658314.0,  -2955968.0),
        ( 0,  0,  2,  0,   213618.0,   -569925.0),
        ( 0,  1,  0,  0,  -185116.0,     48888.0),
        ( 0,  0,  0,  2,  -114332.0,     -3149.0),
        ( 2,  0, -2,  0,    58793.0,    246158.0),
        ( 2, -1, -1,  0,    57066.0,   -152138.0),
        ( 2,  0,  1,  0,    53322.0,   -170733.0),
        ( 2, -1,  0,  0,    45758.0,   -204586.0),
        ( 0,  1, -1,  0,   -40923.0,   -129620.0),
        ( 1,  0,  0,  0,   -34720.0,    108743.0),
        ( 0,  1,  1,  0,   -30383.0,    104755.0),
        ( 2,  0,  0, -2,    15327.0,     10321.0),
        ( 0,  0,  1,  2,   -12528.0,         0.0),
        ( 0,  0,  1, -2,    10980.0,     79661.0),
        ( 4,  0, -1,  0,    10675.0,    -34782.0),
        ( 0,  0,  3,  0,    10034.0,    -23210.0),
        ( 4,  0, -2,  0,     8548.0,    -21636.0),
        ( 2,  1, -1,  0,    -7888.0,     24208.0),
        ( 2,  1,  0,  0,    -6766.0,     30824.0),
        ( 1,  0, -1,  0,    -5163.0,     -8379.0),
        ( 1,  1,  0,  0,     4987.0,    -16675.0),
        ( 2, -1,  1,  0,     4036.0,    -12831.0),
        ( 2,  0,  2,  0,     3994.0,    -10445.0),
        ( 4,  0,  0,  0,     3861.0,    -11650.0),
        ( 2,  0, -3,  0,     3665.0,     14403.0),
        ( 0,  1, -2,  0,    -2689.0,     -7003.0),
        ( 2,  0, -1,  2,    -2602.0,         0.0),
        ( 2, -1, -2,  0,     2390.0,     10056.0),
        ( 1,  0,  1,  0,    -2348.0,      6322.0),
        ( 2, -2,  0,  0,     2236.0,     -9884.0),
        ( 0,  1,  2,  0,    -2120.0,      5751.0),
        ( 0,  2,  0,  0,    -2069.0,         0.0),
        ( 2, -2, -1,  0,     2048.0,     -4950.0),
        ( 2,  0,  1, -2,    -1773.0,      4130.0),
        ( 2,  0,  0,  2,    -1595.0,         0.0),
        ( 4, -1, -1,  0,     1215.0,     -3958.0),
        ( 0,  0,  2,  2,    -1110.0,         0.0),
        ( 3,  0, -1,  0,     -892.0,      3258.0),
        ( 2,  1,  1,  0,     -810.0,      2616.0),
        ( 4, -1, -2,  0,      759.0,     -1897.0),
        ( 0,  2, -1,  0,     -713.0,     -2117.0),
        ( 2,  2, -1,  0,     -700.0,      2354.0),
        ( 2,  1, -2,  0,      691.0,         0.0),
        ( 2, -1,  0, -2,      596.0,         0.0),
        ( 4,  0,  1,  0,      549.0,     -1423.0),
        ( 0,  0,  4,  0,      537.0,     -1117.0),
        ( 4, -1,  0,  0,      520.0,     -1571.0),
        ( 1,  0, -2,  0,     -487.0,     -1739.0),
        ( 2,  1,  0, -2,     -399.0,         0.0),
        ( 0,  0,  2, -2,     -381.0,     -4421.0),
        ( 1,  1,  1,  0,      351.0,         0.0),
        ( 3,  0, -2,  0,     -340.0,         0.0),
        ( 4,  0, -3,  0,      330.0,         0.0),
        ( 2, -1,  2,  0,      327.0,         0.0),
        ( 0,  2,  1,  0,     -323.0,      1165.0),
        ( 1,  1, -1,  0,      299.0,         0.0),
        ( 2,  0,  3,  0,      294.0,         0.0),
        ( 2,  0, -1, -2,        0.0,      8752.0),
    ];

    // Table 47.B: periodic terms for latitude (Σb)
    // Each row: (D, M, M', F, Σb)
    // Σb in units of 0.000001 degrees (multiply sin(arg))
    #[rustfmt::skip]
    const TABLE_B: [(i32, i32, i32, i32, f64); 60] = [
        ( 0,  0,  0,  1,  5128122.0),
        ( 0,  0,  1,  1,   280602.0),
        ( 0,  0,  1, -1,   277693.0),
        ( 2,  0,  0, -1,   173237.0),
        ( 2,  0, -1,  1,    55413.0),
        ( 2,  0, -1, -1,    46271.0),
        ( 2,  0,  0,  1,    32573.0),
        ( 0,  0,  2,  1,    17198.0),
        ( 2,  0,  1, -1,     9266.0),
        ( 0,  0,  2, -1,     8822.0),
        ( 2, -1,  0, -1,     8216.0),
        ( 2,  0, -2, -1,     4324.0),
        ( 2,  0,  1,  1,     4200.0),
        ( 2,  1,  0, -1,    -3359.0),
        ( 2, -1, -1,  1,     2463.0),
        ( 2, -1,  0,  1,     2211.0),
        ( 2, -1, -1, -1,     2065.0),
        ( 0,  1, -1, -1,    -1870.0),
        ( 4,  0, -1, -1,     1828.0),
        ( 0,  1,  0,  1,    -1794.0),
        ( 0,  0,  0,  3,    -1749.0),
        ( 0,  1, -1,  1,    -1565.0),
        ( 1,  0,  0,  1,    -1491.0),
        ( 0,  1,  1,  1,    -1475.0),
        ( 0,  1,  1, -1,    -1410.0),
        ( 0,  1,  0, -1,    -1344.0),
        ( 1,  0,  0, -1,    -1335.0),
        ( 0,  0,  3,  1,     1107.0),
        ( 4,  0,  0, -1,     1021.0),
        ( 4,  0, -1,  1,      833.0),
        ( 0,  0,  1, -3,      777.0),
        ( 4,  0, -2,  1,      671.0),
        ( 2,  0,  0, -3,      607.0),
        ( 2,  0,  2, -1,      596.0),
        ( 2, -1,  1, -1,      491.0),
        ( 2,  0, -2,  1,     -451.0),
        ( 0,  0,  3, -1,      439.0),
        ( 2,  0,  2,  1,      422.0),
        ( 2,  0, -3, -1,      421.0),
        ( 2,  1, -1,  1,     -366.0),
        ( 2,  1,  0,  1,     -351.0),
        ( 4,  0,  0,  1,      331.0),
        ( 2, -1,  1,  1,      315.0),
        ( 2, -2,  0, -1,      302.0),
        ( 0,  0,  1,  3,     -283.0),
        ( 2,  1,  1, -1,     -229.0),
        ( 1,  1,  0, -1,      223.0),
        ( 1,  1,  0,  1,      223.0),
        ( 0,  1, -2, -1,     -220.0),
        ( 2,  1, -1, -1,     -220.0),
        ( 1,  0,  1,  1,     -185.0),
        ( 2, -1, -2, -1,      181.0),
        ( 0,  1,  2,  1,     -177.0),
        ( 4,  0, -2, -1,      176.0),
        ( 4, -1, -1, -1,      166.0),
        ( 1,  0,  1, -1,     -164.0),
        ( 4,  0,  1, -1,      132.0),
        ( 1,  0, -1, -1,     -119.0),
        ( 4, -1,  0, -1,      115.0),
        ( 2, -2,  0,  1,      107.0),
    ];

    // Compute Σl, Σr from Table 47.A
    let mut sigma_l: f64 = 0.0;
    let mut sigma_r: f64 = 0.0;
    for &(cd, cm, cmp, cf, sl, sr) in &TABLE_A {
        let arg = cd as f64 * d_r + cm as f64 * m_r + cmp as f64 * mp_r + cf as f64 * f_r;
        let e_factor = match cm.abs() {
            1 => e,
            2 => e2,
            _ => 1.0,
        };
        sigma_l += sl * e_factor * arg.sin();
        sigma_r += sr * e_factor * arg.cos();
    }

    // Compute Σb from Table 47.B
    let mut sigma_b: f64 = 0.0;
    for &(cd, cm, cmp, cf, sb) in &TABLE_B {
        let arg = cd as f64 * d_r + cm as f64 * m_r + cmp as f64 * mp_r + cf as f64 * f_r;
        let e_factor = match cm.abs() {
            1 => e,
            2 => e2,
            _ => 1.0,
        };
        sigma_b += sb * e_factor * arg.sin();
    }

    // Additive corrections
    sigma_l += 3958.0 * a1.sin() + 1962.0 * (lp_r - f_r).sin() + 318.0 * a2.sin();

    sigma_b += -2235.0 * lp_r.sin()
        + 382.0 * a3.sin()
        + 175.0 * (a1 - f_r).sin()
        + 175.0 * (a1 + f_r).sin()
        + 127.0 * (lp_r - mp_r).sin()
        - 115.0 * (lp_r + mp_r).sin();

    // Final coordinates
    let lambda = (lp + sigma_l * 1e-6).to_radians();
    let beta = (sigma_b * 1e-6).to_radians();
    let distance_km = 385000.56 + sigma_r * 0.001;

    // Ecliptic → Equatorial (ECI) conversion
    let epsilon = (23.439291 - 0.0130042 * t).to_radians();

    let cos_lam = lambda.cos();
    let sin_lam = lambda.sin();
    let cos_beta = beta.cos();
    let sin_beta = beta.sin();
    let cos_eps = epsilon.cos();
    let sin_eps = epsilon.sin();

    let x = distance_km * cos_beta * cos_lam;
    let y = distance_km * (cos_eps * cos_beta * sin_lam - sin_eps * sin_beta);
    let z = distance_km * (sin_eps * cos_beta * sin_lam + cos_eps * sin_beta);

    Vec3::new(x, y, z)
}

/// Moon ephemeris source abstraction.
///
/// Implementations provide Moon position (and optionally velocity) in the ECI
/// (J2000-ish) frame for a given epoch. This trait decouples consumers from
/// the specific ephemeris model (Meeus analytical, tabulated JPL Horizons
/// data, SPICE kernel, …), allowing the integrator and targeters to share a
/// single source of truth.
///
/// The default `velocity_eci` implementation uses a central finite difference
/// (±1 second) over `position_eci`. Tabulated sources (e.g. Hermite-interpolated
/// Horizons data) that can provide velocity more accurately should override it.
pub trait MoonEphemeris: Send + Sync {
    /// Moon position in ECI [km] at the given epoch.
    fn position_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs>;

    /// Moon velocity in ECI [km/s] at the given epoch.
    ///
    /// Default: central finite difference over `position_eci` with a 1-second
    /// step. Override for sources that can supply analytic velocity.
    fn velocity_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
        let dt = 1.0;
        let r_plus = self.position_eci(&epoch.add_seconds(dt));
        let r_minus = self.position_eci(&epoch.add_seconds(-dt));
        (r_plus - r_minus) / (2.0 * dt)
    }

    /// Short human-readable name of the ephemeris source (e.g. "meeus", "horizons").
    fn name(&self) -> &str;
}

/// Meeus analytical Moon ephemeris (Chapter 47 of "Astronomical Algorithms").
///
/// Wraps the existing [`moon_position_eci`] free function so it can be used
/// through the [`MoonEphemeris`] trait. Accuracy: see [`moon_position_eci`].
#[derive(Debug, Default, Clone, Copy)]
pub struct MeeusMoonEphemeris;

impl MoonEphemeris for MeeusMoonEphemeris {
    fn position_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
        moon_position_eci(epoch)
    }

    fn name(&self) -> &str {
        "meeus"
    }
}

/// Blanket implementation so that shared trait objects (`Arc<dyn MoonEphemeris>`)
/// and owned wrappers (`Arc<MeeusMoonEphemeris>`, …) can be used wherever a
/// concrete `MoonEphemeris` is expected.
///
/// This lets a single ephemeris instance be fanned out via `Arc::clone` to the
/// integrator's force model *and* to any number of auxiliary targeting helpers
/// without re-parsing tables or re-fetching data — e.g.
/// `ThirdBodyGravity::moon_with_ephemeris(Arc::clone(&shared_ephem))`.
impl<T: MoonEphemeris + ?Sized> MoonEphemeris for std::sync::Arc<T> {
    fn position_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
        (**self).position_eci(epoch)
    }

    fn velocity_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
        (**self).velocity_eci(epoch)
    }

    fn name(&self) -> &str {
        (**self).name()
    }
}

/// Moon ephemeris backed by a tabulated JPL Horizons vector table with
/// cubic Hermite interpolation.
///
/// Accuracy: depends on table sampling step. At 1-hour spacing the
/// interpolation error is well below 100 m over a multi-week mission
/// (dominated by third-body high-frequency perturbations, not cubic
/// truncation). See [`crate::horizons::HorizonsTable::interpolate`] for
/// the underlying method.
///
/// When queried outside the table's epoch range, this ephemeris falls back
/// to [`MeeusMoonEphemeris`] and increments an internal counter (retrievable
/// via [`HorizonsMoonEphemeris::fallback_count`]). This lets callers detect
/// silent drift into the lower-accuracy regime without panicking.
#[derive(Debug)]
pub struct HorizonsMoonEphemeris {
    table: crate::horizons::HorizonsTable,
    fallback: MeeusMoonEphemeris,
    fallback_count: std::sync::atomic::AtomicUsize,
}

impl HorizonsMoonEphemeris {
    /// Wrap an already-parsed `HorizonsTable`.
    pub fn from_table(table: crate::horizons::HorizonsTable) -> Self {
        Self {
            table,
            fallback: MeeusMoonEphemeris,
            fallback_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Load a Horizons CSV file and wrap it.
    pub fn from_file(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, crate::horizons::HorizonsError> {
        let table = crate::horizons::HorizonsTable::from_file(path)?;
        Ok(Self::from_table(table))
    }

    /// Fetch a Moon ephemeris from JPL Horizons (target `301`, center
    /// `500@399` = Earth geocenter) over the given epoch range and wrap it.
    ///
    /// Only available with the `fetch-horizons` feature on non-WASM targets.
    #[cfg(all(feature = "fetch-horizons", not(target_arch = "wasm32")))]
    pub fn fetch(
        start: &Epoch,
        stop: &Epoch,
        step: &str,
    ) -> Result<Self, crate::horizons::HorizonsError> {
        let table = crate::horizons::HorizonsTable::fetch_vector_table(
            "301", "500@399", start, stop, step, None,
        )?;
        Ok(Self::from_table(table))
    }

    /// First and last epochs in the underlying table.
    pub fn date_range(&self) -> Option<(Epoch, Epoch)> {
        self.table.date_range()
    }

    /// Number of times `position_eci` or `velocity_eci` fell back to Meeus
    /// because the query epoch was outside the table range.
    pub fn fallback_count(&self) -> usize {
        self.fallback_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn record_fallback(&self) {
        self.fallback_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl MoonEphemeris for HorizonsMoonEphemeris {
    fn position_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
        match self.table.interpolate(epoch) {
            Some(sample) => Vec3::from_raw(sample.position),
            None => {
                self.record_fallback();
                self.fallback.position_eci(epoch)
            }
        }
    }

    fn velocity_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
        // Hermite interpolation gives us analytic velocity — use it directly
        // rather than the trait's default central-difference implementation.
        match self.table.interpolate(epoch) {
            Some(sample) => Vec3::from_raw(sample.velocity),
            None => {
                self.record_fallback();
                self.fallback.velocity_eci(epoch)
            }
        }
    }

    fn name(&self) -> &str {
        "horizons"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_distance_range() {
        // Moon distance varies between ~356,500 km (perigee) and ~406,700 km (apogee)
        let dates = [
            Epoch::from_gregorian(2024, 3, 1, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 7, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 14, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 21, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 28, 0, 0, 0.0),
        ];

        for epoch in &dates {
            let pos = moon_position_eci(epoch);
            let dist = pos.magnitude();
            assert!(
                dist > 340_000.0 && dist < 420_000.0,
                "Moon distance at JD {} should be 340k-420k km, got {dist:.0} km",
                epoch.jd()
            );
        }
    }

    #[test]
    fn moon_mean_distance() {
        let n = 30;
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let total_dist: f64 = (0..n)
            .map(|i| {
                let epoch = epoch0.add_seconds(i as f64 * 86400.0);
                moon_position_eci(&epoch).magnitude()
            })
            .sum();
        let mean_dist = total_dist / n as f64;

        assert!(
            (mean_dist - 384_400.0).abs() < 5_000.0,
            "Mean moon distance should be ~384,400 km, got {mean_dist:.0} km"
        );
    }

    #[test]
    fn moon_orbital_period() {
        let epoch0 = Epoch::from_gregorian(2024, 3, 10, 0, 0, 0.0);
        let pos0 = moon_position_eci(&epoch0).normalize();

        let epoch1 = epoch0.add_seconds(27.3 * 86400.0);
        let pos1 = moon_position_eci(&epoch1).normalize();

        let dot = pos0.dot(&pos1);
        assert!(
            dot > 0.9,
            "Moon should return near starting direction after ~27.3 days, dot={dot:.3}"
        );

        let epoch_half = epoch0.add_seconds(13.7 * 86400.0);
        let pos_half = moon_position_eci(&epoch_half).normalize();
        let dot_half = pos0.dot(&pos_half);
        assert!(
            dot_half < 0.0,
            "Moon should be roughly opposite after half orbit, dot={dot_half:.3}"
        );
    }

    #[test]
    fn moon_not_in_ecliptic() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let pos = moon_position_eci(&epoch);
        let z_frac = pos.z().abs() / pos.magnitude();

        let mut max_z_frac = 0.0_f64;
        for i in 0..28 {
            let ep = epoch.add_seconds(i as f64 * 86400.0);
            let p = moon_position_eci(&ep);
            max_z_frac = max_z_frac.max(p.z().abs() / p.magnitude());
        }
        assert!(
            max_z_frac > 0.1,
            "Moon should have significant z-component at some point, max z/r = {max_z_frac:.3}"
        );
        let _ = z_frac;
    }

    #[test]
    fn moon_distance_apollo11_epoch() {
        // Apollo 11 TLI: 1969-07-16. Moon was near apogee, ~394,000 km (JPL Horizons)
        let epoch = Epoch::from_iso8601("1969-07-16T16:22:03Z").unwrap();
        let dist = moon_position_eci(&epoch).magnitude();
        // Meeus analytical model has ~2-3% distance error
        assert!(
            (dist - 394_000.0).abs() < 15_000.0,
            "Moon distance at Apollo 11 TLI should be ~394,000 km, got {dist:.0} km"
        );
    }

    #[test]
    fn meeus_ephemeris_matches_free_function() {
        // `MeeusMoonEphemeris` must delegate to `moon_position_eci` without
        // any transformation — trait wrapper should be a zero-cost abstraction.
        let ephem = MeeusMoonEphemeris;
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        assert_eq!(ephem.position_eci(&epoch), moon_position_eci(&epoch));
        assert_eq!(ephem.name(), "meeus");
    }

    #[test]
    fn meeus_ephemeris_velocity_is_finite_difference() {
        // Default `velocity_eci` should be a central difference of position.
        let ephem = MeeusMoonEphemeris;
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let v = ephem.velocity_eci(&epoch);
        // Moon orbital velocity is ~1.022 km/s.
        let v_mag = v.magnitude();
        assert!(
            (0.9..1.2).contains(&v_mag),
            "Moon velocity should be ~1 km/s, got {v_mag:.3} km/s"
        );
        // Cross-check against a manual central difference with a larger step.
        let dt = 10.0;
        let expected = (moon_position_eci(&epoch.add_seconds(dt))
            - moon_position_eci(&epoch.add_seconds(-dt)))
            / (2.0 * dt);
        let err = (v - expected).magnitude();
        assert!(
            err < 1e-3,
            "Velocity finite difference (1s) should match 10s finite difference within 1 m/s, err={err:e}"
        );
    }

    #[test]
    fn meeus_ephemeris_is_clone_and_default() {
        // `MeeusMoonEphemeris` is stateless so it should be `Default` + `Copy`.
        let e1 = MeeusMoonEphemeris;
        let e2 = MeeusMoonEphemeris;
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        assert_eq!(e1.position_eci(&epoch), e2.position_eci(&epoch));
    }

    #[test]
    fn horizons_moon_ephemeris_interpolates_within_range() {
        // Synthetic table with constant-velocity motion so we can verify
        // that the tabulated source is used (not Meeus).
        let csv = "\
$$SOE
2459000.0, A, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
2459000.5, A, 43200.0, 0.0, 0.0, 1.0, 0.0, 0.0,
2459001.0, A, 86400.0, 0.0, 0.0, 1.0, 0.0, 0.0,
$$EOE
";
        let table = crate::horizons::HorizonsTable::parse_csv(csv).unwrap();
        let ephem = HorizonsMoonEphemeris::from_table(table);
        assert_eq!(ephem.name(), "horizons");

        // At 1/4 of the way between the first two samples (0.125 days):
        // synthetic body at x = 10800 km, velocity 1 km/s along +x.
        let epoch = Epoch::from_jd(2459000.125);
        let pos = ephem.position_eci(&epoch);
        let vel = ephem.velocity_eci(&epoch);
        assert!((pos.x() - 10_800.0).abs() < 1e-6);
        assert!((vel.x() - 1.0).abs() < 1e-9);
        assert_eq!(ephem.fallback_count(), 0);
    }

    #[test]
    fn horizons_moon_ephemeris_falls_back_to_meeus_out_of_range() {
        let csv = "\
$$SOE
2459000.0, A, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
2459001.0, A, 86400.0, 0.0, 0.0, 1.0, 0.0, 0.0,
$$EOE
";
        let table = crate::horizons::HorizonsTable::parse_csv(csv).unwrap();
        let ephem = HorizonsMoonEphemeris::from_table(table);

        // Epoch way before the table — should fall back to Meeus.
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let pos = ephem.position_eci(&epoch);
        // Meeus should return a realistic Moon distance (not near 0).
        let dist = pos.magnitude();
        assert!(
            (300_000.0..500_000.0).contains(&dist),
            "Meeus fallback should return Moon-like distance, got {dist:.0} km"
        );
        assert_eq!(ephem.fallback_count(), 1);

        // Also querying velocity should increment the fallback counter.
        let _ = ephem.velocity_eci(&epoch);
        assert_eq!(ephem.fallback_count(), 2);
    }

    #[test]
    fn horizons_moon_ephemeris_date_range_exposed() {
        let csv = "\
$$SOE
2459000.0, A, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
2459001.0, A, 86400.0, 0.0, 0.0, 1.0, 0.0, 0.0,
$$EOE
";
        let table = crate::horizons::HorizonsTable::parse_csv(csv).unwrap();
        let ephem = HorizonsMoonEphemeris::from_table(table);
        let (first, last) = ephem.date_range().unwrap();
        assert_eq!(first.jd(), 2459000.0);
        assert_eq!(last.jd(), 2459001.0);
    }

    #[test]
    fn arc_dyn_moon_ephemeris_is_usable_via_blanket_impl() {
        // Regression guard for the blanket `impl MoonEphemeris for Arc<T>`.
        // Both `Arc<MeeusMoonEphemeris>` and `Arc<dyn MoonEphemeris>` must
        // satisfy the `MoonEphemeris` bound so they can be passed to
        // `ThirdBodyGravity::moon_with_ephemeris` (see orts perturbations).
        use std::sync::Arc;
        let owned: Arc<MeeusMoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let erased: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);

        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        // Calls must go through the blanket impl on `Arc<_>`, not through
        // auto-deref, because `moon_with_ephemeris<E: MoonEphemeris>` takes
        // the value by generic bound.
        assert_eq!(owned.position_eci(&epoch), moon_position_eci(&epoch));
        assert_eq!(erased.position_eci(&epoch), moon_position_eci(&epoch));
        assert_eq!(owned.name(), "meeus");
        assert_eq!(erased.name(), "meeus");

        // The velocity default should also forward through the blanket impl.
        let v_owned = owned.velocity_eci(&epoch);
        let v_erased = erased.velocity_eci(&epoch);
        assert_eq!(v_owned, v_erased);
    }

    #[test]
    fn default_velocity_eci_calls_position_at_plus_minus_one_second() {
        // Regression guard for the documented behavior of
        // `MoonEphemeris::velocity_eci`: "central finite difference over
        // `position_eci` with a 1-second step". Uses a counting wrapper to
        // verify both that `position_eci` is the only method called and
        // that the offsets are exactly ±1 second.
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingEphem {
            calls: AtomicUsize,
            last_offsets: Mutex<Vec<f64>>,
            base_jd: f64,
        }
        impl MoonEphemeris for CountingEphem {
            fn position_eci(&self, epoch: &Epoch) -> Vec3<frame::Gcrs> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                let offset_sec = (epoch.jd() - self.base_jd) * 86400.0;
                self.last_offsets.lock().unwrap().push(offset_sec);
                // Linear test input: position at time t is (t, 0, 0) where
                // t is in seconds from base_jd. The slope is 1 km/s on x.
                Vec3::new(offset_sec, 0.0, 0.0)
            }
            fn name(&self) -> &str {
                "counting"
            }
        }

        let base = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let ephem = CountingEphem {
            calls: AtomicUsize::new(0),
            last_offsets: Mutex::new(Vec::new()),
            base_jd: base.jd(),
        };
        let v = ephem.velocity_eci(&base);

        // position_eci must have been called exactly twice.
        assert_eq!(
            ephem.calls.load(Ordering::Relaxed),
            2,
            "default velocity_eci should call position_eci exactly twice"
        );
        // The offsets must be ±1 second from the base epoch. The tolerance
        // reflects the ~50 microsecond precision of `Epoch::add_seconds` at
        // modern JDs (f64 ULP on ~2.46e6 JD ≈ 5e-10 days ≈ 50 µs).
        let mut offsets = ephem.last_offsets.lock().unwrap().clone();
        offsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            (offsets[0] - (-1.0)).abs() < 1e-3,
            "first offset should be -1 s, got {} s",
            offsets[0]
        );
        assert!(
            (offsets[1] - 1.0).abs() < 1e-3,
            "second offset should be +1 s, got {} s",
            offsets[1]
        );
        // On the linear test input the central difference recovers the slope
        // to within the JD ULP precision on ±1 s inputs. The default
        // implementation divides by `2 * 1.0` where the numerator is
        // `position(+dt) - position(-dt) ≈ 2 km` — so the output is dominated
        // by JD precision on the offsets rather than on the raw slope.
        assert!(
            (v.x() - 1.0).abs() < 1e-3,
            "linear input slope should be 1 km/s, got {}",
            v.x()
        );
        assert_eq!(v.y(), 0.0);
        assert_eq!(v.z(), 0.0);
    }
}
