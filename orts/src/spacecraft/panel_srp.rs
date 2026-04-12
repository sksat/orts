use arika::epoch::Epoch;
use arika::sun;
use nalgebra::Vector3;

use crate::perturbations::{SOLAR_RADIATION_PRESSURE, shadow_function};
use arika::earth::R as R_EARTH;

use crate::model::{HasAttitude, HasMass, HasOrbit, Model};

use super::{ExternalLoads, SpacecraftShape};

/// Attitude-dependent solar radiation pressure model using flat surface panels.
///
/// Implements [`LoadModel`] to produce both translational acceleration and
/// SRP torque from per-panel radiation forces.  For the [`SpacecraftShape::Sphere`]
/// variant, the `cr` and `area` are read from the shape itself.
///
/// Per-panel force (simplified per panel):
///
/// ```text
/// F_panel = -P_sr × Cr × A × cos(θ) × (AU/r_sun)² × ŝ   [N]
/// ```
///
/// where `θ` is the angle between the panel normal and the Sun direction,
/// and `ŝ` is the unit vector from the satellite toward the Sun.
pub struct PanelSrp {
    shape: SpacecraftShape,
    /// Central body radius for cylindrical shadow model [km].
    /// `None` disables shadow computation (always sunlit).
    shadow_body_radius: Option<f64>,
}

impl PanelSrp {
    /// Create a panel-based (attitude-dependent) SRP model from surface panels.
    pub fn panels(panels: Vec<super::SurfacePanel>) -> Self {
        Self {
            shape: SpacecraftShape::Panels(panels),
            shadow_body_radius: None,
        }
    }

    /// Create an SRP model for Earth orbit with cylindrical Earth shadow.
    ///
    /// For the [`SpacecraftShape::Sphere`] variant, `cr` and `area` come from
    /// the shape. For [`SpacecraftShape::Panels`], each panel carries its own `cr`.
    pub fn for_earth(shape: SpacecraftShape) -> Self {
        Self {
            shape,
            shadow_body_radius: Some(R_EARTH),
        }
    }

    /// Create an SRP model without shadow.
    pub fn new(shape: SpacecraftShape) -> Self {
        Self {
            shape,
            shadow_body_radius: None,
        }
    }

    /// Set or override the shadow body radius (builder pattern).
    pub fn with_shadow_body(mut self, radius: f64) -> Self {
        self.shadow_body_radius = Some(radius);
        self
    }
}

impl PanelSrp {
    /// Compute SRP loads from full state (using capability trait methods).
    pub(crate) fn loads_from_state(
        &self,
        orbit: &crate::OrbitalState,
        attitude: &crate::attitude::AttitudeState,
        mass: f64,
        epoch: Option<&Epoch>,
    ) -> ExternalLoads {
        let epoch = match epoch {
            Some(e) => e,
            None => return ExternalLoads::zeros(),
        };

        let sun_pos = sun::sun_position_eci(epoch).into_inner();
        let sat_to_sun = sun_pos - *orbit.position();
        let r_sun = sat_to_sun.magnitude();
        let s_hat = sat_to_sun / r_sun;

        // Shadow check
        if let Some(body_r) = self.shadow_body_radius {
            let illumination = shadow_function(orbit.position(), &sun_pos, body_r);
            if illumination < 0.5 {
                return ExternalLoads::zeros();
            }
        }

        let distance_ratio = sun::AU_KM / r_sun;
        let base_pressure = SOLAR_RADIATION_PRESSURE * distance_ratio * distance_ratio; // [N/m²]

        match &self.shape {
            SpacecraftShape::Sphere { area, cr, .. } => {
                // a = -base_pressure * Cr * (A/m) * ŝ  [m/s²]
                // Divide by 1000 to convert to km/s²
                let a_mag = base_pressure * cr * area / mass / 1000.0;
                ExternalLoads {
                    acceleration_inertial: arika::frame::Vec3::from_raw(-a_mag * s_hat),
                    torque_body: arika::frame::Vec3::zeros(),
                    mass_rate: 0.0,
                }
            }
            SpacecraftShape::Panels(panels) => {
                // Transform Sun direction to body frame
                let s_body = attitude
                    .rotation_to_body()
                    .transform(&arika::frame::Vec3::from_raw(s_hat))
                    .into_inner();

                let mut total_force_body = Vector3::zeros(); // [N]
                let mut total_torque_body = Vector3::zeros(); // [N·m]

                for panel in panels {
                    // cos(θ) = n̂ · ŝ  (panel must face the Sun)
                    let cos_theta = panel.normal.dot(&s_body);
                    if cos_theta <= 0.0 {
                        continue;
                    }

                    // F = -base_pressure * Cr * A * cos(θ) * ŝ_body  [N]
                    // Force is away from Sun (opposite ŝ)
                    let force = -base_pressure * panel.cr * panel.area * cos_theta * s_body;

                    total_force_body += force;
                    total_torque_body += panel.cp_offset.cross(&force);
                }

                // a_body [m/s²] → a_inertial [km/s²]
                let a_body = arika::frame::Vec3::from_raw(total_force_body / mass);
                let a_inertial = attitude.rotation_to_eci().transform(&a_body) / 1000.0;

                ExternalLoads {
                    acceleration_inertial: a_inertial,
                    torque_body: arika::frame::Vec3::from_raw(total_torque_body),
                    mass_rate: 0.0,
                }
            }
        }
    }
}

impl<S: HasAttitude + HasOrbit<Frame = arika::frame::SimpleEci> + HasMass> Model<S> for PanelSrp {
    fn name(&self) -> &str {
        "panel_srp"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        self.loads_from_state(state.orbit(), state.attitude(), state.mass(), epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::SpacecraftState;
    use crate::attitude::AttitudeState;
    use crate::perturbations::SolarRadiationPressure;
    use crate::spacecraft::SurfacePanel;
    use arika::earth::MU as MU_EARTH;
    use nalgebra::{Vector4, vector};

    fn test_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    fn iss_state() -> SpacecraftState {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        SpacecraftState {
            orbit: OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0]),
            attitude: AttitudeState::identity(),
            mass: 1000.0,
        }
    }

    fn quat_from_axis_angle(axis: Vector3<f64>, angle: f64) -> Vector4<f64> {
        let q =
            nalgebra::UnitQuaternion::from_axis_angle(&nalgebra::Unit::new_normalize(axis), angle);
        Vector4::new(q.w, q.i, q.j, q.k)
    }

    // ======== Basic ========

    #[test]
    fn panel_srp_name() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        assert_eq!(Model::<SpacecraftState>::name(&srp), "panel_srp");
    }

    #[test]
    fn no_epoch_returns_zero() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let loads = srp.eval(0.0, &iss_state(), None);
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
        assert_eq!(loads.torque_body.into_inner(), Vector3::zeros());
    }

    #[test]
    fn for_earth_defaults() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        assert_eq!(srp.shadow_body_radius, Some(R_EARTH));
    }

    // ======== Sphere ========

    #[test]
    fn sphere_nonzero_srp() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));
        assert!(loads.acceleration_inertial.magnitude() > 0.0);
    }

    #[test]
    fn sphere_zero_torque() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));
        assert_eq!(loads.torque_body.into_inner(), Vector3::zeros());
    }

    #[test]
    fn sphere_attitude_independent() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();

        let s1 = iss_state();
        let mut s2 = iss_state();
        s2.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 2.0, 3.0), 1.2);

        let l1 = srp.eval(0.0, &s1, Some(&epoch));
        let l2 = srp.eval(0.0, &s2, Some(&epoch));

        assert!(
            (l1.acceleration_inertial - l2.acceleration_inertial).magnitude() < 1e-15,
            "Sphere SRP should be attitude-independent"
        );
    }

    #[test]
    fn sphere_away_from_sun() {
        let srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        let sun_dir = sun::sun_direction_eci(&epoch).into_inner();
        let cos_angle = loads
            .acceleration_inertial
            .into_inner()
            .normalize()
            .dot(&sun_dir);
        assert!(
            cos_angle < -0.5,
            "SRP should point away from Sun, cos_angle={cos_angle:.3}"
        );
    }

    #[test]
    fn sphere_matches_scalar_srp() {
        let epoch = test_epoch();
        let state = iss_state(); // mass = 1000.0

        // Sphere: area=20.0, cr=1.5 → area_to_mass = 20.0/1000.0 = 0.02
        let panel_srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5));

        let scalar_srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
        };

        let panel_loads = panel_srp.eval(0.0, &state, Some(&epoch));
        let scalar_a = scalar_srp.acceleration(state.orbit.position(), Some(&epoch));

        let rel_err = (panel_loads.acceleration_inertial.into_inner() - scalar_a).magnitude()
            / scalar_a.magnitude();
        assert!(
            rel_err < 1e-10,
            "PanelSrp sphere should match SolarRadiationPressure: rel_err={rel_err:.3e}"
        );
    }

    // ======== Ideal single panel + single Sun direction ========

    #[test]
    fn single_panel_face_on_analytical() {
        // A single panel facing exactly toward the Sun at identity attitude.
        // At March equinox, Sun is roughly +X, satellite at +X.
        // Panel normal = +X in body frame, identity attitude → +X in inertial.
        // cos(θ) ≈ 1, F = -P * Cr * A * ŝ
        let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let state = iss_state(); // at +X, identity attitude

        let loads = srp.eval(0.0, &state, Some(&epoch));

        // Expected magnitude: P_sr * (AU/r_sun)^2 * Cr * A * cos(θ) / (mass * 1000)
        // cos(θ) ≈ 1 (panel faces Sun), r_sun ≈ AU
        let expected_a = SOLAR_RADIATION_PRESSURE * 1.5 * 10.0 / (1000.0 * 1000.0);
        let actual_a = loads.acceleration_inertial.magnitude();

        let rel_err = (actual_a - expected_a).abs() / expected_a;
        assert!(
            rel_err < 0.05,
            "Single panel face-on: expected ~{expected_a:.3e}, got {actual_a:.3e}, rel_err={rel_err:.3}"
        );

        // Direction should be away from Sun (roughly -X)
        let sun_dir = sun::sun_direction_eci(&epoch).into_inner();
        assert!(
            loads
                .acceleration_inertial
                .into_inner()
                .normalize()
                .dot(&sun_dir)
                < -0.5
        );

        // No torque (panel at CoM)
        assert!(loads.torque_body.magnitude() < 1e-20);
    }

    #[test]
    fn single_panel_backface_zero() {
        // Panel normal = -X (facing away from Sun), should get zero force.
        let panel = SurfacePanel::at_com(10.0, Vector3::new(-1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let state = iss_state(); // at +X, Sun roughly at +X

        let loads = srp.eval(0.0, &state, Some(&epoch));
        assert!(
            loads.acceleration_inertial.magnitude() < 1e-20,
            "Backface panel should produce zero SRP"
        );
    }

    // ======== Panels — scaling ========

    #[test]
    fn panel_force_scales_with_area() {
        let epoch = test_epoch();
        let state = iss_state();

        let p1 = SurfacePanel::at_com(5.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let p2 = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);

        let l1 = PanelSrp::new(SpacecraftShape::panels(vec![p1])).eval(0.0, &state, Some(&epoch));
        let l2 = PanelSrp::new(SpacecraftShape::panels(vec![p2])).eval(0.0, &state, Some(&epoch));

        let ratio = l2.acceleration_inertial.magnitude() / l1.acceleration_inertial.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "2x area should give 2x force, ratio={ratio}"
        );
    }

    #[test]
    fn panel_force_scales_with_cr() {
        let epoch = test_epoch();
        let state = iss_state();

        let p1 = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.0);
        let p2 = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(2.0);

        let l1 = PanelSrp::new(SpacecraftShape::panels(vec![p1])).eval(0.0, &state, Some(&epoch));
        let l2 = PanelSrp::new(SpacecraftShape::panels(vec![p2])).eval(0.0, &state, Some(&epoch));

        let ratio = l2.acceleration_inertial.magnitude() / l1.acceleration_inertial.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "2x Cr should give 2x force, ratio={ratio}"
        );
    }

    // ======== Attitude coupling ========

    #[test]
    fn panels_different_attitude_different_srp() {
        // Use a panel normal aligned with the actual Sun direction for a clean test.
        let epoch = test_epoch();
        let sun_dir = sun::sun_direction_eci(&epoch).into_inner();

        let panel = SurfacePanel::at_com(10.0, sun_dir, 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));

        // Identity attitude: panel faces Sun → non-zero SRP
        let s1 = iss_state();
        let l1 = srp.eval(0.0, &s1, Some(&epoch));

        // Rotated 90° about Z: panel perpendicular to Sun → zero SRP
        let mut s2 = iss_state();
        s2.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2);
        let l2 = srp.eval(0.0, &s2, Some(&epoch));

        assert!(l1.acceleration_inertial.magnitude() > 1e-15);
        // After 90° rotation the cos(θ) is very small but might not be exactly zero
        // due to Sun direction having a small Z component. Check it's much smaller.
        assert!(
            l2.acceleration_inertial.magnitude() < l1.acceleration_inertial.magnitude() * 0.1,
            "90° rotation should drastically reduce SRP: face-on={:.3e}, rotated={:.3e}",
            l1.acceleration_inertial.magnitude(),
            l2.acceleration_inertial.magnitude()
        );
    }

    // ======== Shadow ========

    #[test]
    fn panel_zero_in_shadow() {
        // Panel normal = +X (faces Sun from -X side), but satellite is behind
        // Earth in shadow → shadow function should zero out the force.
        // We rotate the body 180° about Z so that body +X points toward +X in inertial
        // (the Sun direction), ensuring the panel *would* receive SRP if sunlit.
        let panel = SurfacePanel::at_com(10.0, Vector3::new(-1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::for_earth(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();

        // Place satellite behind Earth (Sun is roughly +X at equinox)
        let mut state = iss_state();
        state.orbit = OrbitalState::new(
            vector![-(R_EARTH + 400.0), 0.0, 0.0],
            vector![0.0, -7.67, 0.0],
        );
        // Rotate 180° about Z so body -X → inertial +X (toward Sun)
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::PI);

        let loads = srp.eval(0.0, &state, Some(&epoch));
        assert_eq!(
            loads.acceleration_inertial.into_inner(),
            Vector3::zeros(),
            "Should be zero in shadow"
        );

        // Verify it *would* be non-zero without shadow (confirms we're testing shadow, not backface)
        let srp_no_shadow = PanelSrp::new(SpacecraftShape::panels(vec![
            SurfacePanel::at_com(10.0, Vector3::new(-1.0, 0.0, 0.0), 2.2).with_cr(1.5),
        ]));
        let loads_no_shadow = srp_no_shadow.eval(0.0, &state, Some(&epoch));
        assert!(
            loads_no_shadow.acceleration_inertial.magnitude() > 0.0,
            "Without shadow, the same panel should receive SRP"
        );
    }

    #[test]
    fn no_shadow_body_always_sunlit() {
        // Place satellite behind Earth at -X with a Sun-facing panel.
        // With shadow_body_radius=None, the force should still be non-zero.
        let panel = SurfacePanel::at_com(10.0, Vector3::new(-1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel])); // no shadow

        let epoch = test_epoch();
        let mut state = iss_state();
        state.orbit = OrbitalState::new(
            vector![-(R_EARTH + 400.0), 0.0, 0.0],
            vector![0.0, -7.67, 0.0],
        );
        // Rotate 180° about Z so body -X → inertial +X (toward Sun)
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::PI);

        let loads = srp.eval(0.0, &state, Some(&epoch));
        assert!(
            loads.acceleration_inertial.magnitude() > 0.0,
            "Without shadow body, satellite behind Earth should still get SRP"
        );

        // And verify that with shadow it would be zero
        let srp_with_shadow = PanelSrp::for_earth(SpacecraftShape::panels(vec![
            SurfacePanel::at_com(10.0, Vector3::new(-1.0, 0.0, 0.0), 2.2).with_cr(1.5),
        ]));
        let loads_shadow = srp_with_shadow.eval(0.0, &state, Some(&epoch));
        assert_eq!(
            loads_shadow.acceleration_inertial.into_inner(),
            Vector3::zeros(),
            "With shadow body, same satellite should be in shadow"
        );
    }

    // ======== Torque ========

    #[test]
    fn panels_cp_offset_produces_torque() {
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            cr: 1.5,
            cp_offset: Vector3::new(0.0, 1.0, 0.0), // 1 m offset in +y
        };
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        assert!(
            loads.torque_body.magnitude() > 0.0,
            "Offset CP should produce non-zero torque"
        );
    }

    #[test]
    fn panels_cp_at_com_zero_torque() {
        let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Panel at CoM should produce zero torque"
        );
    }

    #[test]
    fn torque_cross_product_direction() {
        // Panel normal +X, CP offset (0, 1, 0).
        // Force is along -X in body frame (away from Sun).
        // τ = r × F = (0,1,0) × (F_x,0,0) = (0*0-0*F_x, 0*F_x-1*0, ... ) → z-component
        // Actually: (0,1,0) × (F_x,0,0) = (1*0 - 0*0, 0*F_x - 0*0, 0*0 - 1*F_x) = (0, 0, -F_x)
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            cr: 1.5,
            cp_offset: Vector3::new(0.0, 1.0, 0.0),
        };
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        // Torque should be primarily about z-axis, and negative
        // τ = (0,1,0) × F where F is mostly along -X → (0,0,-F_x) with F_x < 0
        // so z-component depends on the Sun direction body-frame projection
        assert!(
            loads.torque_body.z().abs() > loads.torque_body.x().abs(),
            "Torque should be primarily about z-axis"
        );
        assert!(
            loads.torque_body.z().abs() > loads.torque_body.y().abs(),
            "Torque should be primarily about z-axis"
        );
        // Force is in -ŝ direction (away from Sun). Sun ≈ +X at equinox,
        // so force ≈ -X in body. τ_z = r_x*F_y - r_y*F_x = 0 - 1*F_x.
        // F_x < 0, so τ_z = -F_x > 0.
        assert!(
            loads.torque_body.z() > 0.0,
            "Torque z-component should be positive: τ_z={:.3e}",
            loads.torque_body.z()
        );
    }

    // ======== Integration with SpacecraftDynamics ========

    #[test]
    fn panel_srp_integrable() {
        use crate::orbital::gravity::PointMass;
        use crate::spacecraft::SpacecraftDynamics;
        use utsuroi::{DynamicalSystem, Integrator, Rk4};

        let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, nalgebra::Matrix3::identity())
            .with_model(PanelSrp::new(SpacecraftShape::panels(vec![panel])))
            .with_epoch(test_epoch());

        let state = iss_state();
        let dy = dynamics.derivatives(0.0, &state.clone().into());
        assert!(dy.plant.orbit.position().magnitude().is_finite());

        // One RK4 step
        let next = Rk4.step(&dynamics, 0.0, &state.into(), 10.0);
        assert!(next.plant.orbit.position().magnitude().is_finite());
        assert!(next.plant.attitude.quaternion.magnitude() > 0.99);
    }

    #[test]
    fn panel_srp_plus_panel_drag_combined() {
        use crate::orbital::gravity::PointMass;
        use crate::spacecraft::{PanelDrag, SpacecraftDynamics};
        use utsuroi::DynamicalSystem;

        let panels = vec![
            SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5),
            SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2).with_cr(1.5),
        ];
        let shape = SpacecraftShape::panels(panels);

        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, nalgebra::Matrix3::identity())
            .with_model(PanelDrag::for_earth(shape.clone()))
            .with_model(PanelSrp::new(shape))
            .with_epoch(test_epoch());

        let state = iss_state();
        let dy = dynamics.derivatives(0.0, &state.into());
        assert!(dy.plant.orbit.position().magnitude().is_finite());
    }

    // ======== Order of magnitude ========

    #[test]
    fn srp_order_of_magnitude_geo() {
        // GEO satellite: A=30m², m=2000kg, Cr=1.5
        // |a| = P_sr * Cr * A/m / 1000 ≈ 4.54e-6 * 1.5 * 0.015 / 1000 ≈ 1.02e-10 km/s²
        let panel = SurfacePanel::at_com(30.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();

        let r_geo = 42164.0; // km
        let v_geo = (MU_EARTH / r_geo).sqrt();
        let state = SpacecraftState {
            orbit: OrbitalState::new(vector![r_geo, 0.0, 0.0], vector![0.0, v_geo, 0.0]),
            attitude: AttitudeState::identity(),
            mass: 2000.0,
        };

        let a_mag = srp
            .eval(0.0, &state, Some(&epoch))
            .acceleration_inertial
            .magnitude();
        assert!(
            a_mag > 1e-12 && a_mag < 1e-8,
            "GEO SRP should be ~1e-10 km/s², got {a_mag:.3e}"
        );
    }

    // ======== Tumbling (time-varying attitude) ========

    fn run_tumbling_srp_test(angular_velocity: Vector3<f64>) -> (f64, f64) {
        use crate::orbital::gravity::PointMass;
        use crate::spacecraft::SpacecraftDynamics;
        use nalgebra::Matrix3;
        use utsuroi::{Integrator, Rk4};

        // Asymmetric single panel: SRP depends on orientation
        let panel = SurfacePanel::at_com(20.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let epoch = test_epoch();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_model(srp)
            .with_epoch(epoch);

        let mut state = iss_state();
        state.attitude.angular_velocity = angular_velocity;

        // Collect SRP magnitude at each step
        let mut magnitudes = Vec::new();
        let _ = Rk4.integrate(&dyn_sc, state.into(), 0.0, 60.0, 1.0, |t, s| {
            let loads = dyn_sc.model_breakdown(t, &s.plant);
            if let Some((_, el)) = loads.first() {
                magnitudes.push(el.acceleration_inertial.magnitude());
            }
        });

        let min = magnitudes.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = magnitudes.iter().cloned().fold(0.0_f64, f64::max);
        (min, max)
    }

    #[test]
    fn tumbling_slow_varying_srp() {
        // Slow tumble about Z (0.05 rad/s ≈ 3°/s): SRP magnitude should vary
        // as the panel rotates in and out of Sun illumination.
        let (min, max) = run_tumbling_srp_test(Vector3::new(0.0, 0.0, 0.05));
        assert!(max > 0.0, "Should have some non-zero SRP during tumble");
        assert!(
            min < max * 0.99,
            "Slow tumble should cause varying SRP: min={min:.3e}, max={max:.3e}"
        );
    }

    #[test]
    fn tumbling_fast_varying_srp() {
        // Fast tumble about Z (1.0 rad/s ≈ 57°/s): SRP should vary more rapidly,
        // with the panel spending time in both sunlit and shadowed orientations.
        let (min, max) = run_tumbling_srp_test(Vector3::new(0.0, 0.0, 1.0));
        assert!(max > 0.0, "Should have some non-zero SRP during tumble");
        assert!(
            min < max * 0.99,
            "Fast tumble should cause varying SRP: min={min:.3e}, max={max:.3e}"
        );
    }

    #[test]
    fn tumbling_multiaxis_varying_srp() {
        // Tumble about all three axes: the panel normal sweeps a complex path.
        let (min, max) = run_tumbling_srp_test(Vector3::new(0.1, 0.05, 0.2));
        assert!(max > 0.0, "Should have some non-zero SRP during tumble");
        assert!(
            min < max * 0.99,
            "Multi-axis tumble should cause varying SRP: min={min:.3e}, max={max:.3e}"
        );
    }

    // ======== Empty panels ========

    #[test]
    fn empty_panels_returns_zeros() {
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
        assert_eq!(loads.torque_body.into_inner(), Vector3::zeros());
    }

    // ======== Mass scaling ========

    #[test]
    fn panel_force_scales_inversely_with_mass() {
        let epoch = test_epoch();
        let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);

        let mut s1 = iss_state();
        s1.mass = 500.0;
        let mut s2 = iss_state();
        s2.mass = 1000.0;

        let l1 = PanelSrp::new(SpacecraftShape::panels(vec![panel.clone()])).eval(
            0.0,
            &s1,
            Some(&epoch),
        );
        let l2 = PanelSrp::new(SpacecraftShape::panels(vec![panel])).eval(0.0, &s2, Some(&epoch));

        let ratio = l1.acceleration_inertial.magnitude() / l2.acceleration_inertial.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "Half mass should give 2x acceleration, ratio={ratio}"
        );
    }

    // ======== Mixed illumination (some panels face Sun, some don't) ========

    #[test]
    fn mixed_illumination_only_sunlit_panels_contribute() {
        let epoch = test_epoch();
        let state = iss_state();

        // Panel facing Sun (+X normal)
        let sunlit = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2).with_cr(1.5);
        // Panel facing away (-X normal) — backface, should not contribute
        let dark = SurfacePanel::at_com(10.0, Vector3::new(-1.0, 0.0, 0.0), 2.2).with_cr(1.5);

        let l_single = PanelSrp::new(SpacecraftShape::panels(vec![sunlit.clone()])).eval(
            0.0,
            &state,
            Some(&epoch),
        );
        let l_mixed = PanelSrp::new(SpacecraftShape::panels(vec![sunlit, dark])).eval(
            0.0,
            &state,
            Some(&epoch),
        );

        // Adding a backface panel should not change the result
        let diff = (l_single.acceleration_inertial - l_mixed.acceleration_inertial).magnitude();
        assert!(
            diff < 1e-20,
            "Backface panel should not contribute: diff={diff:.3e}"
        );
    }

    // ======== Builder ========

    #[test]
    fn with_shadow_body_builder() {
        let srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5)).with_shadow_body(R_EARTH);
        assert_eq!(srp.shadow_body_radius, Some(R_EARTH));
    }

    // ======== Cube (symmetric multi-panel) ========

    #[test]
    fn cube_srp_three_faces_illuminated() {
        // A cube has 6 faces; at most 3 face the Sun at any time.
        // For identity attitude and Sun ≈ +X, the +X face is fully illuminated,
        // while ±Y and ±Z faces get glancing illumination from the Sun's small
        // off-axis components. The -X, and the other back faces get zero.
        let cube = SpacecraftShape::cube(0.5, 2.2, 1.5); // 1m cube, half_size=0.5
        let srp = PanelSrp::new(cube);
        let epoch = test_epoch();
        let state = iss_state();

        let loads = srp.eval(0.0, &state, Some(&epoch));

        // Should produce non-zero force
        assert!(loads.acceleration_inertial.magnitude() > 0.0);

        // For a symmetric cube, the CP offsets of opposite faces cancel for face-on,
        // but glancing faces have non-cancelling CP torques. Net torque should be small
        // but possibly non-zero due to Sun not being exactly +X.
        // Just verify it's finite and much smaller than force * typical offset.
        assert!(loads.torque_body.magnitude().is_finite());
    }

    // ======== Proptest: cos(θ) scaling with panel tilt ========

    mod prop {
        use super::*;
        use proptest::prelude::*;

        /// Generate an angle in [0, π/2) so the panel always faces the Sun.
        fn angle_facing_sun() -> impl Strategy<Value = f64> {
            (0.01_f64..std::f64::consts::FRAC_PI_2 - 0.01).boxed()
        }

        proptest! {
            #[test]
            fn cos_theta_scaling(angle in angle_facing_sun()) {
                // Rotate the panel about Z by `angle` from the Sun direction.
                // Expected: force ∝ cos(angle) relative to face-on (angle=0).
                let epoch = test_epoch();

                // Face-on panel (normal = +X, Sun ≈ +X)
                let p_face_on = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2)
                    .with_cr(1.5);
                let l_face_on = PanelSrp::new(SpacecraftShape::panels(vec![p_face_on]))
                    .eval(0.0, &iss_state(), Some(&epoch));

                // Tilted panel: rotate spacecraft about Z by `angle`
                let mut state = iss_state();
                state.attitude.quaternion =
                    quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), angle);

                let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2)
                    .with_cr(1.5);
                let l_tilted = PanelSrp::new(SpacecraftShape::panels(vec![panel]))
                    .eval(0.0, &state, Some(&epoch));

                let face_on_mag = l_face_on.acceleration_inertial.magnitude();
                let tilted_mag = l_tilted.acceleration_inertial.magnitude();

                if face_on_mag > 1e-20 {
                    // The ratio should be approximately cos(angle), but the Sun
                    // direction is not exactly +X (it's approximately +X at equinox).
                    // So we compute the actual expected cos(θ) from the Sun direction.
                    let sun_dir = sun::sun_direction_eci(&epoch).into_inner();
                    // At identity: panel normal in inertial = +X
                    // At rotated: panel normal in inertial = (cos(angle), sin(angle), 0)
                    let rotated_normal = Vector3::new(angle.cos(), angle.sin(), 0.0);

                    let cos_theta_face = sun_dir.dot(&Vector3::new(1.0, 0.0, 0.0)).max(0.0);
                    let cos_theta_tilt = sun_dir.dot(&rotated_normal).max(0.0);

                    if cos_theta_face > 1e-10 {
                        let expected_ratio = cos_theta_tilt / cos_theta_face;
                        let actual_ratio = tilted_mag / face_on_mag;
                        let err = (actual_ratio - expected_ratio).abs();
                        prop_assert!(
                            err < 0.01,
                            "cos(θ) scaling failed: angle={angle:.4}, expected_ratio={expected_ratio:.6}, actual_ratio={actual_ratio:.6}, err={err:.6}"
                        );
                    }
                }
            }
        }
    }
}
