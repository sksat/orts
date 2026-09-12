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
    /// Radius [km]. Private: the constructors establish that it is finite and
    /// positive, and a body whose radius is zero or `NaN` would silently stop
    /// casting a shadow rather than fail.
    radius: f64,
    /// Shadow geometry for this body.
    ///
    /// Per body rather than per model: a cylindrical shadow ignores the
    /// penumbra, which is a 0.5% effect for the body a spacecraft orbits and a
    /// factor of 1.86 in eclipse duration for one as far away as the Earth is
    /// from a lunar orbit.
    pub shadow_model: ShadowModel,
    /// Whether this is the body at the origin of the propagation frame.
    ///
    /// The compatibility builders name a radius or a geometry without saying
    /// which body they mean, and what they have always meant is the central
    /// one. A distant occulter keeps the geometry it was built with, which for
    /// the Earth seen from a lunar orbit has to stay conical.
    central: bool,
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
            central: true,
        }
    }

    /// Radius [km].
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// Whether this is the body at the origin of the propagation frame.
    pub fn is_central(&self) -> bool {
        self.central
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
            central: false,
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
            central: false,
        }
    }

    /// This body's position in the propagation frame `F` [km].
    fn position_in<F: EphemerisFrameBridge>(&self, epoch: &Epoch) -> Vector3<f64> {
        if self.central {
            // The origin is the origin in every frame, and this is the body
            // every run carries: asking for the rotation would pay for the
            // precession and nutation again, next to the one the caller already
            // computed for the Sun.
            return Vector3::zeros();
        }
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
/// Every part of the geometry is in the propagation frame `F`: the observer and
/// the Sun arrive as `Vec3<F>`, and each occulter's own position comes from its
/// ephemeris and is rotated into `F` the way the Sun's is. Taking raw vectors
/// here would let a caller pass GCRS positions while `F` says `Cirs`, rotating
/// one part of the geometry and not the rest.
pub fn illumination<F: EphemerisFrameBridge>(
    occulters: &[OccultingBody],
    observer: &Vec3<F>,
    sun: &Vec3<F>,
    epoch: &Epoch,
) -> f64 {
    let observer = observer.inner();
    let sun = sun.inner();
    // The lists a run actually carries take no allocation: nothing in the way,
    // one body, which is every Earth orbit, or two, which is every lunar one.
    // This is called once per force evaluation, so an integrator stage that had
    // no allocator traffic keeps having none.
    match occulters {
        [] => 1.0,
        [only] => match seen_by::<F>(only, observer, sun, epoch) {
            Some(seen) => (1.0 - seen.obscured).clamp(0.0, 1.0),
            None => 1.0,
        },
        [first, second] => {
            let obscured = match (
                seen_by::<F>(first, observer, sun, epoch),
                seen_by::<F>(second, observer, sun, epoch),
            ) {
                (None, None) => 0.0,
                (Some(one), None) | (None, Some(one)) => one.obscured,
                (Some(a), Some(b)) => combine(&a, &b),
            };
            (1.0 - obscured).clamp(0.0, 1.0)
        }
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
/// Two bodies are read geometrically, by where their discs sit: see
/// [`combine`]. That is exact, and it covers every list this library builds —
/// a central body, and the Earth for a lunar orbiter.
///
/// Beyond two, bodies whose discs meet, directly or through another, form a
/// group. Groups hide different parts of the Sun, so what they hide adds, which
/// is exact. Within a group the shares are combined as `1 - Π(1 - aᵢ)`: each
/// body hides its share of what the others leave. That is symmetric, so no
/// ordering of the list can change it, and it is total only if one body is
/// total on its own. It is an approximation — three discs meeting each other
/// enclose an area no pairwise bookkeeping recovers, the exact answer being the
/// area of a union of circles inside the Sun's disc — but an approximation that
/// cannot depend on how the list was written. Reaching it takes a caller's own
/// list of three or more.
fn obscured_fraction(seen: &[Seen]) -> f64 {
    match seen {
        [] => 0.0,
        [only] => only.obscured,
        [a, b] => combine(a, b),
        many => {
            // Bodies that meet, directly or through another. `Relation::Clear`
            // is symmetric, so the partition does not depend on the order.
            let mut group_of: Vec<usize> = (0..many.len()).collect();
            for i in 0..many.len() {
                for j in (i + 1)..many.len() {
                    if many[i].against(&many[j]) != Relation::Clear {
                        let (a, b) = (root(&group_of, i), root(&group_of, j));
                        if a != b {
                            group_of[a] = b;
                        }
                    }
                }
            }

            // One factor per group, each the product of what its bodies leave.
            let mut left: Vec<(usize, f64)> = Vec::with_capacity(many.len());
            for (i, body) in many.iter().enumerate() {
                let key = root(&group_of, i);
                match left.iter_mut().find(|(group, _)| *group == key) {
                    Some((_, product)) => *product *= 1.0 - body.obscured,
                    None => left.push((key, 1.0 - body.obscured)),
                }
            }
            left.iter().map(|(_, product)| 1.0 - product).sum()
        }
    }
}

/// The representative of `i`'s group.
fn root(group_of: &[usize], mut i: usize) -> usize {
    while group_of[i] != i {
        i = group_of[i];
    }
    i
}

/// What two bodies hide between them, by where their discs sit.
///
/// The one rule both the two-body path and the general fold go through, so
/// there is one answer to the question rather than two.
fn combine(a: &Seen, b: &Seen) -> f64 {
    // `against` asks whether the receiver's disc is inside the other's, so
    // containment has to be asked both ways round: taking the caller's order
    // would read a wide body followed by a small one as merely overlapping,
    // and the answer would depend on how the list was written.
    if a.against(b) == Relation::Inside || b.against(a) == Relation::Inside {
        // One disc is inside the other, so the wider body hides it too —
        // unless the contained one reports more, which a cylindrical shadow
        // inside a conical one can: the first is total or nothing, the second
        // leaves a ring.
        return a.obscured.max(b.obscured);
    }
    match a.against(b) {
        // Different parts of the Sun: they add. Exact.
        Relation::Clear => a.obscured + b.obscured,
        // A shared part and parts of their own. The exact answer is the area of
        // a union of two circles inside a third, which this does not compute;
        // this lands between the larger fraction and the sum, and is total only
        // if one of them is.
        Relation::Inside | Relation::Overlapping => {
            a.obscured + b.obscured - a.obscured * b.obscured
        }
    }
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

    /// Bodies that meet are combined symmetrically, so the value cannot depend
    /// on the order the list was written in. A sequential fold did: three
    /// mutually overlapping discs of 0.8, 0.5 and 0.4 came out at 0.98 one way
    /// round and 1.1 the other — a total eclipse none of them produces.
    #[test]
    fn three_or_more_bodies_do_not_depend_on_their_order() {
        let at = |angle: f64, obscured: f64| Seen {
            obscured,
            direction: (Vector3::x() * angle.cos() + Vector3::y() * angle.sin()).normalize(),
            angular_radius: 0.2,
        };
        let (a, b, c) = (at(0.0, 0.8), at(0.1, 0.5), at(0.2, 0.4));
        // All three meet, so they are one group: the product of what each
        // leaves, and no ordering of it differs.
        let want = 1.0 - (1.0 - 0.8) * (1.0 - 0.5) * (1.0 - 0.4);
        for order in [[a, b, c], [c, b, a], [b, a, c], [c, a, b]] {
            let obscured = obscured_fraction(&order);
            assert!(
                (obscured - want).abs() < 1e-15,
                "expected {want}, got {obscured}"
            );
            assert!(obscured < 1.0, "no body here is total on its own");
        }
    }

    /// Three bodies leave nothing only if one of them hides the Sun whole: the
    /// symmetric rule has a factor of zero exactly there.
    #[test]
    fn three_bodies_are_total_only_when_one_of_them_is() {
        let at = |angle: f64, obscured: f64| Seen {
            obscured,
            direction: (Vector3::x() * angle.cos() + Vector3::y() * angle.sin()).normalize(),
            angular_radius: 0.2,
        };
        assert!(obscured_fraction(&[at(0.0, 0.9), at(0.1, 0.9), at(0.2, 0.9)]) < 1.0);
        assert_eq!(
            obscured_fraction(&[at(0.0, 0.9), at(0.1, 1.0), at(0.2, 0.9)]),
            1.0
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
            illumination::<SimpleEci>(
                &occulters,
                &Vec3::from_raw(behind),
                &Vec3::from_raw(sun),
                &epoch
            ),
            0.0
        );
        assert_eq!(
            illumination::<SimpleEci>(
                &occulters,
                &Vec3::from_raw(in_front),
                &Vec3::from_raw(sun),
                &epoch
            ),
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
        // 100 km up, on the sunward side of the Moon: the Moon itself is not in
        // the way there, so only the Earth can darken this.
        let radius = KnownBody::Moon.properties().radius + 100.0;
        let satellite = sun_from_moon.normalize() * radius;

        let moon_only = vec![OccultingBody::central(
            KnownBody::Moon.properties().radius,
            ShadowModel::Cylindrical,
        )];
        assert_eq!(
            illumination::<SimpleEci>(
                &moon_only,
                &Vec3::from_raw(satellite),
                &Vec3::from_raw(sun_from_moon),
                &epoch,
            ),
            1.0,
            "the Moon alone leaves this position sunlit"
        );

        let with_earth = default_occulters(KnownBody::Moon, ShadowModel::Cylindrical);
        assert_eq!(with_earth.len(), 2, "the Moon's set carries the Earth");
        let illum = illumination::<SimpleEci>(
            &with_earth,
            &Vec3::from_raw(satellite),
            &Vec3::from_raw(sun_from_moon),
            &epoch,
        );
        assert_eq!(
            illum, 0.0,
            "the Earth's umbra covers a lunar orbit at a total lunar eclipse"
        );
    }

    /// The occulter's own position is rotated into the propagation frame, as
    /// the Sun's is.
    ///
    /// The whole geometry moves together: the observer and the Sun arrive as
    /// `Vec3<F>` and the occulter is rotated into `F` from its ephemeris, so
    /// the same physical arrangement has to give the same illumination in two
    /// frames. Leaving the occulter in GCRS while the rest turned would not.
    ///
    /// The arrangement is a partial eclipse on purpose: a total one, or full
    /// sunlight, would give 0 or 1 in both frames whether or not the rotation
    /// was applied.
    #[test]
    fn the_whole_geometry_is_read_in_one_frame() {
        use arika::frame::{Cirs, Gcrs};

        // Far enough from J2000 for precession to have turned the frames
        // apart: 2026 is 26 years of about 20 arcseconds a year.
        let epoch = Epoch::from_iso8601("2026-03-03T09:00:00Z").expect("a valid epoch");
        let rotation = <Cirs as EphemerisFrameBridge>::ephemeris_rotation(&epoch);

        // Observer at the origin, Sun along +y, and a body between them whose
        // disc half-covers the Sun's: 2618 km at 500000 km is an apparent
        // radius of 0.3 degrees against the Sun's 0.267, offset by 0.4.
        let observer_gcrs = Vector3::zeros();
        let sun_gcrs = Vector3::new(0.0, arika::sun::AU_KM, 0.0);
        let offset = 0.4_f64.to_radians();
        let occulter_gcrs = Vector3::new(offset.sin(), offset.cos(), 0.0) * 500_000.0;
        let occulter = OccultingBody::from_ephemeris(
            Arc::new(move |_| Vec3::from_raw(occulter_gcrs)),
            2618.0,
            ShadowModel::Conical,
        );

        let in_gcrs = illumination::<Gcrs>(
            std::slice::from_ref(&occulter),
            &Vec3::from_raw(observer_gcrs),
            &Vec3::from_raw(sun_gcrs),
            &epoch,
        );
        assert!(
            in_gcrs > 0.0 && in_gcrs < 1.0,
            "the arrangement has to be a partial eclipse to be sensitive at all, got {in_gcrs}"
        );

        let in_cirs = illumination::<Cirs>(
            std::slice::from_ref(&occulter),
            &rotation.transform(&Vec3::<Gcrs>::from_raw(observer_gcrs)),
            &rotation.transform(&Vec3::<Gcrs>::from_raw(sun_gcrs)),
            &epoch,
        );
        // The two differ by the rotation's own arithmetic, a few parts in
        // 1e12. Leaving the occulter unrotated would move it by the frames'
        // 0.14 degrees of precession against an offset of 0.4, which changes
        // the illumination in the first decimal.
        assert!(
            (in_gcrs - in_cirs).abs() < 1e-9,
            "the same arrangement in two frames: {in_gcrs} against {in_cirs}"
        );

        // And the rotation is not the identity here, so the test above had
        // something to catch: the occulter moves by kilometres between frames.
        let turned = (*rotation
            .transform(&Vec3::<Gcrs>::from_raw(occulter_gcrs))
            .inner()
            - occulter_gcrs)
            .magnitude();
        assert!(
            turned > 1.0,
            "the frames should differ by more than a kilometre here, got {turned:.3} km"
        );
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
            illumination::<SimpleEci>(
                &[occulter],
                &Vec3::from_raw(Vector3::zeros()),
                &Vec3::from_raw(sun),
                &epoch,
            ),
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
            illumination::<SimpleEci>(
                &[behind_the_sun],
                &Vec3::from_raw(Vector3::zeros()),
                &Vec3::from_raw(sun),
                &epoch,
            ),
            1.0
        );
    }

    /// Containment does not depend on which way round the pair arrives. The
    /// relation is asked of one disc about another, so a wide body followed by
    /// a small one inside it has to read the same as the reverse.
    #[test]
    fn containment_is_found_whichever_order_the_pair_comes_in() {
        let wide = Seen {
            obscured: 0.9,
            direction: Vector3::x(),
            angular_radius: 0.5,
        };
        let inside = Seen {
            obscured: 0.2,
            direction: (Vector3::x() + Vector3::y() * 0.05).normalize(),
            angular_radius: 0.01,
        };
        assert!((combine(&wide, &inside) - 0.9).abs() < 1e-15);
        assert!((combine(&inside, &wide) - 0.9).abs() < 1e-15);
        // And through the two-body path, which calls `combine` in list order.
        for pair in [[wide, inside], [inside, wide]] {
            assert!((obscured_fraction(&pair) - 0.9).abs() < 1e-15);
        }
    }

    /// A radius that is not geometry would leave a body casting no shadow, or
    /// carry `NaN` into the force, so both constructors refuse it. Each case is
    /// named because a validation nothing exercises is one that can quietly go
    /// away.
    #[test]
    fn a_radius_that_is_not_geometry_is_refused() {
        use std::panic::catch_unwind;

        for radius in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            let central = catch_unwind(|| OccultingBody::central(radius, ShadowModel::Cylindrical));
            assert!(central.is_err(), "`central` accepted a radius of {radius}");
            let from_ephemeris = catch_unwind(|| {
                OccultingBody::from_ephemeris(
                    Arc::new(|_| Vec3::from_raw(Vector3::zeros())),
                    radius,
                    ShadowModel::Conical,
                )
            });
            assert!(
                from_ephemeris.is_err(),
                "`from_ephemeris` accepted a radius of {radius}"
            );
        }
    }

    /// Bodies that stand clear of each other hide different parts of the Sun,
    /// so their shares add — however many of them there are, and whatever
    /// order they arrive in. Only the bodies within one group are approximated.
    #[test]
    fn clear_discs_add_exactly_however_many_there_are() {
        let at = |angle: f64, obscured: f64| Seen {
            obscured,
            direction: (Vector3::x() * angle.cos() + Vector3::y() * angle.sin()).normalize(),
            angular_radius: 0.05,
        };
        let pair = [at(0.0, 0.3), at(1.0, 0.2)];
        assert_eq!(pair[0].against(&pair[1]), Relation::Clear);
        assert!((obscured_fraction(&pair) - 0.5).abs() < 1e-15);

        let three = [at(0.0, 0.3), at(1.0, 0.2), at(2.0, 0.1)];
        for order in [
            [three[0], three[1], three[2]],
            [three[2], three[0], three[1]],
        ] {
            let obscured = obscured_fraction(&order);
            assert!(
                (obscured - 0.6).abs() < 1e-15,
                "three clear discs add to 0.6, got {obscured}"
            );
        }
    }

    /// A group of touching discs adds beside the bodies clear of it: the two
    /// parts of the Sun are different, whatever happens inside the group.
    #[test]
    fn a_group_of_touching_discs_adds_beside_a_clear_body() {
        let at = |angle: f64, obscured: f64, angular_radius: f64| Seen {
            obscured,
            direction: (Vector3::x() * angle.cos() + Vector3::y() * angle.sin()).normalize(),
            angular_radius,
        };
        let touching = [at(0.0, 0.3, 0.1), at(0.15, 0.3, 0.1)];
        let clear = at(2.0, 0.2, 0.05);
        let group = 1.0 - 0.7 * 0.7;
        for order in [
            [touching[0], touching[1], clear],
            [clear, touching[0], touching[1]],
            [touching[1], clear, touching[0]],
        ] {
            let obscured = obscured_fraction(&order);
            assert!(
                (obscured - (group + 0.2)).abs() < 1e-15,
                "expected {} got {obscured}",
                group + 0.2
            );
        }
    }
}
