//! Meeus analytical solar ephemeris and Equation of Time.
//!
//! Low-precision (~1 arcminute) analytic model from Meeus "Astronomical
//! Algorithms" Chapter 25 / Chapter 28. Returns positions in [`crate::frame::Gcrs`]
//! (the analytical "geocentric inertial") rather than [`crate::frame::SimpleEci`],
//! documenting that this is not a propagator state vector.
//!
//! # Frame
//!
//! The series itself is referred to the **mean equinox and equator of date**:
//! the mean longitude `L₀ = 280.46646 + 36000.76983·T` advances at the tropical
//! rate (360.0076983°/Julian year) and the obliquity used to rotate ecliptic →
//! equatorial is the mean obliquity of date. The returned vectors are therefore
//! rotated back to J2000 with the IAU 1976 precession
//! ([`crate::earth::fk5`]) before being typed `Vec3<Gcrs>`; nutation and the
//! J2000→GCRS frame bias are neglected (≤ 17″ and ~20 mas respectively, both
//! far below the model's own ~1′ accuracy).
//!
//! Skipping that rotation is a secular direction error equal to the accumulated
//! precession — 0.335° in 2024, growing ~1.4°/century — and it does not cancel
//! against a propagator state, so it is not a frame-labelling technicality.
//! [`equation_of_time`] deliberately stays in of-date coordinates: it is a
//! difference of two of-date longitudes.
//!
//! # Time scale
//!
//! Meeus ephemerides take a dynamical time argument (TDB), so the public
//! signatures require `&Epoch<Tdb>` — the caller converts at the boundary
//! (`utc.to_tdb()`), which keeps the TDB dependency explicit in the type
//! rather than hidden inside each function.

use crate::body::KnownBody;
use nalgebra::Vector3;

#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::epoch::{Epoch, Tdb};
use crate::frame::{self, Rotation, Vec3};
use crate::planets;
use crate::sun::AU_KM;

/// Solar orbital elements at epoch.
///
/// Intermediate values used by both `sun_direction_eci` and `equation_of_time`.
struct SolarElements {
    /// Mean longitude [degrees] (not normalized to 0-360)
    l0_deg: f64,
    /// Ecliptic longitude [radians]
    lambda_rad: f64,
    /// Obliquity of the ecliptic [radians]
    epsilon_rad: f64,
}

/// Compute solar orbital elements at the given epoch.
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
fn solar_elements(epoch: &Epoch<Tdb>) -> SolarElements {
    let t = epoch.centuries_since_j2000();

    // Mean longitude (degrees)
    let l0 = 280.46646 + 36000.76983 * t;
    // Mean anomaly (degrees)
    let m_deg = 357.52911 + 35999.05029 * t;
    let m = m_deg.to_radians();

    // Equation of center (degrees)
    let c = (1.9146 - 0.004817 * t) * m.sin() + 0.019993 * (2.0 * m).sin();

    // Sun's ecliptic longitude (radians)
    let lambda = (l0 + c).to_radians();

    // Obliquity of the ecliptic
    let epsilon = planets::obliquity(epoch);

    SolarElements {
        l0_deg: l0,
        lambda_rad: lambda,
        epsilon_rad: epsilon,
    }
}

/// Mean obliquity of the ecliptic at J2000 [rad].
///
/// The fixed obliquity that rotates a J2000 *ecliptic* vector — such as the
/// Standish planetary elements' output — into J2000 equatorial coordinates.
/// Evaluating [`planets::obliquity`] at J2000 keeps the polynomial's leading
/// coefficient as the single source of truth.
fn j2000_obliquity() -> f64 {
    planets::obliquity(&Epoch::<Tdb>::from_jd_tdb(crate::epoch::J2000_JD))
}

/// Sun direction (unit vector) referred to the mean equator and equinox of date
/// — the frame the Meeus series is actually expressed in.
///
/// Kept separate from [`sun_direction_eci`] so the precession rotation to J2000
/// is a single visible step, and so tests can measure the size of that step.
/// A caller that needs this frame — a right ascension that has to share GMST's
/// equinox, as in a local hour angle `GMST + λ − α` — rotates the public
/// J2000 direction with
/// [`Rotation::<Gcrs, MeanEquinoxOfDate>::iau1976_precession`](crate::earth::mean_equinox).
pub(crate) fn sun_direction_mean_of_date(epoch: &Epoch<Tdb>) -> Vec3<frame::MeanEquinoxOfDate> {
    let el = solar_elements(epoch);

    let x = el.lambda_rad.cos();
    let y = el.epsilon_rad.cos() * el.lambda_rad.sin();
    let z = el.epsilon_rad.sin() * el.lambda_rad.sin();

    Vec3::from_raw(Vector3::new(x, y, z).normalize())
}

/// Approximate sun direction (unit vector) in ECI (J2000) frame.
///
/// Uses a low-precision analytical model based on mean orbital elements.
/// Accuracy is ~1 arcminute, sufficient for visualization purposes.
///
/// The Meeus series is referred to the mean equinox of date, so the result is
/// rotated back to J2000 by the IAU 1976 precession (see the module docs).
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
pub fn sun_direction_eci(epoch: &Epoch<Tdb>) -> Vec3<frame::Gcrs> {
    Rotation::<frame::MeanEquinoxOfDate, frame::Gcrs>::iau1976_precession(&epoch.to_tt())
        .transform(&sun_direction_mean_of_date(epoch))
}

/// Equation of Time [hours].
///
/// Returns `apparent_solar_time - mean_solar_time`.
/// Positive means the apparent Sun is ahead of the mean Sun.
///
/// Range: approximately -0.27 to +0.27 hours (-16 to +16 minutes).
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 28.
pub fn equation_of_time(epoch: &Epoch<Tdb>) -> f64 {
    let el = solar_elements(epoch);

    // Right ascension from ecliptic longitude
    let alpha_rad = f64::atan2(
        el.epsilon_rad.cos() * el.lambda_rad.sin(),
        el.lambda_rad.cos(),
    );

    // EoT = L₀ - α (apparent - mean), then convert to hours
    // Positive in November (sundial fast), negative in February (sundial slow).
    let l0_rad = el.l0_deg.to_radians();
    let mut eot_rad = l0_rad - alpha_rad;

    // Normalize to [-π, π]
    eot_rad = ((eot_rad + core::f64::consts::PI) % core::f64::consts::TAU + core::f64::consts::TAU)
        % core::f64::consts::TAU
        - core::f64::consts::PI;

    // Convert radians to hours: 2π rad = 24 hours
    eot_rad * 24.0 / core::f64::consts::TAU
}

/// Sun-Earth distance [km] at the given epoch.
///
/// Uses simplified Meeus model with eccentricity correction.
/// Accuracy: ~0.01 AU (~1.5 million km), sufficient for perturbation calculations.
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
pub fn sun_distance_km(epoch: &Epoch<Tdb>) -> f64 {
    let t = epoch.centuries_since_j2000();

    let m_deg = 357.52911 + 35999.05029 * t;
    let m = m_deg.to_radians();

    // Distance in AU (Meeus Eq. 25.5)
    let r_au = 1.000_140_12 - 0.016_708_17 * m.cos() - 0.000_139_89 * (2.0 * m).cos();

    r_au * AU_KM
}

/// Sun position vector in ECI (J2000) frame [km].
///
/// Returns the geocentric position of the Sun. Combines direction and distance.
pub fn sun_position_eci(epoch: &Epoch<Tdb>) -> Vec3<frame::Gcrs> {
    let direction = sun_direction_eci(epoch);
    let distance = sun_distance_km(epoch);
    direction * distance
}

/// Sun distance [km] from a given central body.
///
/// - `"earth"` / `"moon"`: delegates to [`sun_distance_km`]
/// - Other known planets: computed from heliocentric orbital elements
/// - Unknown bodies: fallback to Earth-Sun distance
pub fn sun_distance_from_body(body: &str, epoch: &Epoch<Tdb>) -> f64 {
    match body {
        "earth" | "moon" => sun_distance_km(epoch),
        _ => planets::heliocentric_position_ecliptic(body, epoch)
            .map(|p| p.magnitude())
            .unwrap_or_else(|| sun_distance_km(epoch)),
    }
}

/// Sun direction (unit vector) as seen from a given central body, in J2000 equatorial frame.
///
/// - `"earth"` / `"moon"`: delegates to [`sun_direction_eci`] (Moon parallax < 0.15°, negligible)
/// - Other known planets: computed from heliocentric orbital elements
/// - Unknown bodies: fallback to +X direction (vernal equinox)
///
/// The returned vector points FROM the body TOWARD the Sun.
pub fn sun_direction_from_body(body: &str, epoch: &Epoch<Tdb>) -> Vec3<frame::Gcrs> {
    match body {
        "earth" | "moon" => sun_direction_eci(epoch),
        _ => {
            if let Some(body_pos_ecl) = planets::heliocentric_position_ecliptic(body, epoch) {
                // The Standish planetary elements are referred to the J2000 mean
                // ecliptic and equinox (their mean longitudes advance at the
                // sidereal rate), so this branch needs no precession rotation —
                // unlike the geocentric Meeus Sun above — and the ecliptic →
                // equatorial rotation takes the *J2000* obliquity, not the
                // obliquity of date (which would leave 11″ of frame error in
                // 2024, 35″ in 2075).
                let sun_dir_ecl = -body_pos_ecl;
                Vec3::from_raw(
                    planets::ecliptic_to_equatorial(&sun_dir_ecl, j2000_obliquity()).normalize(),
                )
            } else {
                // Unknown body: fallback to +X (vernal equinox direction)
                Vec3::new(1.0, 0.0, 0.0)
            }
        }
    }
}

/// Why a central body has no Sun position to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunPositionError {
    /// The body has no ephemeris here. [`crate::planets`] carries Standish
    /// elements for Mercury through Saturn only, so Uranus and Neptune cannot
    /// be placed relative to the Sun.
    UnsupportedBody(KnownBody),
    /// The central body *is* the Sun, so there is no body-to-Sun vector — and
    /// no solar third-body term or shadow to model either.
    CentralBodyIsSun,
}

impl core::fmt::Display for SunPositionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedBody(body) => write!(
                f,
                "no Sun ephemeris for {}: this crate carries planetary elements \
                 for Mercury through Saturn only",
                body.properties().name
            ),
            Self::CentralBodyIsSun => {
                write!(f, "the Sun has no position relative to itself")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SunPositionError {}

/// Sun position vector as seen from `body`, in the J2000 equatorial frame [km].
///
/// The vector points from the body toward the Sun — what a third-body term and
/// a cannonball SRP model both need. Fails rather than substituting Earth's,
/// because a wrong central body is not a small error: from Mars in 2026 the
/// geocentric Sun direction is up to 176° away from the true one, and the
/// distance ratio scales the third-body term by up to 3.8x.
///
/// The Moon's vector carries lunar parallax: Moon-to-Sun is
/// `(Earth → Sun) − (Earth → Moon)`, and both terms are already available (as
/// exact as those two analytical series are, which is ~1′ here). The `&str`
/// helper shares Earth's vector for the Moon instead, dropping a parallax of
/// up to 0.15° — nine times that ~1′ — so there is no reason to keep it where
/// the Moon position is one subtraction away.
pub fn sun_position_from_body(
    body: KnownBody,
    epoch: &Epoch<Tdb>,
) -> Result<Vec3<frame::Gcrs>, SunPositionError> {
    match body {
        KnownBody::Sun => Err(SunPositionError::CentralBodyIsSun),
        KnownBody::Earth => Ok(sun_position_eci(epoch)),
        KnownBody::Moon => Ok(Vec3::from_raw(
            sun_position_eci(epoch).into_inner()
                - crate::moon::moon_position_eci(epoch).into_inner(),
        )),
        KnownBody::Mercury
        | KnownBody::Venus
        | KnownBody::Mars
        | KnownBody::Jupiter
        | KnownBody::Saturn => {
            // The string keys the element table uses. Spelled out here rather
            // than lowercased from `properties().name`, which needs `alloc`
            // and this crate builds without it.
            let name = match body {
                KnownBody::Mercury => "mercury",
                KnownBody::Venus => "venus",
                KnownBody::Mars => "mars",
                KnownBody::Jupiter => "jupiter",
                KnownBody::Saturn => "saturn",
                _ => unreachable!("the arm above lists exactly these bodies"),
            };
            let pos_ecl = planets::heliocentric_position_ecliptic(name, epoch)
                .ok_or(SunPositionError::UnsupportedBody(body))?;
            // Body-to-Sun is the negated heliocentric position. The Standish
            // elements are referred to the J2000 mean ecliptic and equinox, so
            // this needs no precession rotation and the ecliptic → equatorial
            // rotation takes the J2000 obliquity. Same reasoning as
            // `sun_direction_from_body`.
            let sun_ecl = -pos_ecl;
            Ok(Vec3::from_raw(planets::ecliptic_to_equatorial(
                &sun_ecl,
                j2000_obliquity(),
            )))
        }
        KnownBody::Uranus | KnownBody::Neptune => Err(SunPositionError::UnsupportedBody(body)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IAU general precession in longitude [arcsec per Julian century].
    /// Independent of arika's own precession code (which uses the IAU 1976
    /// 5029.0966″/century; the two differ by ~0.3″/century).
    const GENERAL_PRECESSION_ARCSEC_PER_CENTURY: f64 = 5028.796195;

    // Frame tests: the Meeus series is of-date, the returned vector is J2000

    #[test]
    fn sun_direction_agrees_with_the_planetary_model_in_j2000() {
        // Cross-model consistency, no external ephemeris required. The
        // Standish/Meeus planetary elements (`planets`) are referred to the
        // J2000 mean ecliptic — their mean longitudes advance at the sidereal
        // rate — so -r_earth is an independent J2000 geocentric Sun direction.
        // The Meeus geocentric series is of-date, so before the precession
        // rotation the two disagreed by the whole accumulated precession
        // (0.335° in 2024, 0.70° in 2050); they now agree to well within the
        // ~1 arcminute accuracy both models claim.
        for (y, m, d) in [
            (1950, 6, 1),
            (2000, 1, 1),
            (2024, 3, 20),
            (2024, 9, 22),
            (2050, 1, 1),
            (2075, 7, 4),
            (2100, 1, 1),
        ] {
            let epoch = Epoch::from_gregorian(y, m, d, 12, 0, 0.0).to_tdb();
            let sun = sun_direction_eci(&epoch).into_inner();
            let earth = planets::heliocentric_position_ecliptic("earth", &epoch)
                .expect("earth is a known body");
            // The J2000 obliquity, matching the frame the Standish elements
            // are referred to — an obliquity of date here would tilt the
            // expectation by the same amount the implementation used to.
            let geocentric_sun =
                planets::ecliptic_to_equatorial(&(-earth), j2000_obliquity()).normalize();
            let sep = sun
                .dot(&geocentric_sun)
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            // The residual is flat at 4.5-8.0" across 1950-2100 (the two
            // models' own accuracy). The bound is set just above that so it
            // also catches a frame error that grows with |T| — either the
            // missing precession (0.335° in 2024, 1.4° by 2100) or an
            // obliquity-of-date rotation applied to the J2000 planetary
            // elements (11″ in 2024, 35″ by 2075).
            assert!(
                sep < 0.005,
                "{y}-{m:02}-{d:02}: Meeus Sun and the planetary model disagree by {sep:.4}° \
                 (precession left unremoved would give ~{:.2}°)",
                GENERAL_PRECESSION_ARCSEC_PER_CENTURY * epoch.centuries_since_j2000() / 3600.0
            );
        }
    }

    #[test]
    fn planet_branch_rotates_the_ecliptic_with_the_j2000_obliquity() {
        // `sun_direction_from_body`'s planet branch takes the Standish
        // heliocentric elements — J2000 mean ecliptic and equinox — into
        // equatorial coordinates. Rotating that result back about the x-axis by
        // the J2000 obliquity must land exactly on -r_body in the ecliptic
        // frame. It does not if the forward rotation used the obliquity of
        // date: the mismatch is 11 arcsec in 2024 and 35 arcsec by 2075, which
        // no other test on this branch (unit norm, variation over six months)
        // can see.
        for (body, y) in [("mars", 2024), ("mars", 2075), ("jupiter", 2100)] {
            let epoch = Epoch::from_gregorian(y, 5, 10, 0, 0, 0.0).to_tdb();
            let dir_eq = sun_direction_from_body(body, &epoch).into_inner();
            // The inverse rotation is the same x-rotation with the sign flipped.
            let dir_ecl = planets::ecliptic_to_equatorial(&dir_eq, -j2000_obliquity());

            let expected = (-planets::heliocentric_position_ecliptic(body, &epoch)
                .expect("known body"))
            .normalize();
            let sep_arcsec = dir_ecl.dot(&expected).clamp(-1.0, 1.0).acos().to_degrees() * 3600.0;
            let of_date_error_arcsec = (0.0130042 * epoch.centuries_since_j2000()).abs() * 3600.0;
            assert!(
                sep_arcsec < 1e-3,
                "{body} {y}: ecliptic round trip off by {sep_arcsec:.3} arcsec \
                 (an obliquity of date would give ~{of_date_error_arcsec:.1})"
            );
        }
    }

    #[test]
    fn sun_of_date_to_j2000_step_is_the_accumulated_precession() {
        // The rotation applied on the way out must be precession-sized, not
        // zero (the old behaviour) and not something else. The expected value
        // comes from the IAU general-precession rate, not from arika's own
        // precession code.
        // 2024 → 0.335°, 2050 → 0.698°, 1975 → 0.349° (before J2000, the same
        // rotation the other way).
        for y in [2024, 2050, 1975] {
            let epoch = Epoch::from_gregorian(y, 7, 1, 0, 0, 0.0).to_tdb();
            let t = epoch.centuries_since_j2000();
            let of_date = sun_direction_mean_of_date(&epoch).into_inner();
            let j2000 = sun_direction_eci(&epoch).into_inner();
            let sep = of_date.dot(&j2000).clamp(-1.0, 1.0).acos().to_degrees();
            // The Sun sits on the ecliptic (β ≈ 0), so the separation is the
            // full precession in longitude.
            let expected = (GENERAL_PRECESSION_ARCSEC_PER_CENTURY * t / 3600.0).abs();
            assert!(
                (sep - expected).abs() < 0.002,
                "{y}: of-date → J2000 step {sep:.4}°, expected {expected:.4}°"
            );
        }
    }

    // Equation of Time tests

    #[test]
    fn eot_february_negative() {
        // Mid-February: EoT ≈ -14 minutes (sundial slow, apparent sun behind mean sun)
        let epoch = Epoch::from_gregorian(2024, 2, 12, 12, 0, 0.0);
        let eot_min = equation_of_time(&epoch.to_tdb()) * 60.0;
        assert!(
            (eot_min - (-14.0)).abs() < 2.0,
            "Feb 12: EoT={eot_min:.1} min, expected ~-14"
        );
    }

    #[test]
    fn eot_november_positive() {
        // Early November: EoT ≈ +16 minutes (sundial fast, apparent sun ahead of mean sun)
        let epoch = Epoch::from_gregorian(2024, 11, 3, 12, 0, 0.0);
        let eot_min = equation_of_time(&epoch.to_tdb()) * 60.0;
        assert!(
            (eot_min - 16.0).abs() < 2.0,
            "Nov 3: EoT={eot_min:.1} min, expected ~+16"
        );
    }

    #[test]
    fn eot_april_near_zero() {
        // Mid-April: EoT ≈ 0 minutes (one of the four zero-crossings)
        let epoch = Epoch::from_gregorian(2024, 4, 15, 12, 0, 0.0);
        let eot_min = equation_of_time(&epoch.to_tdb()) * 60.0;
        assert!(
            eot_min.abs() < 2.0,
            "Apr 15: EoT={eot_min:.1} min, expected ~0"
        );
    }

    #[test]
    fn eot_annual_range() {
        // EoT should stay within ±17 minutes throughout the year
        for month in 1..=12 {
            let epoch = Epoch::from_gregorian(2024, month, 15, 12, 0, 0.0);
            let eot_min = equation_of_time(&epoch.to_tdb()) * 60.0;
            assert!(
                eot_min.abs() < 17.0,
                "Month {month}: EoT={eot_min:.1} min, out of range"
            );
        }
    }

    // Sun direction tests

    #[test]
    fn sun_direction_is_unit_vector() {
        // Check at several dates across a year
        let dates = [
            Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 9, 22, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 12, 21, 12, 0, 0.0),
        ];
        for epoch in &dates {
            let dir = sun_direction_eci(&epoch.to_tdb());
            let norm = dir.magnitude();
            assert!(
                (norm - 1.0).abs() < 1e-10,
                "Not unit vector at JD {}: norm = {norm}",
                epoch.jd()
            );
        }
    }

    #[test]
    fn march_equinox_sun_near_plus_x() {
        // At March equinox (~2024-03-20), sun is near +X direction (RA ≈ 0°)
        let epoch = Epoch::from_gregorian(2024, 3, 20, 3, 6, 0.0); // ~03:06 UTC is 2024 equinox
        let dir = sun_direction_eci(&epoch.to_tdb());

        // X should be dominant and positive
        assert!(
            dir.x() > 0.9,
            "March equinox: x={:.3} should be > 0.9",
            dir.x()
        );
        // Y and Z should be small
        assert!(
            dir.y().abs() < 0.2,
            "March equinox: y={:.3} should be near 0",
            dir.y()
        );
        assert!(
            dir.z().abs() < 0.1,
            "March equinox: z={:.3} should be near 0",
            dir.z()
        );
    }

    #[test]
    fn june_solstice_sun_positive_z() {
        // At June solstice (~2024-06-20), sun has significant +Z (northern declination ~23.4°)
        let epoch = Epoch::from_gregorian(2024, 6, 20, 20, 51, 0.0);
        let dir = sun_direction_eci(&epoch.to_tdb());

        // Z should be positive and near sin(23.44°) ≈ 0.398
        assert!(
            dir.z() > 0.35,
            "June solstice: z={:.3} should be > 0.35",
            dir.z()
        );
        // X should be near 0 (RA ≈ 90°)
        assert!(
            dir.x().abs() < 0.15,
            "June solstice: x={:.3} should be near 0",
            dir.x()
        );
        // Y should be dominant and positive
        assert!(
            dir.y() > 0.85,
            "June solstice: y={:.3} should be > 0.85",
            dir.y()
        );
    }

    #[test]
    fn september_equinox_sun_near_minus_x() {
        // At September equinox (~2024-09-22), sun is near -X direction (RA ≈ 180°)
        let epoch = Epoch::from_gregorian(2024, 9, 22, 12, 44, 0.0);
        let dir = sun_direction_eci(&epoch.to_tdb());

        // X should be dominant and negative
        assert!(
            dir.x() < -0.9,
            "September equinox: x={:.3} should be < -0.9",
            dir.x()
        );
        // Y and Z should be small
        assert!(
            dir.y().abs() < 0.2,
            "September equinox: y={:.3} should be near 0",
            dir.y()
        );
        assert!(
            dir.z().abs() < 0.1,
            "September equinox: z={:.3} should be near 0",
            dir.z()
        );
    }

    #[test]
    fn december_solstice_sun_negative_z() {
        // At December solstice (~2024-12-21), sun has significant -Z (southern declination ~-23.4°)
        let epoch = Epoch::from_gregorian(2024, 12, 21, 9, 21, 0.0);
        let dir = sun_direction_eci(&epoch.to_tdb());

        // Z should be negative and near -sin(23.44°) ≈ -0.398
        assert!(
            dir.z() < -0.35,
            "December solstice: z={:.3} should be < -0.35",
            dir.z()
        );
        // Y should be negative (RA ≈ 270°)
        assert!(
            dir.y() < -0.85,
            "December solstice: y={:.3} should be < -0.85",
            dir.y()
        );
    }

    #[test]
    fn sun_direction_varies_over_year() {
        // Verify the sun position actually changes throughout the year
        let epoch1 = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let epoch2 = Epoch::from_gregorian(2024, 7, 1, 12, 0, 0.0);
        let dir1 = sun_direction_eci(&epoch1.to_tdb());
        let dir2 = sun_direction_eci(&epoch2.to_tdb());

        // Should be significantly different (roughly opposite)
        let dot = dir1.dot(&dir2);
        assert!(
            dot < 0.0,
            "Jan vs Jul sun directions should be roughly opposite, dot={dot:.3}"
        );
    }

    // Sun distance tests

    #[test]
    fn sun_distance_approximately_1au() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let d = sun_distance_km(&epoch.to_tdb());
        let d_au = d / AU_KM;
        assert!(
            (d_au - 1.0).abs() < 0.02,
            "Sun distance should be ~1 AU, got {d_au:.4} AU"
        );
    }

    #[test]
    fn perihelion_closer_than_aphelion() {
        // Perihelion ~Jan 3, Aphelion ~Jul 4
        let perihelion = Epoch::from_gregorian(2024, 1, 3, 12, 0, 0.0);
        let aphelion = Epoch::from_gregorian(2024, 7, 5, 12, 0, 0.0);

        let d_peri = sun_distance_km(&perihelion.to_tdb());
        let d_aph = sun_distance_km(&aphelion.to_tdb());

        assert!(
            d_peri < d_aph,
            "Perihelion ({d_peri:.0} km) should be closer than aphelion ({d_aph:.0} km)"
        );
        // Eccentricity ~0.0167, so difference should be ~3.3%
        let ratio = d_aph / d_peri;
        assert!(
            (ratio - 1.034).abs() < 0.01,
            "Aphelion/perihelion ratio should be ~1.034, got {ratio:.4}"
        );
    }

    #[test]
    fn sun_position_magnitude_matches_distance() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0).to_tdb();
        let pos = sun_position_eci(&epoch);
        let dist = sun_distance_km(&epoch);

        let rel_err = (pos.magnitude() - dist).abs() / dist;
        assert!(
            rel_err < 1e-10,
            "Position magnitude should match distance, rel_err={rel_err:.6e}"
        );
    }

    // sun_direction_from_body tests

    #[test]
    fn sun_direction_from_body_earth_matches_eci() {
        let dates = [
            Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 9, 22, 12, 0, 0.0),
        ];
        for epoch in &dates {
            let from_body = sun_direction_from_body("earth", &epoch.to_tdb());
            let eci = sun_direction_eci(&epoch.to_tdb());
            let diff = (from_body - eci).magnitude();
            assert!(
                diff < 1e-10,
                "earth should match sun_direction_eci, diff={diff:.2e}"
            );
        }
    }

    #[test]
    fn sun_direction_from_body_moon_matches_eci() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let from_body = sun_direction_from_body("moon", &epoch.to_tdb());
        let eci = sun_direction_eci(&epoch.to_tdb());
        let diff = (from_body - eci).magnitude();
        assert!(
            diff < 1e-10,
            "moon should match sun_direction_eci, diff={diff:.2e}"
        );
    }

    #[test]
    fn sun_direction_from_body_mars_is_unit_vector() {
        let dates = [
            Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 12, 1, 12, 0, 0.0),
        ];
        for epoch in &dates {
            let dir = sun_direction_from_body("mars", &epoch.to_tdb());
            let norm = dir.magnitude();
            assert!(
                (norm - 1.0).abs() < 1e-10,
                "Mars sun direction should be unit vector, norm={norm}"
            );
        }
    }

    #[test]
    fn sun_direction_from_body_mars_varies() {
        let epoch1 = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let epoch2 = Epoch::from_gregorian(2024, 7, 1, 12, 0, 0.0);
        let dir1 = sun_direction_from_body("mars", &epoch1.to_tdb());
        let dir2 = sun_direction_from_body("mars", &epoch2.to_tdb());
        let dot = dir1.dot(&dir2);
        assert!(
            dot < 0.9,
            "Mars sun direction should change significantly over 6 months, dot={dot:.3}"
        );
    }

    #[test]
    fn sun_direction_from_body_unknown_fallback() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let dir = sun_direction_from_body("pluto", &epoch.to_tdb());
        assert!(
            (dir.x() - 1.0).abs() < 1e-10 && dir.y().abs() < 1e-10 && dir.z().abs() < 1e-10,
            "Unknown body should return +X fallback, got ({}, {}, {})",
            dir.x(),
            dir.y(),
            dir.z()
        );
    }

    // sun_distance_from_body tests

    #[test]
    fn sun_distance_from_body_earth_matches() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let from_body = sun_distance_from_body("earth", &epoch.to_tdb());
        let direct = sun_distance_km(&epoch.to_tdb());
        assert!(
            (from_body - direct).abs() < 1.0,
            "earth distance should match sun_distance_km: {from_body} vs {direct}"
        );
    }

    #[test]
    fn sun_distance_from_body_mars() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let dist = sun_distance_from_body("mars", &epoch.to_tdb());
        let dist_au = dist / AU_KM;
        assert!(
            dist_au > 1.3 && dist_au < 1.7,
            "Mars-Sun distance should be 1.3-1.7 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn sun_distance_from_body_jupiter() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let dist = sun_distance_from_body("jupiter", &epoch.to_tdb());
        let dist_au = dist / AU_KM;
        assert!(
            dist_au > 4.5 && dist_au < 5.8,
            "Jupiter-Sun distance should be 4.5-5.8 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn sun_distance_from_body_unknown_fallback() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let dist = sun_distance_from_body("pluto", &epoch.to_tdb());
        let earth_dist = sun_distance_km(&epoch.to_tdb());
        assert!(
            (dist - earth_dist).abs() < 1.0,
            "Unknown body should fall back to Earth distance"
        );
    }

    #[test]
    fn sun_position_direction_matches() {
        let epoch = Epoch::from_gregorian(2024, 9, 22, 12, 0, 0.0);
        let pos = sun_position_eci(&epoch.to_tdb());
        let dir = sun_direction_eci(&epoch.to_tdb());

        let pos_dir = pos.normalize();
        let diff = (pos_dir - dir).magnitude();
        assert!(
            diff < 1e-10,
            "Position direction should match unit direction, diff={diff:.6e}"
        );
    }

    // sun_position_from_body tests

    /// Earth keeps the geocentric vector the Earth-only path already returns.
    #[test]
    fn sun_position_from_earth_matches_the_geocentric_vector() {
        for (y, m, d) in [(2000, 1, 1), (2026, 6, 15), (2075, 12, 31)] {
            let epoch = Epoch::from_gregorian(y, m, d, 0, 0, 0.0).to_tdb();
            let from_body =
                sun_position_from_body(KnownBody::Earth, &epoch).expect("Earth is supported");
            let geocentric = sun_position_eci(&epoch);
            let diff = (from_body.into_inner() - geocentric.into_inner()).magnitude();
            assert!(diff < 1e-9, "{y}-{m:02}-{d:02}: {diff} km apart");
        }
    }

    /// From the Moon, the Sun is Earth's vector minus the Earth-to-Moon vector.
    ///
    /// Not Earth's vector: the difference is the lunar distance, about 0.15° of
    /// parallax, which the `&str` helper drops.
    #[test]
    fn sun_position_from_the_moon_subtracts_the_lunar_offset() {
        let epoch = Epoch::from_gregorian(2026, 6, 15, 0, 0, 0.0).to_tdb();
        let from_body =
            sun_position_from_body(KnownBody::Moon, &epoch).expect("the Moon is supported");

        let expected = sun_position_eci(&epoch).into_inner()
            - crate::moon::moon_position_eci(&epoch).into_inner();
        assert!(
            (from_body.into_inner() - expected).magnitude() < 1e-9,
            "should be Earth->Sun minus Earth->Moon"
        );

        // And it is distinguishable from the geocentric vector: the offset is
        // the Earth-Moon distance, ~384400 km.
        let offset = (from_body.into_inner() - sun_position_eci(&epoch).into_inner()).magnitude();
        assert!(
            offset > 300_000.0,
            "the lunar offset should be there, got {offset} km"
        );
    }

    /// A planet's own distance to the Sun, not Earth's.
    ///
    /// The bound is each planet's aphelion/perihelion range, so a vector that
    /// had silently stayed geocentric (1 AU) fails for every one of them.
    #[test]
    fn sun_position_from_a_planet_uses_that_planet_s_orbit() {
        let epoch = Epoch::from_gregorian(2026, 1, 1, 0, 0, 0.0).to_tdb();
        // (body, perihelion AU, aphelion AU)
        for (body, lo, hi) in [
            (KnownBody::Mercury, 0.30, 0.48),
            (KnownBody::Venus, 0.71, 0.74),
            (KnownBody::Mars, 1.37, 1.68),
            (KnownBody::Jupiter, 4.94, 5.47),
            (KnownBody::Saturn, 9.00, 10.13),
        ] {
            let pos = sun_position_from_body(body, &epoch).expect("supported planet");
            let au = pos.into_inner().magnitude() / AU_KM;
            assert!(
                au > lo && au < hi,
                "{}: {au:.3} AU is outside [{lo}, {hi}]",
                body.properties().name
            );
        }
    }

    /// The position agrees with the direction and distance helpers it replaces.
    #[test]
    fn sun_position_from_a_planet_agrees_with_the_direction_and_distance() {
        let epoch = Epoch::from_gregorian(2026, 1, 1, 0, 0, 0.0).to_tdb();
        let pos = sun_position_from_body(KnownBody::Mars, &epoch).expect("Mars is supported");
        let dir = sun_direction_from_body("mars", &epoch);
        let dist = sun_distance_from_body("mars", &epoch);

        let expected = dir.into_inner() * dist;
        let rel = (pos.into_inner() - expected).magnitude() / dist;
        assert!(rel < 1e-12, "relative difference {rel}");
    }

    /// Uranus and Neptune have no elements here, so they are refused rather
    /// than answered with a direction nothing computed — which is what the
    /// `&str` helper does, falling back to `+X`.
    #[test]
    fn sun_position_from_an_unsupported_planet_is_an_error() {
        let epoch = Epoch::from_gregorian(2026, 1, 1, 0, 0, 0.0).to_tdb();
        for body in [KnownBody::Uranus, KnownBody::Neptune] {
            assert_eq!(
                sun_position_from_body(body, &epoch),
                Err(SunPositionError::UnsupportedBody(body)),
                "{} should be refused",
                body.properties().name
            );
        }
        // What the fallback did instead: a unit vector along +X.
        let fallback = sun_direction_from_body("uranus", &epoch).into_inner();
        assert_eq!(fallback, nalgebra::Vector3::new(1.0, 0.0, 0.0));
    }

    /// The Sun has no position relative to itself.
    #[test]
    fn sun_position_from_the_sun_is_an_error() {
        let epoch = Epoch::from_gregorian(2026, 1, 1, 0, 0, 0.0).to_tdb();
        assert_eq!(
            sun_position_from_body(KnownBody::Sun, &epoch),
            Err(SunPositionError::CentralBodyIsSun)
        );
    }
}
