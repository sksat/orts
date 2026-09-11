//! Which bodies block the Sun, and what that leaves of it.
//!
//! [`arika::eclipse`] answers for one occulter at a time. A spacecraft can have
//! more than one: a lunar orbiter is shadowed by the Moon it orbits and, a few
//! times a year, by the Earth. This module holds the list, evaluates each
//! occulter's ephemeris, and combines the results into the one illumination
//! fraction a force model or a sensor asks for.

use std::sync::Arc;

use arika::body::KnownBody;
use arika::earth::transform::EphemerisFrameBridge;
use arika::eclipse::{self, SUN_RADIUS_KM, ShadowModel};
use arika::epoch::{Epoch, Tdb};
use arika::frame::Vec3;
use nalgebra::Vector3;

use crate::perturbations::BodyPositionFn;

/// A body that can block the Sun: where it is, how big it is, and which shadow
/// geometry to use for it.
///
/// The position is a closure for the same reason
/// [`ThirdBodyGravity`](crate::perturbations::ThirdBodyGravity) takes one: the
/// model has to be `Send + Sync`, and a substituted ephemeris has to fit.
#[derive(Clone)]
pub struct OccultingBody {
    /// Where this body is relative to the central body [km], in GCRS axes.
    ///
    /// The central body sits at the origin, so its own closure returns zero.
    position_fn: BodyPositionFn,
    /// Radius [km].
    pub radius: f64,
    /// Shadow geometry for this body.
    ///
    /// Per body rather than per model: a cylindrical shadow ignores the
    /// penumbra, which is a 0.5% effect for the body a spacecraft orbits and a
    /// factor of 1.86 in eclipse duration for one as far away as the Earth is
    /// from a lunar orbit.
    pub shadow_model: ShadowModel,
}

impl OccultingBody {
    /// The central body, at the origin of the propagation frame.
    ///
    /// # Panics
    /// Panics unless the radius is finite and positive. A zero radius hides
    /// nothing and a negative one is not geometry; either would pass silently
    /// into the eclipse test, which answers "sunlit" for both and so would give
    /// a spacecraft no shadow at all.
    pub fn central(radius: f64, shadow_model: ShadowModel) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0,
            "an occulting body needs a finite positive radius, got {radius}"
        );
        Self {
            position_fn: Arc::new(|_| Vec3::from_raw(Vector3::zeros())),
            radius,
            shadow_model,
        }
    }

    /// A body whose position comes from the given ephemeris, relative to the
    /// central body [km] in GCRS axes.
    ///
    /// For an occulter this module does not name: another moon of the body
    /// being orbited, or a substituted ephemeris for one it does.
    ///
    /// # Panics
    /// Panics unless the radius is finite and positive, as
    /// [`central`](Self::central) does.
    pub fn from_ephemeris(
        position_fn: BodyPositionFn,
        radius: f64,
        shadow_model: ShadowModel,
    ) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0,
            "an occulting body needs a finite positive radius, got {radius}"
        );
        Self {
            position_fn,
            radius,
            shadow_model,
        }
    }

    /// The Earth, seen from a Moon-centred frame.
    ///
    /// Conical: from a lunar orbit the Earth is far enough that a cylindrical
    /// shadow calls 6.83 hours a year dark where the conical one calls 3.67.
    pub fn earth_from_moon() -> Self {
        Self {
            position_fn: Arc::new(|epoch: &Epoch<Tdb>| {
                Vec3::from_raw(-*arika::moon::moon_position_eci(epoch).inner())
            }),
            radius: arika::earth::R,
            shadow_model: ShadowModel::Conical,
        }
    }

    /// This body's position in the propagation frame `F` [km].
    fn position_in<F: EphemerisFrameBridge>(&self, epoch: &Epoch) -> Vector3<f64> {
        let gcrs = (self.position_fn)(&epoch.to_tdb());
        *F::ephemeris_rotation(epoch).transform(&gcrs).inner()
    }
}

/// The bodies that block the Sun for a spacecraft orbiting `central`.
///
/// The central body itself, and the Earth for a lunar orbiter: measured over
/// 2026, a 100 km lunar orbit spends 4.0–7.7 hours a year inside the Earth's
/// shadow, in runs of up to 257 minutes, on the dates of the lunar eclipses.
/// The mirror case is not worth carrying — an Earth orbiter spends 0.2 hours a
/// year in the Moon's penumbra and none in its umbra, a five-hundredth of the
/// effect.
///
/// `central_model` is the caller's, because the models disagree on what the
/// central body deserves: the two SRP models take a cylindrical shadow and the
/// sun sensor a conical one, and this function is not the place to change
/// either.
///
/// The Sun occults nothing, so orbiting it gives an empty list.
pub fn default_occulters(central: KnownBody, central_model: ShadowModel) -> Vec<OccultingBody> {
    if central == KnownBody::Sun {
        return Vec::new();
    }
    let mut occulters = vec![OccultingBody::central(
        central.properties().radius,
        central_model,
    )];
    if central == KnownBody::Moon {
        occulters.push(OccultingBody::earth_from_moon());
    }
    occulters
}

/// What one occulter leaves of the Sun, and where it sits on the sky.
#[derive(Clone, Copy)]
struct Seen {
    /// The fraction of the Sun's disc this body hides, in [0, 1].
    obscured: f64,
    /// Unit vector from the observer toward the body.
    direction: Vector3<f64>,
    /// Apparent angular radius of the body [rad].
    angular_radius: f64,
}

/// The fraction of the Sun's light reaching `observer`, with every body in
/// `occulters` in the way.
///
/// Positions are in the propagation frame `F`; each occulter's own position
/// comes from its ephemeris and is rotated into `F` the way the Sun's is.
pub fn illumination<F: EphemerisFrameBridge>(
    occulters: &[OccultingBody],
    observer: &Vector3<f64>,
    sun: &Vector3<f64>,
    epoch: &Epoch,
) -> f64 {
    // The two lists a run actually carries take no allocation: nothing in the
    // way, or one body in the way, which is every Earth orbit. This is called
    // once per force evaluation, so an integrator stage that had no allocator
    // traffic keeps having none.
    match occulters {
        [] => 1.0,
        [only] => match seen_by::<F>(only, observer, sun, epoch) {
            Some(seen) => (1.0 - seen.obscured).clamp(0.0, 1.0),
            None => 1.0,
        },
        many => {
            let seen: Vec<Seen> = many
                .iter()
                .filter_map(|body| seen_by::<F>(body, observer, sun, epoch))
                .collect();
            (1.0 - obscured_fraction(&seen)).clamp(0.0, 1.0)
        }
    }
}

/// What one body hides of the Sun, and where it sits, or `None` if it hides
/// nothing.
fn seen_by<F: EphemerisFrameBridge>(
    body: &OccultingBody,
    observer: &Vector3<f64>,
    sun: &Vector3<f64>,
    epoch: &Epoch,
) -> Option<Seen> {
    let position = body.position_in::<F>(epoch);
    let to_body = position - observer;
    let distance = to_body.magnitude();
    // A body the observer sits inside, or on, has no direction to speak of.
    if distance < 1e-10 {
        return None;
    }
    // A body farther away than the Sun is behind it and hides nothing, however
    // its disc lies on the sky. `arika` rejects a body behind the observer but
    // not one beyond the light, and a caller's own ephemeris can put one there.
    if distance - body.radius >= (sun - observer).magnitude() {
        return None;
    }
    let illum = eclipse::illumination(
        observer,
        sun,
        &position,
        SUN_RADIUS_KM,
        body.radius,
        body.shadow_model,
    );
    if illum >= 1.0 {
        return None;
    }
    Some(Seen {
        obscured: 1.0 - illum,
        direction: to_body / distance,
        angular_radius: (body.radius / distance).clamp(-1.0, 1.0).asin(),
    })
}

/// How much of the Sun's disc a set of bodies hides between them.
///
/// Three cases, decided by where the bodies' own discs sit relative to each
/// other:
///
/// - **Clear of each other**: they hide different parts of the Sun, so their
///   fractions add. Exact.
/// - **One inside the other**: the nearer body already hides that part of the
///   sky, the far body included, so the larger fraction stands. Exact.
/// - **Overlapping without either containing the other**: they hide a shared
///   part of the Sun and parts of their own. The exact answer is the area of a
///   union of two circles inside a third, which this does not compute; taking
///   `a + b - ab` instead lands between the largest fraction and the sum of
///   them, and it cannot turn two partial eclipses into a total one — totality
///   still needs one body to produce it alone.
///
/// The last case is reachable with the set [`default_occulters`] gives a lunar
/// orbiter, and only there: during a lunar eclipse the Sun sits behind the
/// Earth, so when the spacecraft crosses the Moon's terminator the Earth's
/// small disc straddles the Moon's limb. It lasts as long as that crossing.
fn obscured_fraction(seen: &[Seen]) -> f64 {
    // Widest disc first: a body's disc can only lie inside a wider one, so
    // this order is what lets the containment test below look at bodies
    // already counted and stop there.
    let mut order: Vec<usize> = (0..seen.len()).collect();
    order.sort_by(|&a, &b| {
        seen[b]
            .angular_radius
            .partial_cmp(&seen[a].angular_radius)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Bodies whose discs meet, directly or through another, form a group. What
    // a group hides is folded together; groups hide different parts of the Sun,
    // so the groups add. Both have to be per group: folding against everything
    // counted so far would discount a body against one it does not meet.
    let mut group_of: Vec<usize> = (0..seen.len()).collect();
    for (a, &i) in order.iter().enumerate() {
        for &j in &order[..a] {
            if seen[i].against(&seen[j]) != Relation::Clear {
                let (root_i, root_j) = (root(&mut group_of, i), root(&mut group_of, j));
                if root_i != root_j {
                    group_of[root_i] = root_j;
                }
            }
        }
    }

    let mut folded: std::collections::HashMap<usize, (f64, Vec<usize>)> =
        std::collections::HashMap::new();
    for &i in &order {
        let key = root(&mut group_of, i);
        let (obscured, taken) = folded.entry(key).or_insert((0.0, Vec::new()));
        if taken
            .iter()
            .any(|&j| seen[i].against(&seen[j]) == Relation::Inside)
        {
            // A wider body already counted covers this one whole, so this body
            // hides nothing the group does not already hide — unless it reports
            // more than the group does, which a cylindrical shadow inside a
            // conical one can: the first is total or nothing, the second leaves
            // a ring. Keeping the larger reading is what stops a configured
            // total eclipse from coming out partial.
            *obscured = obscured.max(seen[i].obscured);
            continue;
        }
        taken.push(i);
        // Shares part of the Sun with the rest of its group: between the
        // largest fraction in the group and their sum, and never total unless
        // one body is.
        *obscured = *obscured + seen[i].obscured - *obscured * seen[i].obscured;
    }
    folded.values().map(|(obscured, _)| obscured).sum()
}

/// The representative of `i`'s group.
fn root(group_of: &mut [usize], mut i: usize) -> usize {
    while group_of[i] != i {
        i = group_of[i];
    }
    i
}

/// Where one body's disc sits relative to another's, as seen from the observer.
#[derive(Debug, PartialEq)]
enum Relation {
    /// The discs do not meet: the two hide different parts of the Sun.
    Clear,
    /// This body's disc lies inside the other's: the other hides it too.
    Inside,
    /// The discs meet, with neither inside the other.
    Overlapping,
}

impl Seen {
    fn against(&self, other: &Seen) -> Relation {
        let separation = self.direction.dot(&other.direction).clamp(-1.0, 1.0).acos();
        if separation >= self.angular_radius + other.angular_radius {
            Relation::Clear
        } else if separation <= other.angular_radius - self.angular_radius {
            Relation::Inside
        } else {
            Relation::Overlapping
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::frame::SimpleEci;

    /// One body's fraction stands on its own, and an empty sky leaves the Sun
    /// whole.
    #[test]
    fn one_body_hides_its_own_fraction_and_no_body_hides_none() {
        assert_eq!(obscured_fraction(&[]), 0.0);
        assert_eq!(
            obscured_fraction(&[Seen {
                obscured: 0.3,
                direction: Vector3::x(),
                angular_radius: 0.1,
            }]),
            0.3
        );
    }

    /// Two bodies far apart on the sky hide different parts of the Sun, so what
    /// they hide adds. This is the case a second occulter is there for.
    #[test]
    fn bodies_apart_on_the_sky_add_what_they_hide() {
        let seen = [
            Seen {
                obscured: 0.25,
                direction: Vector3::x(),
                angular_radius: 0.01,
            },
            Seen {
                obscured: 0.4,
                direction: Vector3::y(),
                angular_radius: 0.01,
            },
        ];
        assert!((obscured_fraction(&seen) - 0.65).abs() < 1e-15);
    }

    /// Two bodies whose discs meet hide a shared part of the Sun and parts of
    /// their own. Adding 0.6 and 0.6 would read as a total eclipse, which
    /// neither produces and their union does not either; taking only the larger
    /// would drop what the second hides on its own.
    #[test]
    fn bodies_whose_discs_meet_do_not_add_up_to_a_total_eclipse() {
        let seen = [
            Seen {
                obscured: 0.6,
                direction: Vector3::x(),
                angular_radius: 0.2,
            },
            Seen {
                obscured: 0.6,
                direction: (Vector3::x() + Vector3::y() * 0.1).normalize(),
                angular_radius: 0.2,
            },
        ];
        let obscured = obscured_fraction(&seen);
        assert!(
            (obscured - (0.6 + 0.6 - 0.36)).abs() < 1e-15,
            "overlapping discs hide more than the larger alone, got {obscured}"
        );
        assert!(
            obscured < 1.0,
            "two partial eclipses must not make a total one"
        );
    }

    /// A body behind another — its disc inside the other's — adds nothing: the
    /// nearer body already hides that part of the sky, the far body included.
    #[test]
    fn a_body_behind_another_adds_nothing() {
        let seen = [
            Seen {
                obscured: 0.9,
                direction: Vector3::x(),
                angular_radius: 0.5,
            },
            Seen {
                obscured: 0.2,
                direction: (Vector3::x() + Vector3::y() * 0.05).normalize(),
                angular_radius: 0.01,
            },
        ];
        assert!((obscured_fraction(&seen) - 0.9).abs() < 1e-15);
    }

    /// A body inside another's disc can still report the stronger eclipse: a
    /// cylindrical shadow is total or nothing, where a conical one the same
    /// size leaves a ring. Discarding the contained body would turn a
    /// configured total eclipse into partial sunlight.
    #[test]
    fn a_contained_body_keeps_the_stronger_reading() {
        let seen = [
            Seen {
                obscured: 0.3,
                direction: Vector3::x(),
                angular_radius: 0.5,
            },
            Seen {
                obscured: 1.0,
                direction: (Vector3::x() + Vector3::y() * 0.05).normalize(),
                angular_radius: 0.02,
            },
        ];
        assert_eq!(
            obscured_fraction(&seen),
            1.0,
            "the contained body reports a total eclipse and the group has to keep it"
        );
    }

    /// A total eclipse by one body leaves nothing whatever else is in the sky.
    #[test]
    fn a_total_eclipse_by_one_body_leaves_nothing() {
        let seen = [
            Seen {
                obscured: 1.0,
                direction: Vector3::x(),
                angular_radius: 0.3,
            },
            Seen {
                obscured: 0.5,
                direction: Vector3::y(),
                angular_radius: 0.01,
            },
        ];
        assert!(obscured_fraction(&seen) >= 1.0);
    }

    /// A body clear of an overlapping pair adds its own fraction, whichever
    /// order the list arrives in. Folding against everything counted so far
    /// discounts it against a body it does not meet: 0.2 before two
    /// overlapping 0.3s would read as 0.65 instead of 0.71.
    #[test]
    fn a_clear_body_adds_whatever_order_it_comes_in() {
        let overlapping = |x: f64| Seen {
            obscured: 0.3,
            direction: (Vector3::x() + Vector3::y() * x).normalize(),
            angular_radius: 0.1,
        };
        let clear = Seen {
            obscured: 0.2,
            direction: -Vector3::x(),
            angular_radius: 0.01,
        };
        let want = 0.2 + (0.3 + 0.3 - 0.09);
        for seen in [
            vec![clear, overlapping(0.0), overlapping(0.15)],
            vec![overlapping(0.0), overlapping(0.15), clear],
            vec![overlapping(0.0), clear, overlapping(0.15)],
        ] {
            let obscured = obscured_fraction(&seen);
            assert!(
                (obscured - want).abs() < 1e-15,
                "expected {want}, got {obscured}"
            );
        }
    }

    /// Three bodies, two of them meeting: the pair combines and the third,
    /// clear of both, adds.
    #[test]
    fn touching_discs_form_one_group_however_many_there_are() {
        let chain = |x: f64| Seen {
            obscured: 0.3,
            direction: (Vector3::x() + Vector3::y() * x).normalize(),
            angular_radius: 0.1,
        };
        let seen = [
            chain(0.0),
            chain(0.15),
            Seen {
                obscured: 0.2,
                direction: -Vector3::x(),
                angular_radius: 0.01,
            },
        ];
        let obscured = obscured_fraction(&seen);
        let pair = 0.3 + 0.3 - 0.09;
        assert!(
            (obscured - (pair + 0.2)).abs() < 1e-15,
            "the touching pair combines ({pair}) and the far body adds (0.2), got {obscured}"
        );
    }

    /// The central body still eclipses what it did: a satellite directly behind
    /// the Earth is dark, and one on the sunlit side is not.
    #[test]
    fn the_central_body_alone_still_eclipses() {
        let occulters = vec![OccultingBody::central(
            arika::earth::R,
            ShadowModel::Cylindrical,
        )];
        let epoch = Epoch::j2000();
        let sun = Vector3::new(arika::sun::AU_KM, 0.0, 0.0);
        let behind = Vector3::new(-7000.0, 0.0, 0.0);
        let in_front = Vector3::new(7000.0, 0.0, 0.0);

        assert_eq!(
            illumination::<SimpleEci>(&occulters, &behind, &sun, &epoch),
            0.0
        );
        assert_eq!(
            illumination::<SimpleEci>(&occulters, &in_front, &sun, &epoch),
            1.0
        );
    }

    /// A lunar orbiter inside the Earth's shadow is dark, and the central-body
    /// list alone calls it sunlit — the gap this module closes.
    ///
    /// 2026-03-03T11:30 is a total lunar eclipse: the Moon, and anything in a
    /// low orbit around it, is inside the Earth's umbra. Orekit 13.1.7 at this
    /// epoch reports a lighting ratio of 1.000 with the Moon as the only
    /// occulter and 0.000 with the Earth added.
    #[test]
    fn a_lunar_orbiter_in_the_earths_shadow_is_dark() {
        let epoch = Epoch::from_iso8601("2026-03-03T11:30:00Z").expect("a valid epoch");
        let sun_from_moon = *arika::sun::sun_position_from_body(KnownBody::Moon, &epoch.to_tdb())
            .expect("the Moon has a Sun ephemeris")
            .inner();
        // 100 km up, on the far side of the Moon from the Sun: the Moon itself
        // is not in the way, so only the Earth can darken this.
        let radius = KnownBody::Moon.properties().radius + 100.0;
        let satellite = sun_from_moon.normalize() * radius;

        let moon_only = vec![OccultingBody::central(
            KnownBody::Moon.properties().radius,
            ShadowModel::Cylindrical,
        )];
        assert_eq!(
            illumination::<SimpleEci>(&moon_only, &satellite, &sun_from_moon, &epoch),
            1.0,
            "the Moon alone leaves this position sunlit"
        );

        let with_earth = default_occulters(KnownBody::Moon, ShadowModel::Cylindrical);
        assert_eq!(with_earth.len(), 2, "the Moon's set carries the Earth");
        let illum = illumination::<SimpleEci>(&with_earth, &satellite, &sun_from_moon, &epoch);
        assert_eq!(
            illum, 0.0,
            "the Earth's umbra covers a lunar orbit at a total lunar eclipse"
        );
    }

    /// The occulter's own position is rotated into the propagation frame, as
    /// the Sun's is. A frame test that only moves the Sun cannot catch a
    /// forgotten rotation on the Earth's Moon-relative vector, so this one
    /// compares the two frames at a geometry where the rotation matters.
    #[test]
    fn a_noncentral_occulter_is_rotated_into_the_propagation_frame() {
        use arika::frame::Cirs;

        // Far enough from J2000 for precession to have turned the frames
        // apart: 2026 is 26 years of about 20 arcseconds a year.
        let epoch = Epoch::from_iso8601("2026-03-03T09:00:00Z").expect("a valid epoch");
        let sun_from_moon = *arika::sun::sun_position_from_body(KnownBody::Moon, &epoch.to_tdb())
            .expect("the Moon has a Sun ephemeris")
            .inner();
        let earth = OccultingBody::earth_from_moon();

        let gcrs = earth.position_in::<arika::frame::Gcrs>(&epoch);
        let cirs = earth.position_in::<Cirs>(&epoch);
        let turned = (gcrs - cirs).magnitude();
        assert!(
            turned > 1.0,
            "the frames should differ by more than a kilometre here, got {turned:.3} km"
        );
        // Same distance, different axes: a rotation and nothing else.
        assert!((gcrs.magnitude() - cirs.magnitude()).abs() < 1e-6);
        // And the Sun is far enough away that the illumination is unaffected by
        // which of the two frames the pair is expressed in.
        let occulters = vec![earth];
        let satellite = sun_from_moon.normalize() * (KnownBody::Moon.properties().radius + 100.0);
        let a = illumination::<arika::frame::Gcrs>(&occulters, &satellite, &sun_from_moon, &epoch);
        let b = illumination::<Cirs>(&occulters, &satellite, &sun_from_moon, &epoch);
        assert!(
            (a - b).abs() < 1e-6,
            "the same geometry in two frames: {a} against {b}"
        );
    }

    /// A radius that is not geometry would give a spacecraft no shadow rather
    /// than a wrong one, which is the harder failure to notice.
    #[test]
    #[should_panic(expected = "finite positive radius")]
    fn a_body_of_no_radius_is_refused() {
        OccultingBody::central(0.0, ShadowModel::Cylindrical);
    }

    #[test]
    #[should_panic(expected = "finite positive radius")]
    fn a_body_of_infinite_radius_is_refused() {
        OccultingBody::central(f64::INFINITY, ShadowModel::Conical);
    }

    /// A caller can bring an occulter this module does not name, which is what
    /// the position closure is for.
    #[test]
    fn a_body_can_come_from_a_callers_own_ephemeris() {
        // Big enough on the sky to cover the Sun: 20000 km at 500000 km is an
        // apparent radius of 2.3 degrees against the Sun's 0.27, where 2000 km
        // would leave an annular ring.
        let fixed = Vector3::new(0.0, 500_000.0, 0.0);
        let occulter = OccultingBody::from_ephemeris(
            Arc::new(move |_| Vec3::from_raw(fixed)),
            20_000.0,
            ShadowModel::Conical,
        );
        let epoch = Epoch::j2000();
        // The Sun straight behind that body, seen from the origin.
        let sun = fixed.normalize() * arika::sun::AU_KM;
        assert_eq!(
            illumination::<SimpleEci>(&[occulter], &Vector3::zeros(), &sun, &epoch),
            0.0,
            "a caller's own body blocks the Sun like any other"
        );
    }

    /// Orbiting the Sun there is nothing to cast a shadow, and every other body
    /// carries at least itself.
    #[test]
    fn the_default_set_follows_the_central_body() {
        assert!(default_occulters(KnownBody::Sun, ShadowModel::Conical).is_empty());
        assert_eq!(
            default_occulters(KnownBody::Earth, ShadowModel::Cylindrical).len(),
            1,
            "an Earth orbiter carries the Earth alone: the Moon is worth a five-hundredth"
        );
        assert_eq!(
            default_occulters(KnownBody::Mars, ShadowModel::Conical).len(),
            1
        );
        let moon = default_occulters(KnownBody::Moon, ShadowModel::Cylindrical);
        assert_eq!(moon.len(), 2);
        assert_eq!(
            moon[0].shadow_model,
            ShadowModel::Cylindrical,
            "the central body takes the caller's model"
        );
        assert_eq!(
            moon[1].shadow_model,
            ShadowModel::Conical,
            "a distant occulter does not inherit it"
        );
    }

    /// A body beyond the Sun hides nothing, whatever its disc does on the sky.
    ///
    /// `arika` rejects an occulter behind the observer but not one behind the
    /// light, and a caller's own ephemeris can put one there.
    #[test]
    fn a_body_beyond_the_sun_hides_nothing() {
        let epoch = Epoch::j2000();
        let sun = Vector3::new(arika::sun::AU_KM, 0.0, 0.0);
        // Twice as far as the Sun, on the same line, and wide enough to cover
        // it several times over if it were in front.
        let far = Vector3::new(2.0 * arika::sun::AU_KM, 0.0, 0.0);
        let behind_the_sun = OccultingBody::from_ephemeris(
            Arc::new(move |_| Vec3::from_raw(far)),
            50.0 * SUN_RADIUS_KM,
            ShadowModel::Conical,
        );
        assert_eq!(
            illumination::<SimpleEci>(&[behind_the_sun], &Vector3::zeros(), &sun, &epoch),
            1.0
        );
    }
}
