//! Sun sensor.
//!
//! Computes the sun direction in the spacecraft body frame from the
//! true spacecraft state and epoch. The sun direction is the
//! satellite→Sun unit vector rotated into the body frame.

use arika::earth::transform::EphemerisFrameBridge;
use arika::eclipse::ShadowModel;

use crate::eclipse::{OccultingBody, default_occulters};

/// The shadow geometry a sun sensor gives the central body.
///
/// Conical, which is what it has always used: a sensor reports the penumbra as
/// a fraction, and the two SRP models' cylindrical shadow would throw that
/// away. A distant occulter carries its own model either way.
pub const SENSOR_CENTRAL_SHADOW_MODEL: ShadowModel = ShadowModel::Conical;
use std::sync::Arc;

use arika::body::KnownBody;
use arika::epoch::{Epoch, Tdb};
use arika::frame::{self, Vec3};
use arika::sun::{self, SunPositionError, sun_position_eci};
use nalgebra::Vector3;

use super::noise::NoiseModel;
use crate::SpacecraftState;
use crate::model::HasAttitude;
use crate::perturbations::SunPositionFn;
use crate::plugin::tick_input::{SunDirectionBody, SunSensorOutput};

/// Sun sensor that measures the sun direction in the body frame.
///
/// Computes the satellite→Sun unit vector and rotates it into the
/// body frame via the attitude quaternion:
///
/// ```text
/// d_eci = normalize(sun_pos_eci - sc_pos_eci)
/// d_body = noise(R_bi · d_eci)
/// ```
///
/// When any body can block the Sun (`occulters` is not empty),
/// the sensor also computes the illumination fraction. During total
/// eclipse (illumination = 0), direction is `None`.
pub struct SunSensor {
    noise: Vec<Box<dyn NoiseModel>>,
    /// The bodies that can block the Sun.
    ///
    /// Empty means no eclipse is modelled and the illumination is always 1.0.
    occulters: Vec<OccultingBody>,
    /// The shadow geometry a body added by
    /// [`with_shadow_body`](Self::with_shadow_body) is given, and what
    /// [`with_shadow_model`](Self::with_shadow_model) records.
    ///
    /// Held beside the list so the two builders do not depend on the order they
    /// are called in: setting the geometry before naming the body has to reach
    /// that body.
    central_shadow_model: ShadowModel,
    /// Where the Sun is, relative to the central body [km].
    ///
    /// The reading is a direction to the Sun, so it depends on the central body
    /// exactly as the force models do. Defaults to the geocentric vector, which
    /// is only correct for Earth-centred propagation.
    sun_position_fn: SunPositionFn,
}

impl SunSensor {
    /// Create an ideal sun sensor (no noise, no eclipse) for Earth orbit.
    ///
    /// The Sun direction is geocentric. Use [`for_body`](Self::for_body) for any
    /// other central body: from Mars in 2026 the geocentric direction is up to
    /// 176° away from where Mars sees the Sun.
    pub fn new() -> Self {
        Self {
            noise: Vec::new(),
            occulters: Vec::new(),
            central_shadow_model: SENSOR_CENTRAL_SHADOW_MODEL,
            sun_position_fn: Arc::new(sun_position_eci),
        }
    }

    /// Create a sun sensor for Earth orbit with conical shadow model.
    pub fn for_earth() -> Self {
        Self::new().with_shadow_body(arika::earth::R)
    }

    /// Create a sun sensor for orbit about `body`, with that body's shadow.
    ///
    /// Orbiting the Sun puts it at the origin, so the direction is `-r_sat` and
    /// nothing eclipses it. Fails for a central body with no Sun ephemeris
    /// (Uranus, Neptune).
    pub fn for_body(body: KnownBody) -> Result<Self, SunPositionError> {
        if body == KnownBody::Sun {
            return Ok(Self {
                noise: Vec::new(),
                occulters: Vec::new(),
                central_shadow_model: SENSOR_CENTRAL_SHADOW_MODEL,
                sun_position_fn: Arc::new(|_| Vec3::from_raw(Vector3::zeros())),
            });
        }
        // Probe now so an unsupported body fails here rather than inside the
        // measurement, where the closure cannot report it.
        sun::sun_position_from_body(body, &Epoch::j2000().to_tdb())?;
        Ok(Self {
            noise: Vec::new(),
            occulters: default_occulters(body, SENSOR_CENTRAL_SHADOW_MODEL),
            central_shadow_model: SENSOR_CENTRAL_SHADOW_MODEL,
            sun_position_fn: Arc::new(move |epoch: &Epoch<Tdb>| {
                sun::sun_position_from_body(body, epoch)
                    .expect("the same body was accepted at construction")
            }),
        })
    }

    /// Add a noise model. Multiple calls chain in order.
    pub fn with_noise(mut self, noise: impl NoiseModel + 'static) -> Self {
        self.noise.push(Box::new(noise));
        self
    }

    /// Drop the shadow, keeping the Sun direction this sensor was built with
    /// (builder pattern).
    ///
    /// The same builder the two SRP models have: `for_body(body)?` carries that
    /// body's conical shadow, and this asks for an unshadowed reading without
    /// going back to the geocentric Sun that `new()` reads.
    pub fn without_shadow(mut self) -> Self {
        self.occulters.clear();
        self
    }

    /// Set the shadow body radius for eclipse computation.
    ///
    /// # Panics
    /// Panics unless the radius is finite and positive, as
    /// [`OccultingBody::central`](crate::eclipse::OccultingBody::central) does:
    /// a body of no radius would silently stop casting a shadow.
    pub fn with_shadow_body(mut self, radius: f64) -> Self {
        self.occulters = vec![OccultingBody::central(radius, self.central_shadow_model)];
        self
    }

    /// Set the shadow model.
    /// Set the shadow geometry of the central body.
    ///
    /// A distant occulter keeps its own: the Earth seen from a lunar orbit has
    /// to stay conical, where a cylindrical shadow would call 6.83 hours a year
    /// dark against the true 3.67.
    pub fn with_shadow_model(mut self, model: ShadowModel) -> Self {
        self.central_shadow_model = model;
        for occulter in self.occulters.iter_mut().filter(|body| body.is_central()) {
            occulter.shadow_model = model;
        }
        self
    }

    /// Add a body that can block the Sun, beside those already there.
    pub fn with_occulter(mut self, occulter: OccultingBody) -> Self {
        self.occulters.push(occulter);
        self
    }

    /// Measure the sun direction in the body frame (fine sun sensor).
    ///
    /// Returns `SunSensorOutput::Fine` with:
    /// - `direction: Some(...)`, a unit vector, when the sun is visible
    ///   (illumination > 0) and the noisy vector still has a direction
    /// - `direction: None` when in total eclipse (illumination = 0), when the
    ///   spacecraft is at the Sun's centre, or when noise left a zero or
    ///   non-finite vector
    /// - `illumination` in \[0, 1\]: actual eclipse-aware illumination fraction,
    ///   reported whether or not a direction came out
    ///
    /// `t` is the sim time of the sample [s], which the noise models are keyed on.
    pub fn measure(&mut self, t: f64, state: &SpacecraftState, epoch: &Epoch) -> SunSensorOutput {
        self.measure_in_frame::<frame::SimpleEci>(t, state, epoch)
    }

    /// Measure the sun direction in the body frame for a state propagated in an
    /// arbitrary inertial frame `F`.
    ///
    /// The analytic Sun ephemeris is expressed in `Gcrs`, so it is rotated into
    /// `F` via [`EphemerisFrameBridge`] before being differenced with the
    /// spacecraft position — identity for `SimpleEci`/`Gcrs` (preserving the
    /// historical behavior exactly), the precession/nutation rotation for an
    /// of-date frame such as `Cirs`. A frame without that impl is a compile
    /// error rather than a silent GCRS-alignment assumption.
    pub fn measure_in_frame<F: EphemerisFrameBridge>(
        &mut self,
        t: f64,
        state: &SpacecraftState<F>,
        epoch: &Epoch,
    ) -> SunSensorOutput {
        super::noise::keyed::check_sample_time(t);
        // Satellite-to-Sun vector in the propagation frame `F`
        let sun_gcrs = (self.sun_position_fn)(&epoch.to_tdb());
        let sun_eci = *F::ephemeris_rotation(epoch).transform(&sun_gcrs).inner();
        let sc_pos = *state.orbit.position_vec().inner();
        let sat_to_sun = sun_eci - sc_pos;
        // At the Sun's centre there is no direction to measure. Passing the
        // difference through unnormalized used to hand a non-unit vector (and,
        // with noise, a plausible-looking random one) to the guest. The same
        // normalization as the output's, so a position whose components' squares
        // overflow is not mistaken for a degenerate one.
        let dir_eci = crate::plugin::tick_input::normalize_finite(sat_to_sun);

        // What every body in the way leaves of the Sun.
        let illumination = crate::eclipse::illumination::<F>(
            &self.occulters,
            &state.orbit.position_vec(),
            &Vec3::from_raw(sun_eci),
            epoch,
        );

        // In total eclipse, direction is unmeasurable
        if illumination <= 0.0 {
            return SunSensorOutput::Fine {
                direction: None,
                illumination: 0.0,
            };
        }

        // Sunlit, but with no direction to report: keep the illumination that
        // was measured, so a guest can tell this from an eclipse.
        let Some(dir_eci) = dir_eci else {
            return SunSensorOutput::Fine {
                direction: None,
                illumination,
            };
        };

        // Rotate to body frame
        let dir_eci_typed = Vec3::<F>::from_raw(dir_eci);
        let dir_body = state.attitude_from_inertial().transform(&dir_eci_typed);
        let mut d = dir_body.into_inner();

        for n in &mut self.noise {
            d = n.apply(t, d);
        }

        // `new` normalizes, and answers `None` when the noise left a vector with
        // no direction. `illumination` stays as measured: it is the geometric
        // fraction of the Sun in view, not a flag for whether the read succeeded.
        SunSensorOutput::Fine {
            direction: SunDirectionBody::new(Vec3::<frame::Body>::from_raw(d)),
            illumination,
        }
    }
}

impl Default for SunSensor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use nalgebra::{Vector3, Vector4};

    fn leo_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        }
    }

    #[test]
    fn ideal_sun_sensor_returns_fine_with_unit_vector() {
        let mut sensor = SunSensor::new();
        let state = leo_state();
        let epoch = Epoch::j2000();
        let output = sensor.measure(0.0, &state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                let dir = direction.expect("should have direction when sunlit");
                let mag = dir.into_inner().magnitude();
                assert!(
                    (mag - 1.0).abs() < 1e-10,
                    "expected unit vector, got magnitude {mag}"
                );
                assert!((illumination - 1.0).abs() < 1e-15);
            }
            _ => panic!("expected Fine output"),
        }
    }

    /// Scales its input by a fixed factor, so the output norm is known exactly.
    struct ScaleNoise(f64);

    impl NoiseModel for ScaleNoise {
        fn apply(&mut self, _t: f64, true_value: Vector3<f64>) -> Vector3<f64> {
            true_value * self.0
        }
    }

    /// Replaces its input, for the degenerate cases a random model reaches only
    /// with vanishing probability.
    struct ReplaceNoise(Vector3<f64>);

    impl NoiseModel for ReplaceNoise {
        fn apply(&mut self, _t: f64, _true_value: Vector3<f64>) -> Vector3<f64> {
            self.0
        }
    }

    #[test]
    fn noise_does_not_change_the_length_of_the_measured_direction() {
        let mut sensor = SunSensor::new().with_noise(ScaleNoise(1.1));
        let output = sensor.measure(0.0, &leo_state(), &Epoch::j2000());
        match output {
            SunSensorOutput::Fine { direction, .. } => {
                let mag = direction
                    .expect("sunlit, so a direction is measured")
                    .into_inner()
                    .magnitude();
                assert!(
                    (mag - 1.0).abs() < 1e-12,
                    "SunDirectionBody is documented as a unit vector, got {mag}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn noise_moves_the_direction_it_reports() {
        let clean = match SunSensor::new().measure(0.0, &leo_state(), &Epoch::j2000()) {
            SunSensorOutput::Fine { direction, .. } => {
                direction.expect("sunlit").into_inner().into_inner()
            }
            _ => panic!("expected Fine output"),
        };
        // Only the part of an offset perpendicular to the direction turns into
        // an angle, so offset perpendicular: the expected angle is then atan(0.02).
        let perp = clean.cross(&Vector3::new(0.0, 0.0, 1.0)).normalize() * 0.02;
        let mut sensor = SunSensor::new().with_noise(ReplaceNoise(clean + perp));
        let noisy = match sensor.measure(0.0, &leo_state(), &Epoch::j2000()) {
            SunSensorOutput::Fine { direction, .. } => {
                direction.expect("sunlit").into_inner().into_inner()
            }
            _ => panic!("expected Fine output"),
        };
        let angle = (noisy.dot(&clean).clamp(-1.0, 1.0)).acos();
        let expected = 0.02_f64.atan();
        assert!(
            (angle - expected).abs() < 1e-9,
            "normalizing must keep the angular error {expected} rad, got {angle}"
        );
    }

    #[test]
    fn a_cancelled_measurement_reports_no_direction_and_keeps_illumination() {
        let mut sensor = SunSensor::new().with_noise(ReplaceNoise(Vector3::zeros()));
        match sensor.measure(0.0, &leo_state(), &Epoch::j2000()) {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(
                    direction.is_none(),
                    "a zero vector carries no direction, so none should be reported"
                );
                assert!(
                    (illumination - 1.0).abs() < 1e-15,
                    "the spacecraft is still in full sun: {illumination}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn non_finite_noise_reports_no_direction() {
        for bad in [
            Vector3::new(f64::NAN, 0.0, 0.0),
            Vector3::new(f64::INFINITY, 0.0, 0.0),
            Vector3::new(0.0, f64::NEG_INFINITY, 0.0),
        ] {
            let mut sensor = SunSensor::new().with_noise(ReplaceNoise(bad));
            match sensor.measure(0.0, &leo_state(), &Epoch::j2000()) {
                SunSensorOutput::Fine { direction, .. } => assert!(
                    direction.is_none(),
                    "{bad:?} cannot be normalized, so none should be reported"
                ),
                _ => panic!("expected Fine output"),
            }
        }
    }

    #[test]
    fn huge_but_finite_noise_still_reports_a_unit_vector() {
        // The squares of these components overflow to inf, so normalizing by
        // `magnitude()` alone would return NaN.
        let mut sensor =
            SunSensor::new().with_noise(ReplaceNoise(Vector3::new(1e200, -2e200, 3e200)));
        match sensor.measure(0.0, &leo_state(), &Epoch::j2000()) {
            SunSensorOutput::Fine { direction, .. } => {
                let v = direction
                    .expect("a finite non-zero vector has a direction")
                    .into_inner()
                    .into_inner();
                assert!(v.iter().all(|c| c.is_finite()), "got {v:?}");
                let mag = v.magnitude();
                assert!((mag - 1.0).abs() < 1e-12, "expected unit vector, got {mag}");
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn subnormal_and_mixed_magnitude_noise_still_report_a_unit_vector() {
        // Scaling by the largest component before normalizing is what keeps
        // these exact: subnormal components would underflow to zero when
        // squared, and a vector mixing 1e-320 with 1e200 would overflow.
        for v in [
            Vector3::new(1e-320, -2e-320, 3e-320),
            Vector3::new(f64::MIN_POSITIVE, 0.0, 0.0),
            Vector3::new(1e-320, 1e200, 0.0),
        ] {
            let mut sensor = SunSensor::new().with_noise(ReplaceNoise(v));
            match sensor.measure(0.0, &leo_state(), &Epoch::j2000()) {
                SunSensorOutput::Fine { direction, .. } => {
                    let mag = direction
                        .unwrap_or_else(|| panic!("{v:?} has a direction"))
                        .into_inner()
                        .magnitude();
                    assert!((mag - 1.0).abs() < 1e-12, "{v:?} gave magnitude {mag}");
                }
                _ => panic!("expected Fine output"),
            }
        }
    }

    #[test]
    fn at_the_suns_centre_there_is_no_direction_but_the_sunlight_is_reported() {
        use arika::sun::sun_position_eci;
        let epoch = Epoch::j2000();
        let sun = sun_position_eci(&epoch.to_tdb()).into_inner();
        let mut state = leo_state();
        state.orbit = OrbitalState::new(sun, Vector3::new(0.0, 0.0, 0.0));

        let mut sensor = SunSensor::new().without_shadow();
        match sensor.measure(0.0, &state, &epoch) {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(
                    direction.is_none(),
                    "no direction exists at the Sun's centre"
                );
                assert!(
                    illumination > 0.0,
                    "this is not an eclipse, so illumination must stay positive: {illumination}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn sun_direction_body_rejects_what_has_no_direction() {
        use crate::plugin::SunDirectionBody;

        let unit = |x, y, z| {
            SunDirectionBody::new(Vec3::<frame::Body>::from_raw(Vector3::new(x, y, z)))
                .map(|d| d.into_inner().into_inner().magnitude())
        };
        assert_eq!(unit(0.0, 0.0, 0.0), None, "a zero vector has no direction");
        assert_eq!(unit(f64::NAN, 1.0, 0.0), None);
        assert_eq!(unit(f64::INFINITY, 0.0, 0.0), None);
        assert_eq!(unit(0.0, f64::NEG_INFINITY, 1.0), None);
        for (x, y, z) in [
            (3.0, 4.0, 0.0),
            (f64::MAX, f64::MAX, f64::MAX),
            (1e-320, -1e-320, 0.0),
            (f64::MIN_POSITIVE, 0.0, 0.0),
        ] {
            let mag = unit(x, y, z).unwrap_or_else(|| panic!("{x},{y},{z} has a direction"));
            assert!(
                (mag - 1.0).abs() < 1e-12,
                "{x},{y},{z} gave magnitude {mag}"
            );
        }
    }

    #[test]
    fn identity_attitude_preserves_eci_direction() {
        let mut sensor = SunSensor::new();
        let state = leo_state();
        let epoch = Epoch::j2000();
        let output = sensor.measure(0.0, &state, &epoch);
        let dir_body = match output {
            SunSensorOutput::Fine { direction, .. } => direction
                .expect("should have direction")
                .into_inner()
                .into_inner(),
            _ => panic!("expected Fine output"),
        };

        // With identity quaternion, body == ECI
        use arika::sun::sun_position_eci;
        let sun_eci = sun_position_eci(&epoch.to_tdb()).into_inner();
        let sc_pos = state.orbit.position_eci().into_inner();
        let expected = (sun_eci - sc_pos).normalize();
        assert!(
            (dir_body - expected).magnitude() < 1e-10,
            "body should match ECI for identity attitude"
        );
    }

    // Frame-generalization characterization (#151)

    fn snapshot_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
            attitude: AttitudeState::new(
                nalgebra::UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::new(0.01, -0.02, 0.03),
            ),
            mass: 50.0,
        }
    }

    /// Characterization: pinned `SimpleEci` body-frame Sun direction, so
    /// rotating the (GCRS) Sun ephemeris into a generic frame `F` — identity for
    /// `SimpleEci` — cannot change it.
    ///
    /// Re-baselined when `arika::sun` started rotating the Meeus series from the
    /// mean equinox of date back to J2000: the direction moved by 0.3383°, which
    /// is the J2000→2024 accumulated precession the ephemeris used to leave in.
    /// The frame generalization this test guards is unaffected — `SimpleEci` is
    /// still the identity bridge — so the snapshot value is the only thing that
    /// moved.
    #[test]
    fn simple_eci_direction_snapshot() {
        let mut sensor = SunSensor::for_earth();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let output = sensor.measure(0.0, &snapshot_state(), &epoch);
        let SunSensorOutput::Fine {
            direction,
            illumination,
        } = output
        else {
            panic!("expected Fine output");
        };
        let got = direction.expect("sunlit").into_inner().into_inner();
        let expected = Vector3::new(
            0.7868574856732309,
            -0.5560253910118033,
            -0.26775186608158713,
        );
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "SimpleEci sun direction changed: {got:?}"
        );
        assert_eq!(illumination, 1.0);
    }

    /// **Discriminating test (#151)**: in a non-GCRS-aligned frame (`Cirs`) the
    /// Sun ephemeris must be rotated into the propagation frame before the
    /// geometry. The measured direction therefore equals the reconstruction
    /// through the GCRS→CIRS rotation (bit-exact) and differs measurably from
    /// the raw GCRS-aligned direction a frame-blind sensor would report.
    #[test]
    fn cirs_measurement_rotates_the_sun_ephemeris() {
        use arika::frame::{Cirs, Gcrs, Rotation};

        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let simple = snapshot_state();
        let pos = *simple.orbit.position();
        let state = SpacecraftState::<Cirs> {
            orbit: OrbitalState::<Cirs>::new_in_frame(pos, *simple.orbit.velocity()),
            attitude: simple.attitude.clone(),
            mass: simple.mass,
        };

        let mut sensor = SunSensor::for_earth();
        let SunSensorOutput::Fine { direction, .. } =
            sensor.measure_in_frame::<Cirs>(0.0, &state, &epoch)
        else {
            panic!("expected Fine output");
        };
        let got = direction.expect("sunlit").into_inner().into_inner();

        let sun_gcrs = sun_position_eci(&epoch.to_tdb());
        let sun_cirs = Rotation::<Gcrs, Cirs>::iau2006_model(&epoch.to_tt()).transform(&sun_gcrs);
        let dir_cirs = (sun_cirs.into_inner() - pos).normalize();
        let expected = state
            .attitude_from_inertial()
            .transform(&Vec3::<Cirs>::from_raw(dir_cirs))
            .into_inner();
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "Cirs sun direction must apply the GCRS→CIRS rotation: {got:?} vs {expected:?}"
        );

        let raw = Vector3::new(0.7903661281325338, -0.551312799346672, -0.2671620870882);
        assert!(
            (got - raw).magnitude() > 1e-8,
            "Cirs direction should differ from the raw GCRS-aligned direction"
        );
    }

    #[test]
    fn eclipse_sensor_returns_none_direction_in_shadow() {
        // Place satellite behind Earth where it should be in eclipse
        let mut sensor = SunSensor::for_earth();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

        // At equinox, Sun is roughly +X. Place satellite behind Earth at -X.
        let state = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(-(6371.0 + 400.0), 0.0, 0.0),
                Vector3::new(0.0, -7.67, 0.0),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };

        let output = sensor.measure(0.0, &state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(
                    direction.is_none(),
                    "direction should be None in total eclipse"
                );
                assert!(
                    illumination < 0.01,
                    "illumination should be ~0 in shadow, got {illumination}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn eclipse_sensor_returns_some_direction_when_sunlit() {
        let mut sensor = SunSensor::for_earth();
        let state = leo_state(); // Sun-side
        let epoch = Epoch::j2000();
        let output = sensor.measure(0.0, &state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(direction.is_some(), "direction should be Some when sunlit");
                assert!(
                    (illumination - 1.0).abs() < 0.01,
                    "illumination should be ~1.0, got {illumination}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn no_eclipse_sensor_always_sunlit() {
        // Without shadow body, even behind Earth should show illumination = 1
        let mut sensor = SunSensor::new();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let state = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(-(6371.0 + 400.0), 0.0, 0.0),
                Vector3::new(0.0, -7.67, 0.0),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };

        let output = sensor.measure(0.0, &state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(
                    direction.is_some(),
                    "no eclipse: direction should always be Some"
                );
                assert!(
                    (illumination - 1.0).abs() < 1e-15,
                    "no eclipse: illumination should be 1.0"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }
    /// `for_body` reads the Sun from the central body, not from Earth.
    ///
    /// The force models have their own tests for this; the sensor needs its
    /// own, because the reading is the attitude controller's input and can
    /// regress on its own. Around Mars in 2026 the geocentric direction is up
    /// to 176° away from where Mars sees the Sun, so a sensor still reading
    /// the geocentric vector points the controller at the wrong sky.
    #[test]
    fn for_body_reads_the_sun_from_that_body() {
        let epoch = Epoch::j2000();
        let mars_sun = sun::sun_position_from_body(KnownBody::Mars, &epoch.to_tdb())
            .expect("Mars is within the planetary elements");

        let mut sensor = SunSensor::for_body(KnownBody::Mars).expect("Mars has a Sun vector");
        // A state far enough out that Mars cannot eclipse it, so the reading is
        // the direction rather than a shadow decision.
        let state = SpacecraftState {
            orbit: OrbitalState::new(
                mars_sun.into_inner().normalize() * 1.0e5,
                Vector3::new(0.0, 1.0, 0.0),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };

        let SunSensorOutput::Fine { direction, .. } = sensor.measure(0.0, &state, &epoch) else {
            panic!("a sunlit sensor reports Fine");
        };
        let measured = direction
            .expect("sunlit, so there is a direction")
            .into_inner()
            .into_inner();

        let to_mars_sun = (mars_sun.into_inner() - *state.orbit.position()).normalize();
        assert!(
            measured.dot(&to_mars_sun) > 0.999_999,
            "the reading follows Mars's Sun: cos = {}",
            measured.dot(&to_mars_sun)
        );

        // The geocentric vector is what this used to read. It has to be a
        // different direction here, or the assertion above proves nothing.
        let earth_sun = sun::sun_position_eci(&epoch.to_tdb());
        let to_earth_sun = (earth_sun.into_inner() - *state.orbit.position()).normalize();
        assert!(
            to_mars_sun.dot(&to_earth_sun) < 0.999,
            "the two Sun directions have to differ for this test to mean anything: cos = {}",
            to_mars_sun.dot(&to_earth_sun)
        );
    }

    /// On Earth `for_body` is `for_earth`. At the Sun the origin *is* the Sun,
    /// so the direction points inward from the spacecraft (`-r_sat`), with no
    /// body to eclipse it.
    #[test]
    fn for_body_on_earth_and_on_the_sun() {
        let epoch = Epoch::j2000();
        let state = leo_state();

        let mut for_body = SunSensor::for_body(KnownBody::Earth).expect("Earth has a Sun vector");
        let mut for_earth = SunSensor::for_earth();
        let a = for_body.measure(0.0, &state, &epoch);
        let b = for_earth.measure(0.0, &state, &epoch);
        match (a, b) {
            (
                SunSensorOutput::Fine {
                    direction: Some(da),
                    illumination: ia,
                },
                SunSensorOutput::Fine {
                    direction: Some(db),
                    illumination: ib,
                },
            ) => {
                assert_eq!(da.into_inner(), db.into_inner(), "same direction on Earth");
                assert_eq!(ia, ib, "same illumination on Earth");
            }
            other => panic!("both report Fine on a sunlit LEO state: {other:?}"),
        }

        // At the Sun the origin *is* the Sun, so the direction points inward
        // from the spacecraft and nothing can shadow it.
        let mut at_sun = SunSensor::for_body(KnownBody::Sun).expect("the Sun needs no ephemeris");
        let SunSensorOutput::Fine {
            direction,
            illumination,
        } = at_sun.measure(0.0, &state, &epoch)
        else {
            panic!("nothing eclipses a spacecraft at the Sun");
        };
        let measured = direction.expect("sunlit").into_inner().into_inner();
        let inward = -state.orbit.position().normalize();
        assert!(
            measured.dot(&inward) > 0.999_999,
            "the Sun is at the origin, so its direction is -r: cos = {}",
            measured.dot(&inward)
        );
        assert!((illumination - 1.0).abs() < 1e-15, "no eclipse at the Sun");
    }

    /// A body outside the planetary elements is refused at construction.
    #[test]
    fn for_body_refuses_a_body_with_no_sun_ephemeris() {
        for body in [KnownBody::Uranus, KnownBody::Neptune] {
            assert!(
                SunSensor::for_body(body).is_err(),
                "{body:?} has no planetary elements, so the sensor cannot read a Sun"
            );
        }
    }
    fn angle_deg(a: &Vector3<f64>, b: &Vector3<f64>) -> f64 {
        a.normalize()
            .dot(&b.normalize())
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    /// `without_shadow` drops the eclipse and keeps the body's Sun.
    ///
    /// Behind Mars at 20 000 km the sensor reports no direction; with the
    /// shadow dropped it reports Mars' Sun direction, not Earth's (the two are
    /// 152.8° apart on this date).
    #[test]
    fn without_shadow_keeps_the_body_sun_direction() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let mars = KnownBody::Mars;
        let mars_sun = sun::sun_position_from_body(mars, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner();
        let earth_sun = sun::sun_position_eci(&epoch.to_tdb()).into_inner();

        // Directly behind Mars, inside its shadow.
        let position = -mars_sun.normalize() * 20_000.0;
        let state = SpacecraftState {
            orbit: OrbitalState::new(position, Vector3::new(0.0, 3.0, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 100.0,
        };

        let mut shadowed = SunSensor::for_body(mars).expect("Mars has a Sun ephemeris");
        assert!(
            matches!(
                shadowed.measure(0.0, &state, &epoch),
                SunSensorOutput::Fine {
                    direction: None,
                    ..
                }
            ),
            "Mars eclipses the satellite"
        );

        let mut ideal = SunSensor::for_body(mars)
            .expect("Mars has a Sun ephemeris")
            .without_shadow();
        let direction = match ideal.measure(0.0, &state, &epoch) {
            SunSensorOutput::Fine { direction, .. } => direction
                .expect("no shadow, so the reading is lit")
                .into_inner()
                .into_inner(),
            other => panic!("expected a fine reading, got {other:?}"),
        };

        let to_mars_sun = angle_deg(&direction, &mars_sun);
        let to_earth_sun = angle_deg(&direction, &earth_sun);
        assert!(
            to_mars_sun < 1.0e-4,
            "the direction is Mars' Sun: {to_mars_sun:.6}° away"
        );
        assert!(
            to_earth_sun > 150.0,
            "and not Earth's: {to_earth_sun:.3}° away"
        );
    }

    /// The shadow radius is the central body's, not Earth's.
    ///
    /// Behind Mars at 20 000 km, a point 5000 km off the anti-Sun axis sits
    /// outside Mars' umbra and inside the one Earth's radius would cast
    /// (measured: illumination 1.0 against 0.0). The other tests here cannot
    /// tell the two apart — the Mars cases are sunward of the body, and the
    /// eclipse cases are on Earth, where the radius is the same either way.
    #[test]
    fn for_body_takes_the_shadow_radius_from_that_body() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let mars = KnownBody::Mars;
        let mars_sun = sun::sun_position_from_body(mars, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner();
        let anti_sun = -mars_sun.normalize();
        let off_axis = anti_sun.cross(&Vector3::new(0.0, 0.0, 1.0)).normalize();

        let position = anti_sun * 20_000.0 + off_axis * 5000.0;
        let state = SpacecraftState {
            orbit: OrbitalState::new(position, Vector3::new(0.0, 3.0, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 100.0,
        };

        let illumination_of = |sensor: &mut SunSensor| match sensor.measure(0.0, &state, &epoch) {
            SunSensorOutput::Fine { illumination, .. } => illumination,
            other => panic!("expected a fine reading, got {other:?}"),
        };

        let mut from_mars = SunSensor::for_body(mars).expect("Mars has a Sun ephemeris");
        assert_eq!(
            illumination_of(&mut from_mars),
            1.0,
            "5000 km off the axis clears Mars' umbra (radius 3396.2 km)"
        );

        let mut with_earth_radius = SunSensor::for_body(mars)
            .expect("Mars has a Sun ephemeris")
            .with_shadow_body(arika::earth::R);
        assert_eq!(
            illumination_of(&mut with_earth_radius),
            0.0,
            "the same point is inside the umbra of a body Earth's size (6378.137 km)"
        );
    }

    /// A sun sensor on a lunar orbiter reports the Earth's eclipse.
    ///
    /// The sensor keeps its own list of occulters, and its answer is
    /// categorical rather than scaled: no direction at all while the Sun is
    /// hidden. Same geometry as the SRP tests — a total lunar eclipse, with the
    /// spacecraft on the sunward side of the Moon.
    #[test]
    fn a_sun_sensor_on_a_lunar_orbiter_sees_the_earths_eclipse() {
        use arika::body::KnownBody;

        let epoch = Epoch::from_iso8601("2026-03-03T11:30:00Z").expect("a valid epoch");
        let sun = *arika::sun::sun_position_from_body(KnownBody::Moon, &epoch.to_tdb())
            .expect("the Moon has a Sun ephemeris")
            .inner();
        let radius = KnownBody::Moon.properties().radius + 100.0;
        let position = sun.normalize() * radius;
        let speed = (KnownBody::Moon.properties().mu / radius).sqrt();
        let across = sun.normalize().cross(&Vector3::z()).normalize() * speed;
        let state = SpacecraftState {
            orbit: crate::OrbitalState::new(position, across),
            attitude: crate::attitude::AttitudeState::identity(),
            mass: 500.0,
        };

        let mut sensor = SunSensor::for_body(KnownBody::Moon).expect("the Moon is supported");
        match sensor.measure(0.0, &state, &epoch) {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert_eq!(illumination, 0.0, "the Earth's umbra hides the Sun");
                assert!(
                    direction.is_none(),
                    "a sensor in the umbra has no direction to report"
                );
            }
            other => panic!("expected a fine sun sensor reading, got {other:?}"),
        }

        let mut lit = SunSensor::for_body(KnownBody::Moon)
            .expect("the Moon is supported")
            .without_shadow();
        match lit.measure(0.0, &state, &epoch) {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert_eq!(illumination, 1.0);
                assert!(direction.is_some());
            }
            other => panic!("expected a fine sun sensor reading, got {other:?}"),
        }
    }

    /// The geometry a caller asks for reaches the body it names, whichever
    /// order the two builders are called in.
    #[test]
    fn the_shadow_model_survives_either_builder_order() {
        let model_first = SunSensor::new()
            .with_shadow_model(ShadowModel::Cylindrical)
            .with_shadow_body(arika::earth::R);
        let body_first = SunSensor::new()
            .with_shadow_body(arika::earth::R)
            .with_shadow_model(ShadowModel::Cylindrical);
        for sensor in [model_first, body_first] {
            assert_eq!(sensor.occulters.len(), 1);
            assert_eq!(
                sensor.occulters[0].shadow_model,
                ShadowModel::Cylindrical,
                "the sensor's conical default must not override the caller"
            );
        }
    }
}
