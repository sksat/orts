//! SGP4 / SDP4 propagation of [`Sgp4Elements`] into a TEME state vector.
//!
//! Wraps the [`sgp4`] crate (a reference-quality SGP4+SDP4 implementation
//! validated against Vallado's "Revisiting Spacetrack Report #3"). The
//! propagator runs in **AFSPC compatibility mode** — the WGS72 geopotential and
//! the AFSPC sidereal-time / epoch expressions — because that is the convention
//! TLE/OMM catalog element sets are generated for, and the mode `sgp4` validates
//! against the official test vectors.
//!
//! The propagation path is allocator-free: it feeds the numeric
//! [`Sgp4Elements`] straight into `sgp4`'s `Orbit` / `Constants` API (the
//! `sgp4` dependency is pulled with only its `libm` feature), so SGP4
//! propagation works in `no_std` builds without `alloc`.
//!
//! Output is in the True Equator, Mean Equinox ([`Teme`]) frame, in km and
//! km/s. Rotating TEME into an integration frame ([`Gcrs`](crate::frame::Gcrs) /
//! [`SimpleEci`](crate::frame::SimpleEci)) is a separate step.

use sgp4::{Constants, MinutesSinceEpoch, Orbit, WGS72, afspc_epoch_to_sidereal_time};

use crate::elements::Sgp4Elements;
use crate::epoch::{DateTime, Epoch, Utc};
use crate::frame::{Teme, Vec3};

/// Error raised while building or running an SGP4 propagator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sgp4Error {
    /// The mean elements could not initialize a propagator — a non-positive
    /// mean motion or an epoch eccentricity outside `[0, 1)`.
    Initialization,
    /// Propagation to the requested time diverged: orbital decay, a negative
    /// semi-latus rectum, or a perturbed eccentricity outside `[0, 1)`.
    Diverged,
}

impl core::fmt::Display for Sgp4Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Sgp4Error::Initialization => {
                f.write_str("SGP4 elements could not initialize a propagator")
            }
            Sgp4Error::Diverged => f.write_str("SGP4 propagation diverged"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Sgp4Error {}

/// An SGP4 propagator built from a mean element set.
///
/// Construction precomputes `sgp4`'s epoch `Constants`; they are not mutated
/// during propagation, so a single propagator can be reused to generate a whole
/// trajectory (one `Constants` build amortized over many `propagate` calls).
#[derive(Debug, Clone)]
pub struct Sgp4Propagator {
    constants: Constants,
    epoch: Epoch<Utc>,
}

impl Sgp4Propagator {
    /// Build a propagator from a mean element set.
    ///
    /// Uses AFSPC compatibility mode (WGS72 + AFSPC sidereal time / epoch).
    pub fn from_elements(elements: &Sgp4Elements) -> Result<Self, Sgp4Error> {
        // Reject non-finite inputs up front: a NaN slips through sgp4's `<= 0`
        // and range checks (every comparison is false), so guard the public
        // entry point explicitly.
        if ![
            elements.inclination,
            elements.raan,
            elements.eccentricity,
            elements.argument_of_perigee,
            elements.mean_anomaly,
            elements.mean_motion,
            elements.bstar,
        ]
        .iter()
        .all(|x| x.is_finite())
        {
            return Err(Sgp4Error::Initialization);
        }

        // arika stores angles in radians and mean motion in rad/s; sgp4's
        // `from_kozai_elements` wants radians and the Kozai mean motion in
        // rad/min (rad/s × 60).
        let orbit = Orbit::from_kozai_elements(
            &WGS72,
            elements.inclination,
            elements.raan,
            elements.eccentricity,
            elements.argument_of_perigee,
            elements.mean_anomaly,
            elements.mean_motion * 60.0,
        )
        .map_err(|_| Sgp4Error::Initialization)?;

        let constants = Constants::new(
            WGS72,
            afspc_epoch_to_sidereal_time,
            afspc_years_since_j2000(&elements.epoch.to_datetime()),
            elements.bstar,
            orbit,
        )
        .map_err(|_| Sgp4Error::Initialization)?;

        Ok(Self {
            constants,
            epoch: elements.epoch,
        })
    }

    /// Propagate to `minutes` after the element-set epoch.
    ///
    /// Returns `(position, velocity)` in the [`Teme`] frame, in km and km/s.
    pub fn propagate_minutes_since_epoch(
        &self,
        minutes: f64,
    ) -> Result<(Vec3<Teme>, Vec3<Teme>), Sgp4Error> {
        let p = self
            .constants
            .propagate_afspc_compatibility_mode(MinutesSinceEpoch(minutes))
            .map_err(|_| Sgp4Error::Diverged)?;
        Ok((
            Vec3::<Teme>::new(p.position[0], p.position[1], p.position[2]),
            Vec3::<Teme>::new(p.velocity[0], p.velocity[1], p.velocity[2]),
        ))
    }

    /// Propagate to the absolute UTC epoch `t`.
    ///
    /// The elapsed time is naive UTC minutes (`(t − epoch)`), matching SGP4's
    /// leap-second-agnostic time convention. Returns `(position, velocity)` in
    /// the [`Teme`] frame, in km and km/s.
    pub fn propagate(&self, t: Epoch<Utc>) -> Result<(Vec3<Teme>, Vec3<Teme>), Sgp4Error> {
        let minutes = (t.jd() - self.epoch.jd()) * 1440.0;
        self.propagate_minutes_since_epoch(minutes)
    }
}

/// Julian years since J2000, AFSPC compatibility expression.
///
/// Mirrors `sgp4::julian_years_since_j2000_afspc_compatibility_mode` but reads
/// arika's [`DateTime`] fields directly, so the no-alloc path avoids
/// constructing a `chrono` datetime. The integer arithmetic (truncating
/// division) is preserved exactly so the epoch matches the AFSPC reference.
fn afspc_years_since_j2000(d: &DateTime) -> f64 {
    let year = d.year as u32;
    let month = d.month;
    let day = d.day;
    let jd_at_midnight =
        (367 * year - (7 * (year + (month + 9) / 12)) / 4 + 275 * month / 9 + day) as f64;
    // Fraction of the day: (((s)/60 + min)/60 + hour)/24, with `sec` already
    // carrying the sub-second part as a fractional f64.
    let day_fraction = ((d.sec / 60.0 + d.min as f64) / 60.0 + d.hour as f64) / 24.0;
    (jd_at_midnight + 1721013.5 + day_fraction - 2451545.0) / 365.25
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::tle;
    use alloc::format;

    // Vallado "Revisiting Spacetrack Report #3" TEME verification satellite
    // (catalog 00005). Expected TEME states (km, km/s) at minutes-since-epoch,
    // from the official SGP4 test vectors (AFSPC mode).
    const L1: &str = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
    const L2: &str = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";

    #[test]
    fn vallado_teme_example_sat_00005() {
        let text = format!("{L1}\n{L2}");
        let elements = tle::parse(&text).unwrap().elements;
        let prop = Sgp4Propagator::from_elements(&elements).unwrap();

        // (tsince [min], position [km], velocity [km/s])
        let cases = [
            (
                0.0,
                [7022.46529266, -1400.08296755, 0.03995155],
                [1.893841015, 6.405893759, 4.534807250],
            ),
            (
                360.0,
                [-7154.03120202, -3783.17682504, -3536.19412294],
                [4.741887409, -4.151817765, -2.093935425],
            ),
            (
                1440.0,
                [-938.55923943, -6268.18748831, -4294.02924751],
                [7.536105209, -0.427127707, 0.989878080],
            ),
        ];

        for (t, expected_r, expected_v) in cases {
            let (r, v) = prop.propagate_minutes_since_epoch(t).unwrap();
            let r = r.into_inner();
            let v = v.into_inner();
            for i in 0..3 {
                assert!(
                    (r[i] - expected_r[i]).abs() < 1.0e-4,
                    "position t={t} i={i}: got {} expected {}",
                    r[i],
                    expected_r[i]
                );
                assert!(
                    (v[i] - expected_v[i]).abs() < 1.0e-6,
                    "velocity t={t} i={i}: got {} expected {}",
                    v[i],
                    expected_v[i]
                );
            }
        }
    }

    #[test]
    fn propagate_by_absolute_epoch_matches_minutes() {
        let text = format!("{L1}\n{L2}");
        let elements = tle::parse(&text).unwrap().elements;
        let prop = Sgp4Propagator::from_elements(&elements).unwrap();

        // propagate(epoch) must agree with propagate_minutes_since_epoch(0).
        let (r_abs, v_abs) = prop.propagate(elements.epoch).unwrap();
        let (r_min, v_min) = prop.propagate_minutes_since_epoch(0.0).unwrap();
        assert!((r_abs.into_inner() - r_min.into_inner()).norm() < 1.0e-6);
        assert!((v_abs.into_inner() - v_min.into_inner()).norm() < 1.0e-9);
    }

    #[test]
    fn vallado_deep_space_sat_04632() {
        // Deep-space (SDP4) Vallado verification satellite: mean motion
        // 1.20 rev/day (< 2π/225 rad/min) drives the Lyddane deep-space path,
        // including backward propagation (negative tsince).
        const DL1: &str = "1 04632U 70093B   04031.91070959 -.00000084  00000-0  10000-3 0  9955";
        const DL2: &str = "2 04632  11.4628 273.1101 1450506 207.6000 143.9350  1.20231981 44145";
        let elements = tle::parse(&format!("{DL1}\n{DL2}")).unwrap().elements;
        let prop = Sgp4Propagator::from_elements(&elements).unwrap();

        let cases = [
            (
                0.0,
                [2334.11450085, -41920.44035349, -0.03867437],
                [2.826321032, -0.065091664, 0.570936053],
            ),
            (
                -5184.0,
                [-29020.02587128, 13819.84419063, -5713.33679183],
                [-1.768068390, -3.235371192, -0.395206135],
            ),
            (
                -4896.0,
                [-15129.94694545, -36907.74526221, -3487.56256701],
                [2.581167187, -1.524204737, 0.504805763],
            ),
        ];

        for (t, expected_r, expected_v) in cases {
            let (r, v) = prop.propagate_minutes_since_epoch(t).unwrap();
            let r = r.into_inner();
            let v = v.into_inner();
            for i in 0..3 {
                assert!(
                    (r[i] - expected_r[i]).abs() < 1.0e-3,
                    "position t={t} i={i}: got {} expected {}",
                    r[i],
                    expected_r[i]
                );
                assert!(
                    (v[i] - expected_v[i]).abs() < 1.0e-6,
                    "velocity t={t} i={i}: got {} expected {}",
                    v[i],
                    expected_v[i]
                );
            }
        }
    }

    #[test]
    fn afspc_epoch_matches_sgp4_reference() {
        use sgp4::chrono::NaiveDate;

        // (year, month, day, hour, min, sec, nanosecond) across Jan/Feb/Mar,
        // the year-2000 pivot, a leap day with a sub-second, and a far date.
        let cases = [
            (2000, 1, 1, 12, 0, 0u32, 0u32),
            (1999, 12, 31, 23, 59, 59, 0),
            (2024, 2, 29, 6, 30, 15, 500_000_000),
            (2056, 3, 1, 0, 0, 0, 0),
            (1980, 6, 15, 18, 45, 7, 250_000_000),
        ];
        for (y, mo, d, h, mi, s, ns) in cases {
            let dt = DateTime::new(y, mo, d, h, mi, s as f64 + ns as f64 / 1.0e9);
            let mine = afspc_years_since_j2000(&dt);
            let ndt = NaiveDate::from_ymd_opt(y, mo, d)
                .unwrap()
                .and_hms_nano_opt(h, mi, s, ns)
                .unwrap();
            let reference = sgp4::julian_years_since_j2000_afspc_compatibility_mode(&ndt);
            assert!(
                (mine - reference).abs() < 1.0e-12,
                "{y}-{mo}-{d}T{h}:{mi}:{s}.{ns}: {mine} vs {reference}"
            );
        }
    }
}
