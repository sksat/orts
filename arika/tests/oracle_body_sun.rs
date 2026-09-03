//! `sun_position_from_body` against Orekit's DE ephemeris.
//!
//! This vector decides where the Sun is for a spacecraft around any central
//! body, so it turns SRP and scales the third-body term. arika builds it from
//! three sources — the Meeus geocentric series for Earth, that series minus
//! the lunar one for the Moon, and the Standish heliocentric elements for the
//! planets, negated and rotated from the J2000 ecliptic to the equator — and
//! every other test on this path compares one of arika's models against
//! another. Orekit's DE data shares neither code nor series with either, so a
//! sign, phase or ecliptic-to-equatorial error shows up here.
//!
//! Fixture: `tools/generate_body_sun_fixtures.py`.

use arika::body::KnownBody;
use arika::epoch::Epoch;
use arika::sun::sun_position_from_body;

/// Per-body bounds on the angle [arcmin] and on the relative distance error.
///
/// The measured worst case across the fixture's six epochs is in the comment;
/// each bound sits just above it. The spread follows the Standish elements'
/// own accuracy, which degrades with the outer planets.
const BOUNDS: &[(&str, KnownBody, f64, f64)] = &[
    // measured: 0.395', 4.8e-5
    ("earth", KnownBody::Earth, 1.0, 1.0e-4),
    // measured: 0.396', 4.8e-5
    ("moon", KnownBody::Moon, 1.0, 1.0e-4),
    // measured: 0.256', 1.9e-5
    ("mercury", KnownBody::Mercury, 1.0, 1.0e-4),
    // measured: 0.315', 3.3e-5
    ("venus", KnownBody::Venus, 1.0, 1.0e-4),
    // measured: 1.163', 1.2e-4
    ("mars", KnownBody::Mars, 2.0, 3.0e-4),
    // measured: 6.501', 7.0e-4
    ("jupiter", KnownBody::Jupiter, 8.0, 1.0e-3),
    // measured: 16.924', 4.1e-3
    ("saturn", KnownBody::Saturn, 20.0, 5.0e-3),
];

/// The errors these bounds are set to catch, in arcminutes.
///
/// A bound is only worth having if it is well under the mistakes it stands
/// against: flipping the sign of the body-to-Sun vector, leaving the ecliptic
/// vector unrotated (the J2000 obliquity), or applying precession where the
/// elements already refer to J2000 (up to 1.4° by 2100).
const SIGN_FLIP_ARCMIN: f64 = 180.0 * 60.0;
const OBLIQUITY_ARCMIN: f64 = 23.439_291 * 60.0;
const PRECESSION_BY_2100_ARCMIN: f64 = 1.4 * 60.0;

fn body_of(name: &str) -> KnownBody {
    BOUNDS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, body, ..)| *body)
        .unwrap_or_else(|| panic!("fixture names a body this test does not map: {name}"))
}

fn bounds_of(body: KnownBody) -> (f64, f64) {
    BOUNDS
        .iter()
        .find(|(_, b, ..)| *b == body)
        .map(|(_, _, arcmin, rel)| (*arcmin, *rel))
        .expect("every mapped body has bounds")
}

#[test]
fn body_sun_vector_matches_orekit() {
    let raw = include_str!("fixtures/body_sun_orekit_reference.json");
    let fixture: serde_json::Value = serde_json::from_str(raw).expect("the fixture parses");
    assert_eq!(
        fixture["frame"], "EME2000",
        "the reference frame arika returns"
    );
    assert_eq!(fixture["unit"], "km", "arika's unit");
    let cases = fixture["cases"].as_array().expect("cases is an array");
    assert_eq!(cases.len(), 42, "7 bodies over 6 epochs");

    for case in cases {
        let name = case["body"].as_str().expect("body name");
        let body = body_of(name);
        let (arcmin_bound, rel_bound) = bounds_of(body);
        let e = case["epoch_utc"].as_array().expect("epoch fields");
        let epoch = Epoch::from_gregorian(
            e[0].as_i64().expect("year") as i32,
            e[1].as_u64().expect("month") as u32,
            e[2].as_u64().expect("day") as u32,
            e[3].as_u64().expect("hour") as u32,
            e[4].as_u64().expect("minute") as u32,
            e[5].as_f64().expect("second"),
        );
        let v = case["sun_from_body_km"].as_array().expect("vector");
        let want = nalgebra::Vector3::new(
            v[0].as_f64().expect("x"),
            v[1].as_f64().expect("y"),
            v[2].as_f64().expect("z"),
        );

        let got = sun_position_from_body(body, &epoch.to_tdb())
            .unwrap_or_else(|e| panic!("{name}: the fixture only lists supported bodies: {e:?}"))
            .into_inner();

        let arcmin = got
            .normalize()
            .dot(&want.normalize())
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
            * 60.0;
        assert!(
            arcmin < arcmin_bound,
            "{name} at {:?}: Sun direction {arcmin:.3}' from Orekit's, bound {arcmin_bound}'",
            &e[..3]
        );

        let rel = (got.magnitude() - want.magnitude()) / want.magnitude();
        assert!(
            rel.abs() < rel_bound,
            "{name} at {:?}: distance off by {rel:.3e}, bound {rel_bound:.0e} \
             ({:.0} km of {:.0} km)",
            &e[..3],
            (got.magnitude() - want.magnitude()).abs(),
            want.magnitude()
        );
    }
}

/// The bounds above are far below the errors they stand against.
///
/// Without this, widening a bound to quiet a failure could pass the loosest
/// body (Saturn, 20') while a real frame error hid under it.
#[test]
fn the_bounds_are_tighter_than_the_errors_they_catch() {
    let loosest = BOUNDS
        .iter()
        .map(|(_, _, arcmin, _)| *arcmin)
        .fold(0.0_f64, f64::max);
    for (label, error) in [
        ("a sign flip", SIGN_FLIP_ARCMIN),
        ("an unrotated ecliptic vector", OBLIQUITY_ARCMIN),
        ("precession applied twice", PRECESSION_BY_2100_ARCMIN),
    ] {
        assert!(
            loosest * 4.0 < error,
            "{label} is {error:.0}', only {:.1}x the loosest bound {loosest}'",
            error / loosest
        );
    }
}
