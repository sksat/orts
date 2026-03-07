use kaname::epoch::Epoch;
use nalgebra::Vector3;
use orts_orbits::body::KnownBody;
use orts_orbits::constants::R_EARTH;
use orts_orbits::drag::OMEGA_EARTH;
use tobari::{AtmosphereModel, Exponential};

use super::{ExternalLoads, LoadModel, SpacecraftState};

/// A flat surface panel on a spacecraft body.
///
/// Represents one face of the spacecraft's outer surface for computing
/// aerodynamic (and eventually SRP) forces.  Each panel has an outward-pointing
/// normal in the body frame, a drag coefficient, and a centre-of-pressure
/// offset from the centre of mass.
///
/// For thin surfaces like solar panels where both sides are exposed to the
/// flow, model each side as a separate panel with opposite normals.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfacePanel {
    /// Panel area [m²].
    pub area: f64,
    /// Outward-pointing unit normal in the body frame.
    pub normal: Vector3<f64>,
    /// Drag coefficient (typically 2.0–2.2 for LEO free-molecular flow).
    pub cd: f64,
    /// Centre-of-pressure offset from the spacecraft CoM [m, body frame].
    pub cp_offset: Vector3<f64>,
}

impl SurfacePanel {
    /// Create a panel whose centre of pressure coincides with the CoM.
    ///
    /// The `normal` vector is normalised internally; it need not be unit length.
    ///
    /// # Panics
    /// Panics if `normal` is zero-length.
    pub fn at_com(area: f64, normal: Vector3<f64>, cd: f64) -> Self {
        let n = normal.normalize();
        assert!(n.magnitude() > 0.5, "Panel normal must be non-zero");
        Self {
            area,
            normal: n,
            cd,
            cp_offset: Vector3::zeros(),
        }
    }
}

/// Spacecraft shape model for aerodynamic force computation.
///
/// Provides a gradation from the simplest attitude-independent model
/// (`Cannonball`) to fully attitude-dependent panel models (`Panels`).
#[derive(Debug, Clone)]
pub enum SpacecraftShape {
    /// Spherical / cannonball model: constant ballistic coefficient,
    /// attitude-independent.  Equivalent to the existing `AtmosphericDrag`.
    Cannonball {
        /// Ballistic coefficient Cd·A/(2m) [m²/kg].
        ballistic_coeff: f64,
    },
    /// Flat-panel model: attitude-dependent.
    Panels(Vec<SurfacePanel>),
}

impl SpacecraftShape {
    /// Create a cannonball (sphere) shape with the given ballistic coefficient.
    pub fn cannonball(ballistic_coeff: f64) -> Self {
        Self::Cannonball { ballistic_coeff }
    }

    /// Create a panel model from an arbitrary set of panels.
    pub fn panels(panels: Vec<SurfacePanel>) -> Self {
        Self::Panels(panels)
    }

    /// Create a cube with the given half-size and drag coefficient.
    ///
    /// Generates 6 panels (±x, ±y, ±z), each with area `(2 * half_size)²` m²
    /// and centre of pressure at the face centre (`half_size` m from CoM along
    /// the face normal).
    pub fn cube(half_size: f64, cd: f64) -> Self {
        let face_area = (2.0 * half_size) * (2.0 * half_size);
        let panels = vec![
            SurfacePanel {
                area: face_area,
                normal: Vector3::new(1.0, 0.0, 0.0),
                cd,
                cp_offset: Vector3::new(half_size, 0.0, 0.0),
            },
            SurfacePanel {
                area: face_area,
                normal: Vector3::new(-1.0, 0.0, 0.0),
                cd,
                cp_offset: Vector3::new(-half_size, 0.0, 0.0),
            },
            SurfacePanel {
                area: face_area,
                normal: Vector3::new(0.0, 1.0, 0.0),
                cd,
                cp_offset: Vector3::new(0.0, half_size, 0.0),
            },
            SurfacePanel {
                area: face_area,
                normal: Vector3::new(0.0, -1.0, 0.0),
                cd,
                cp_offset: Vector3::new(0.0, -half_size, 0.0),
            },
            SurfacePanel {
                area: face_area,
                normal: Vector3::new(0.0, 0.0, 1.0),
                cd,
                cp_offset: Vector3::new(0.0, 0.0, half_size),
            },
            SurfacePanel {
                area: face_area,
                normal: Vector3::new(0.0, 0.0, -1.0),
                cd,
                cp_offset: Vector3::new(0.0, 0.0, -half_size),
            },
        ];
        Self::Panels(panels)
    }
}

/// Attitude-dependent drag model using flat surface panels.
///
/// Implements [`LoadModel`] to produce both translational acceleration and
/// aerodynamic torque from per-panel drag forces.  For the [`SpacecraftShape::Cannonball`]
/// variant, behaves identically to the scalar `AtmosphericDrag` in `orts-orbits`.
pub struct PanelDrag {
    shape: SpacecraftShape,
    atmosphere: Box<dyn AtmosphereModel>,
    body: Option<KnownBody>,
    body_radius: f64,
    omega_body: f64,
}

impl PanelDrag {
    /// Create a panel drag model for Earth orbit.
    ///
    /// Uses piecewise exponential atmosphere and WGS-84 geodetic altitude by default.
    pub fn for_earth(shape: SpacecraftShape) -> Self {
        Self {
            shape,
            atmosphere: Box::new(Exponential),
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
        }
    }

    /// Replace the atmospheric density model (builder pattern).
    pub fn with_atmosphere(mut self, model: Box<dyn AtmosphereModel>) -> Self {
        self.atmosphere = model;
        self
    }
}

impl PanelDrag {
    /// Check if the position is inside the central body.
    fn is_inside(&self, position: &Vector3<f64>) -> bool {
        match self.body {
            Some(KnownBody::Earth) => {
                let p2 = position.x * position.x + position.y * position.y;
                let z2 = position.z * position.z;
                p2 / (kaname::WGS84_A * kaname::WGS84_A) + z2 / (kaname::WGS84_B * kaname::WGS84_B)
                    < 1.0
            }
            _ => position.magnitude() < self.body_radius,
        }
    }

    /// Compute altitude [km] from position.
    fn altitude(&self, position: &Vector3<f64>) -> f64 {
        match self.body {
            Some(KnownBody::Earth) => kaname::geodetic_altitude(position),
            _ => position.magnitude() - self.body_radius,
        }
    }

    /// Compute relative velocity accounting for atmosphere co-rotation [km/s].
    fn relative_velocity(&self, state: &SpacecraftState) -> Vector3<f64> {
        let omega = Vector3::new(0.0, 0.0, self.omega_body);
        state.orbit.velocity - omega.cross(&state.orbit.position)
    }
}

impl LoadModel for PanelDrag {
    fn name(&self) -> &str {
        "panel_drag"
    }

    fn loads(&self, _t: f64, state: &SpacecraftState, epoch: Option<&Epoch>) -> ExternalLoads {
        // Inside body → zero
        if self.is_inside(&state.orbit.position) {
            return ExternalLoads::zeros();
        }

        let alt = self.altitude(&state.orbit.position);
        let rho = self.atmosphere.density(alt, &state.orbit.position, epoch);
        if rho == 0.0 {
            return ExternalLoads::zeros();
        }

        // Relative velocity (inertial frame, km/s)
        let v_rel = self.relative_velocity(state);
        let v_rel_mag_km = v_rel.magnitude();
        if v_rel_mag_km < 1e-10 {
            return ExternalLoads::zeros();
        }

        match &self.shape {
            SpacecraftShape::Cannonball { ballistic_coeff } => {
                // a = -B * ρ * |v_rel| * v_rel  (same as AtmosphericDrag)
                let v_rel_m = v_rel * 1000.0; // km/s → m/s
                let v_rel_mag_m = v_rel_m.magnitude();
                let a_drag_m = -ballistic_coeff * rho * v_rel_mag_m * v_rel_m;
                ExternalLoads {
                    acceleration_inertial: a_drag_m / 1000.0, // m/s² → km/s²
                    torque_body: Vector3::zeros(),
                }
            }
            SpacecraftShape::Panels(panels) => {
                // Transform flow direction to body frame
                let r_bi = state.attitude.inertial_to_body();
                let v_body = r_bi * v_rel; // km/s in body frame
                let v_body_m = v_body * 1000.0; // m/s
                let v_body_mag_m = v_body_m.magnitude();
                let v_hat_body = v_body_m / v_body_mag_m;

                let mut total_force_body = Vector3::zeros(); // N
                let mut total_torque_body = Vector3::zeros(); // N·m

                for panel in panels {
                    // cos(θ) = n̂ · (-v̂): panel must face the flow
                    let cos_theta = panel.normal.dot(&(-v_hat_body)).max(0.0);
                    if cos_theta <= 0.0 {
                        continue;
                    }

                    let a_proj = panel.area * cos_theta; // m²

                    // F = -½ ρ Cd A_proj |v|² v̂  [N]
                    let force =
                        -0.5 * rho * panel.cd * a_proj * v_body_mag_m * v_body_mag_m * v_hat_body;

                    total_force_body += force;
                    total_torque_body += panel.cp_offset.cross(&force);
                }

                // a_body [m/s²] → a_inertial [km/s²]
                let a_body = total_force_body / state.mass; // m/s²
                let r_ib = state.attitude.rotation_matrix();
                let a_inertial = r_ib * a_body / 1000.0; // km/s²

                ExternalLoads {
                    acceleration_inertial: a_inertial,
                    torque_body: total_torque_body,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ======== SurfacePanel ========

    #[test]
    fn at_com_zero_cp_offset() {
        let p = SurfacePanel::at_com(2.0, Vector3::new(1.0, 0.0, 0.0), 2.2);
        assert_eq!(p.cp_offset, Vector3::zeros());
    }

    #[test]
    fn at_com_normalises_normal() {
        let p = SurfacePanel::at_com(1.0, Vector3::new(3.0, 4.0, 0.0), 2.0);
        let expected = Vector3::new(0.6, 0.8, 0.0);
        assert!(
            (p.normal - expected).magnitude() < 1e-15,
            "Normal should be normalised, got {:?}",
            p.normal
        );
    }

    #[test]
    fn at_com_already_unit() {
        let n = Vector3::new(0.0, 0.0, 1.0);
        let p = SurfacePanel::at_com(5.0, n, 2.2);
        assert!((p.normal - n).magnitude() < 1e-15);
    }

    #[test]
    #[should_panic]
    fn at_com_zero_normal_panics() {
        SurfacePanel::at_com(1.0, Vector3::zeros(), 2.0);
    }

    #[test]
    fn at_com_preserves_area_and_cd() {
        let p = SurfacePanel::at_com(3.5, Vector3::new(0.0, 1.0, 0.0), 2.1);
        assert!((p.area - 3.5).abs() < 1e-15);
        assert!((p.cd - 2.1).abs() < 1e-15);
    }

    // ======== SpacecraftShape::cannonball ========

    #[test]
    fn cannonball_variant() {
        let shape = SpacecraftShape::cannonball(0.01);
        match shape {
            SpacecraftShape::Cannonball { ballistic_coeff } => {
                assert!((ballistic_coeff - 0.01).abs() < 1e-15);
            }
            _ => panic!("Expected Cannonball variant"),
        }
    }

    // ======== SpacecraftShape::panels ========

    #[test]
    fn panels_stores_panels() {
        let panels = vec![
            SurfacePanel::at_com(1.0, Vector3::new(1.0, 0.0, 0.0), 2.0),
            SurfacePanel::at_com(2.0, Vector3::new(0.0, 1.0, 0.0), 2.2),
        ];
        let shape = SpacecraftShape::panels(panels.clone());
        match shape {
            SpacecraftShape::Panels(p) => {
                assert_eq!(p.len(), 2);
                assert!((p[0].area - 1.0).abs() < 1e-15);
                assert!((p[1].area - 2.0).abs() < 1e-15);
            }
            _ => panic!("Expected Panels variant"),
        }
    }

    // ======== SpacecraftShape::cube ========

    #[test]
    fn cube_has_six_panels() {
        let shape = SpacecraftShape::cube(0.5, 2.2);
        match &shape {
            SpacecraftShape::Panels(panels) => {
                assert_eq!(panels.len(), 6, "Cube should have 6 faces");
            }
            _ => panic!("Expected Panels variant"),
        }
    }

    #[test]
    fn cube_face_area() {
        let half = 0.5;
        let expected_area = (2.0 * half) * (2.0 * half); // 1.0 m²
        let shape = SpacecraftShape::cube(half, 2.2);
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                assert!(
                    (p.area - expected_area).abs() < 1e-15,
                    "Panel {i} area: expected {expected_area}, got {}",
                    p.area
                );
            }
        }
    }

    #[test]
    fn cube_normals_are_unit() {
        let shape = SpacecraftShape::cube(1.0, 2.0);
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                assert!(
                    (p.normal.magnitude() - 1.0).abs() < 1e-15,
                    "Panel {i} normal not unit: magnitude = {}",
                    p.normal.magnitude()
                );
            }
        }
    }

    #[test]
    fn cube_normals_are_axis_aligned() {
        let shape = SpacecraftShape::cube(1.0, 2.0);
        if let SpacecraftShape::Panels(panels) = &shape {
            let normals: Vec<_> = panels.iter().map(|p| p.normal).collect();
            let expected = [
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(-1.0, 0.0, 0.0),
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(0.0, -1.0, 0.0),
                Vector3::new(0.0, 0.0, 1.0),
                Vector3::new(0.0, 0.0, -1.0),
            ];
            for (i, (n, e)) in normals.iter().zip(expected.iter()).enumerate() {
                assert!(
                    (n - e).magnitude() < 1e-15,
                    "Panel {i}: expected normal {e:?}, got {n:?}"
                );
            }
        }
    }

    #[test]
    fn cube_cp_at_face_centre() {
        let half = 0.75;
        let shape = SpacecraftShape::cube(half, 2.0);
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                // CP should be at half_size along the normal direction
                let expected_cp = p.normal * half;
                assert!(
                    (p.cp_offset - expected_cp).magnitude() < 1e-15,
                    "Panel {i}: expected CP {expected_cp:?}, got {:?}",
                    p.cp_offset
                );
            }
        }
    }

    #[test]
    fn cube_all_same_cd() {
        let cd = 2.2;
        let shape = SpacecraftShape::cube(0.5, cd);
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                assert!(
                    (p.cd - cd).abs() < 1e-15,
                    "Panel {i} cd: expected {cd}, got {}",
                    p.cd
                );
            }
        }
    }

    #[test]
    fn cube_opposite_normals_cancel() {
        let shape = SpacecraftShape::cube(1.0, 2.0);
        if let SpacecraftShape::Panels(panels) = &shape {
            let normal_sum: Vector3<f64> = panels.iter().map(|p| p.normal).sum();
            assert!(
                normal_sum.magnitude() < 1e-14,
                "Opposite normals should cancel: sum = {normal_sum:?}"
            );
        }
    }

    // ======== PanelDrag ========

    #[test]
    fn panel_drag_name() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.01));
        assert_eq!(drag.name(), "panel_drag");
    }

    #[test]
    fn panel_drag_for_earth_defaults() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.01));
        assert_eq!(drag.body, Some(KnownBody::Earth));
        assert!((drag.body_radius - R_EARTH).abs() < 1e-10);
        assert!((drag.omega_body - OMEGA_EARTH).abs() < 1e-15);
    }

    #[test]
    fn panel_drag_with_atmosphere_builder() {
        use tobari::HarrisPriester;

        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.01))
            .with_atmosphere(Box::new(HarrisPriester::new()));
        // Should compile and not panic — atmosphere model replaced
        assert_eq!(drag.name(), "panel_drag");
    }

    // ======== PanelDrag loads() — shared helpers ========

    use orts_attitude::AttitudeState;
    use orts_integrator::State;
    use nalgebra::Vector4;

    fn iss_state() -> SpacecraftState {
        let r = R_EARTH + 400.0;
        let v = (orts_orbits::constants::MU_EARTH / r).sqrt();
        SpacecraftState {
            orbit: State {
                position: Vector3::new(r, 0.0, 0.0),
                velocity: Vector3::new(0.0, v, 0.0),
            },
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    // ======== Cannonball branch ========

    #[test]
    fn cannonball_nonzero_drag_at_iss() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.005));
        let loads = drag.loads(0.0, &iss_state(), None);
        assert!(
            loads.acceleration_inertial.magnitude() > 0.0,
            "Cannonball should produce non-zero drag at ISS altitude"
        );
    }

    #[test]
    fn cannonball_zero_torque() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.005));
        let loads = drag.loads(0.0, &iss_state(), None);
        assert_eq!(
            loads.torque_body,
            Vector3::zeros(),
            "Cannonball should produce zero torque"
        );
    }

    #[test]
    fn cannonball_attitude_independent() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.005));
        let s1 = iss_state();
        let mut s2 = iss_state();
        // Rotate 90° about z-axis: q = (cos45, 0, 0, sin45)
        let c = std::f64::consts::FRAC_PI_4.cos();
        let s = std::f64::consts::FRAC_PI_4.sin();
        s2.attitude.quaternion = Vector4::new(c, 0.0, 0.0, s);

        let l1 = drag.loads(0.0, &s1, None);
        let l2 = drag.loads(0.0, &s2, None);
        assert!(
            (l1.acceleration_inertial - l2.acceleration_inertial).magnitude() < 1e-15,
            "Cannonball drag should not depend on attitude"
        );
    }

    #[test]
    fn cannonball_opposes_velocity() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.005));
        let loads = drag.loads(0.0, &iss_state(), None);
        // v_rel is mostly in +y → drag should be in -y
        assert!(loads.acceleration_inertial.y < 0.0);
    }

    // ======== Panels branch — acceleration ========

    #[test]
    fn panels_facing_flow_nonzero_drag() {
        // Single panel facing -y (into the +y flow)
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.loads(0.0, &iss_state(), None);
        assert!(
            loads.acceleration_inertial.magnitude() > 0.0,
            "Panel facing flow should produce drag"
        );
    }

    #[test]
    fn panels_backface_zero_drag() {
        // Single panel facing +y (away from the +y flow) — backface
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, 1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.loads(0.0, &iss_state(), None);
        assert_eq!(
            loads.acceleration_inertial,
            Vector3::zeros(),
            "Panel facing away from flow should produce zero drag"
        );
    }

    #[test]
    fn panels_drag_opposes_velocity() {
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.loads(0.0, &iss_state(), None);
        // Drag should oppose velocity (predominantly -y)
        assert!(
            loads.acceleration_inertial.y < 0.0,
            "Panel drag should oppose velocity"
        );
    }

    #[test]
    fn panels_different_attitude_different_drag() {
        // This is the core coupling test: rotating the spacecraft changes the drag
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);

        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        // Identity attitude: panel normal -y faces the +y flow → full drag
        let s1 = iss_state();
        let l1 = drag.loads(0.0, &s1, None);

        // Rotate 90° about z: panel normal rotates from -y to +x in inertial
        // → panel no longer faces the +y flow → different drag
        let mut s2 = iss_state();
        let c = std::f64::consts::FRAC_PI_4.cos();
        let s = std::f64::consts::FRAC_PI_4.sin();
        s2.attitude.quaternion = Vector4::new(c, 0.0, 0.0, s);

        let l2 = drag.loads(0.0, &s2, None);

        let diff = (l1.acceleration_inertial - l2.acceleration_inertial).magnitude();
        assert!(
            diff > 1e-15,
            "Different attitudes should produce different drag: diff = {diff:.3e}"
        );
    }

    #[test]
    fn panels_rotated_to_backface_zero() {
        // Panel faces -y in body frame. Rotate 180° about z → panel faces +y → backface
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let mut state = iss_state();
        // 180° rotation about z: q = (0, 0, 0, 1)
        state.attitude.quaternion = Vector4::new(0.0, 0.0, 0.0, 1.0);
        let loads = drag.loads(0.0, &state, None);

        assert!(
            loads.acceleration_inertial.magnitude() < 1e-15,
            "Panel rotated to backface should produce zero drag, got {:?}",
            loads.acceleration_inertial
        );
    }

    #[test]
    fn panels_above_atmosphere_zero() {
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let state = SpacecraftState {
            orbit: State {
                position: Vector3::new(R_EARTH + 3000.0, 0.0, 0.0),
                velocity: Vector3::new(0.0, 5.0, 0.0),
            },
            attitude: AttitudeState::identity(),
            mass: 500.0,
        };
        let loads = drag.loads(0.0, &state, None);
        assert_eq!(loads.acceleration_inertial, Vector3::zeros());
    }

    // ======== Panels branch — torque ========

    #[test]
    fn panels_at_com_zero_torque() {
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.loads(0.0, &iss_state(), None);
        assert_eq!(
            loads.torque_body,
            Vector3::zeros(),
            "Panel at CoM should produce zero torque"
        );
    }

    #[test]
    fn panels_cp_offset_produces_torque() {
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, -1.0, 0.0),
            cd: 2.2,
            cp_offset: Vector3::new(1.0, 0.0, 0.0), // 1 m offset in +x
        };
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.loads(0.0, &iss_state(), None);

        assert!(
            loads.torque_body.magnitude() > 0.0,
            "Offset CP should produce non-zero torque"
        );
        // Force is in -y body frame (opposing +y flow), CP offset in +x
        // τ = r × F = (1,0,0) × (0,F_y,0) = (0,0,F_y) → z-component
        assert!(
            loads.torque_body.z.abs() > loads.torque_body.x.abs(),
            "Torque should be primarily about z-axis"
        );
    }

    #[test]
    fn panels_double_offset_double_torque() {
        let make_panel = |offset: f64| SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, -1.0, 0.0),
            cd: 2.2,
            cp_offset: Vector3::new(offset, 0.0, 0.0),
        };

        let drag1 = PanelDrag::for_earth(SpacecraftShape::panels(vec![make_panel(1.0)]));
        let drag2 = PanelDrag::for_earth(SpacecraftShape::panels(vec![make_panel(2.0)]));
        let state = iss_state();

        let t1 = drag1.loads(0.0, &state, None).torque_body;
        let t2 = drag2.loads(0.0, &state, None).torque_body;

        // τ = r × F, so doubling r doubles τ (force is the same)
        let ratio = t2.magnitude() / t1.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "Double offset should give double torque, got ratio {ratio}"
        );
    }

    // ======== Equivalence: Cannonball ↔ AtmosphericDrag ========

    #[test]
    fn cannonball_matches_atmospheric_drag() {
        use orts_orbits::drag::AtmosphericDrag;
        use orts_orbits::perturbations::ForceModel;

        let b = 0.005;
        let panel_drag = PanelDrag::for_earth(SpacecraftShape::cannonball(b));
        let atmo_drag = AtmosphericDrag::for_earth(Some(b));

        let state = iss_state();
        let panel_loads = panel_drag.loads(0.0, &state, None);
        let atmo_accel = atmo_drag.acceleration(0.0, &state.orbit, None);

        let diff = (panel_loads.acceleration_inertial - atmo_accel).magnitude();
        assert!(
            diff < 1e-15,
            "Cannonball PanelDrag should match AtmosphericDrag: diff = {diff:.3e}"
        );
    }

    // ======== Equivalence: single panel at CoM (cos θ = 1) ↔ Cannonball ========

    #[test]
    fn single_panel_facing_flow_matches_cannonball() {
        // Single panel at CoM: A=10 m², Cd=2.2, normal facing flow
        // For cannonball: B = Cd * A / (2 * m) = 2.2 * 10 / (2 * 500) = 0.022
        // Panel force:  F = -½ ρ Cd A |v|² v̂    → a = F/m = -½ ρ Cd A/m |v|² v̂
        // Cannonball:   a = -B ρ |v| v  = -(Cd*A/(2m)) ρ |v| v = -½ ρ Cd A/m |v|² v̂
        // These should be identical when cos θ = 1
        let area = 10.0;
        let cd = 2.2;
        let mass = 500.0;
        let b = cd * area / (2.0 * mass);

        // Panel facing -y (into the +y flow at identity attitude)
        let panel = SurfacePanel::at_com(area, Vector3::new(0.0, -1.0, 0.0), cd);
        let panel_drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let cannon_drag = PanelDrag::for_earth(SpacecraftShape::cannonball(b));

        let state = iss_state();
        let panel_loads = panel_drag.loads(0.0, &state, None);
        let cannon_loads = cannon_drag.loads(0.0, &state, None);

        // The accelerations should be very close but not exactly identical because:
        // - Cannonball uses v_rel in inertial frame directly
        // - Panels transform to body frame then back
        // With identity attitude, these should be numerically identical
        let diff = (panel_loads.acceleration_inertial - cannon_loads.acceleration_inertial)
            .magnitude();
        let rel = diff / cannon_loads.acceleration_inertial.magnitude();
        assert!(
            rel < 1e-10,
            "Single panel (cos θ=1) should match cannonball: relative diff = {rel:.3e}"
        );
    }

    // ======== Torque tests ========

    #[test]
    fn cube_symmetric_zero_net_torque() {
        // Symmetric cube at CoM has CP offsets, but opposite faces cancel
        // For flow in +y (identity attitude), +y face is backface, -y face is front
        // But the other 4 faces (±x, ±z) have cos(θ)=0 for exact +y flow
        // So only -y face contributes, with CP at (0, -half, 0)
        // Force is in -ŷ body: τ = (0,-h,0) × (0,F,0) = 0 (parallel!)
        let drag = PanelDrag::for_earth(SpacecraftShape::cube(0.5, 2.2));
        let loads = drag.loads(0.0, &iss_state(), None);
        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Cube with flow along axis should have zero torque (CP parallel to force)"
        );
    }

    // ======== Quantitative attitude coupling ========

    /// Helper: make a quaternion for rotation by `angle` about the given axis.
    fn quat_from_axis_angle(axis: Vector3<f64>, angle: f64) -> Vector4<f64> {
        let half = angle / 2.0;
        let (s, c) = half.sin_cos();
        let a = axis.normalize();
        Vector4::new(c, a.x * s, a.y * s, a.z * s)
    }

    #[test]
    fn cos_theta_scaling_45_degrees() {
        // Rotate 45° about x: cos θ = cos(45°) = √2/2
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let s0 = iss_state(); // identity attitude
        let mut s45 = iss_state();
        s45.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_4);

        let a0 = drag.loads(0.0, &s0, None).acceleration_inertial.magnitude();
        let a45 = drag.loads(0.0, &s45, None).acceleration_inertial.magnitude();

        let ratio = a45 / a0;
        let expected = std::f64::consts::FRAC_PI_4.cos(); // cos(45°) = √2/2
        assert!(
            (ratio - expected).abs() < 1e-10,
            "45° rotation: expected ratio {expected:.6}, got {ratio:.6}"
        );
    }

    #[test]
    fn cos_theta_scaling_60_degrees() {
        // Rotate 60° about x: cos θ = cos(60°) = 0.5
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let s0 = iss_state();
        let mut s60 = iss_state();
        s60.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_3);

        let a0 = drag.loads(0.0, &s0, None).acceleration_inertial.magnitude();
        let a60 = drag.loads(0.0, &s60, None).acceleration_inertial.magnitude();

        let ratio = a60 / a0;
        assert!(
            (ratio - 0.5).abs() < 1e-10,
            "60° rotation: expected ratio 0.5, got {ratio:.6}"
        );
    }

    #[test]
    fn cos_theta_scaling_90_degrees_zero() {
        // Rotate 90° about x: cos θ = 0 → zero drag
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let mut s90 = iss_state();
        s90.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2);

        let a = drag.loads(0.0, &s90, None).acceleration_inertial.magnitude();
        assert!(
            a < 1e-20,
            "90° rotation: expected zero drag, got {a:.3e}"
        );
    }

    #[test]
    fn force_direction_always_anti_velocity() {
        // Pure drag invariant: a_inertial ∥ -v_rel for any attitude with nonzero drag.
        // Proof: F_body ∝ -v̂_body → a_inertial = R_ib*(-K·v̂_body) = -K·v̂_inertial
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let angles = [0.0, 0.3, 0.7, 1.0, -0.5];
        let axes = [
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(1.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 1.0),
        ];

        let base = iss_state();
        let v_rel = base.orbit.velocity
            - Vector3::new(0.0, 0.0, OMEGA_EARTH).cross(&base.orbit.position);

        for (axis, angle) in axes.iter().zip(angles.iter()) {
            let mut state = iss_state();
            state.attitude.quaternion = quat_from_axis_angle(*axis, *angle);

            let loads = drag.loads(0.0, &state, None);
            let a = loads.acceleration_inertial;

            if a.magnitude() < 1e-20 {
                continue; // backface, direction undefined
            }

            // Check: a × v_rel ≈ 0 (parallel)
            let cross = a.cross(&v_rel);
            let cross_rel = cross.magnitude() / (a.magnitude() * v_rel.magnitude());
            assert!(
                cross_rel < 1e-10,
                "axis={axis:?}, angle={angle}: force not parallel to -v_rel, |a×v|/|a||v| = {cross_rel:.3e}"
            );

            // Check: a · v_rel < 0 (opposing)
            assert!(
                a.dot(&v_rel) < 0.0,
                "axis={axis:?}, angle={angle}: force not opposing velocity"
            );
        }
    }

    #[test]
    fn energy_dissipation_always_negative() {
        // F · v_rel ≤ 0 for drag at any attitude (energy is always removed)
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        for i in 0..20 {
            let angle = (i as f64) * std::f64::consts::PI / 10.0; // 0 to 2π
            let mut state = iss_state();
            state.attitude.quaternion =
                quat_from_axis_angle(Vector3::new(1.0, 1.0, 1.0), angle);

            let loads = drag.loads(0.0, &state, None);
            let a = loads.acceleration_inertial;
            let v_rel = state.orbit.velocity
                - Vector3::new(0.0, 0.0, OMEGA_EARTH).cross(&state.orbit.position);

            let power = a.dot(&v_rel); // F·v / m, proportional to power
            assert!(
                power <= 0.0,
                "Drag should always dissipate energy: angle={angle:.2}, F·v = {power:.3e}"
            );
        }
    }

    #[test]
    fn two_sided_panel_no_dead_zone() {
        // Two panels with opposite normals (±y): at least one faces the flow at any attitude
        let panels = vec![
            SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2),
            SurfacePanel::at_com(10.0, Vector3::new(0.0, 1.0, 0.0), 2.2),
        ];
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(panels));

        // At identity: -y panel faces +y flow → drag
        let a0 = drag
            .loads(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();
        assert!(a0 > 0.0);

        // At 180° about z: +y panel faces +y flow → same drag magnitude
        let mut s180 = iss_state();
        s180.attitude.quaternion = Vector4::new(0.0, 0.0, 0.0, 1.0);
        let a180 = drag
            .loads(0.0, &s180, None)
            .acceleration_inertial
            .magnitude();
        assert!(
            (a0 - a180).abs() / a0 < 1e-10,
            "Two-sided panel should have same drag at 0° and 180°: a0={a0:.3e}, a180={a180:.3e}"
        );

        // At 45° about x: only -y panel contributes (cos θ = cos45),
        // +y panel has cos θ = -cos45 → clamped to 0.
        // Opposite normals never both face the flow simultaneously:
        //   max(cosθ, 0) + max(-cosθ, 0) = |cosθ|
        let mut s45 = iss_state();
        s45.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_4);
        let a45 = drag
            .loads(0.0, &s45, None)
            .acceleration_inertial
            .magnitude();
        let ratio = a45 / a0;
        let expected = std::f64::consts::FRAC_PI_4.cos(); // cos(45°) = √2/2
        assert!(
            (ratio - expected).abs() < 1e-10,
            "Two-sided at 45°: expected ratio {expected:.6}, got {ratio:.6}"
        );

        // At 90° about x: flow perpendicular to both normals → zero drag
        let mut s90 = iss_state();
        s90.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2);
        let a90 = drag
            .loads(0.0, &s90, None)
            .acceleration_inertial
            .magnitude();
        assert!(
            a90 < 1e-20,
            "Two-sided at 90° about x: both panels perpendicular → zero, got {a90:.3e}"
        );
    }

    #[test]
    fn cube_projected_area_analytic() {
        // For a cube (6 faces ±x,±y,±z), the total projected area in direction v̂ is:
        // A_proj = A * (|v̂_x| + |v̂_y| + |v̂_z|) in body frame
        // At identity: v̂_body = (0,1,0) → A_proj = A * 1 = A
        // At 45° about z: v̂_body = (sin45, cos45, 0) → A_proj = A * (sin45 + cos45) = A * √2
        let half = 0.5;
        let cd = 2.2;
        let drag = PanelDrag::for_earth(SpacecraftShape::cube(half, cd));

        let a0 = drag
            .loads(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();

        // 45° about z: v̂_body has components in both x and y
        let mut s45z = iss_state();
        s45z.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_4);
        let a45z = drag
            .loads(0.0, &s45z, None)
            .acceleration_inertial
            .magnitude();

        let ratio = a45z / a0;
        let expected = std::f64::consts::SQRT_2; // (sin45 + cos45) / 1
        assert!(
            (ratio - expected).abs() < 1e-10,
            "Cube at 45° about z: expected ratio {expected:.6}, got {ratio:.6}"
        );
    }

    #[test]
    fn torque_exact_cross_product() {
        // Panel with known offset: verify τ = r × F exactly
        // Setup: offset (1,0,0) m, flow in +y → force in -y body
        // τ = (1,0,0) × (0,F_y,0) = (0*0-0*F_y, 0*0-1*0, 1*F_y-0*0) = (0,0,F_y)
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, -1.0, 0.0),
            cd: 2.2,
            cp_offset: Vector3::new(1.0, 0.0, 0.0),
        };
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.loads(0.0, &iss_state(), None);

        // Reconstruct F_body from acceleration: F = a_body * mass
        // a_inertial = R_ib * (F_body / mass) / 1000
        // At identity: R_ib = I, so a_inertial = F_body / (mass * 1000)
        let f_body_y = loads.acceleration_inertial.y * iss_state().mass * 1000.0; // N

        // Expected torque: τ = r × F = (1,0,0) × (0,F_y,0) = (0, 0, F_y)
        assert!(
            loads.torque_body.x.abs() < 1e-20,
            "τ_x should be 0, got {:.3e}",
            loads.torque_body.x
        );
        assert!(
            loads.torque_body.y.abs() < 1e-20,
            "τ_y should be 0, got {:.3e}",
            loads.torque_body.y
        );
        let rel_err = (loads.torque_body.z - f_body_y).abs() / f_body_y.abs();
        assert!(
            rel_err < 1e-10,
            "τ_z should equal F_y ({f_body_y:.6e}), got {:.6e}, rel_err={rel_err:.3e}",
            loads.torque_body.z
        );
    }

    // ======== Mock atmosphere for isolated frame-transform tests ========

    /// Constant density regardless of altitude/position/epoch.
    struct ConstantDensity(f64);

    impl AtmosphereModel for ConstantDensity {
        fn density(&self, _alt: f64, _pos: &Vector3<f64>, _epoch: Option<&Epoch>) -> f64 {
            self.0
        }
    }

    /// Compose two quaternions: result represents R(q_second) * R(q_first).
    ///
    /// Delegates to nalgebra `UnitQuaternion` multiplication to avoid
    /// hand-coding Hamilton product with nalgebra's confusing Vector4 accessors
    /// (`.x`→[0], `.w`→[3] do NOT match quaternion component names).
    fn quat_compose(q_second: &Vector4<f64>, q_first: &Vector4<f64>) -> Vector4<f64> {
        use nalgebra::{Quaternion, UnitQuaternion};
        let uq_second = UnitQuaternion::from_quaternion(Quaternion::new(
            q_second[0], q_second[1], q_second[2], q_second[3],
        ));
        let uq_first = UnitQuaternion::from_quaternion(Quaternion::new(
            q_first[0], q_first[1], q_first[2], q_first[3],
        ));
        let result = uq_second * uq_first;
        Vector4::new(result.w, result.i, result.j, result.k)
    }

    /// Build a PanelDrag with constant density, no co-rotation, spherical body.
    /// Isolates pure frame-transformation physics from atmosphere position dependence.
    fn mock_drag(shape: SpacecraftShape, rho: f64) -> PanelDrag {
        PanelDrag {
            shape,
            atmosphere: Box::new(ConstantDensity(rho)),
            body: None,
            body_radius: 100.0, // well inside any test orbit
            omega_body: 0.0,    // no co-rotation
        }
    }

    #[test]
    fn equivariance_acceleration_under_inertial_rotation() {
        // With constant density and no co-rotation, rotating the entire scenario
        // (position, velocity, attitude) by R in inertial frame should rotate
        // the acceleration by R: a' = R · a.
        //
        // Proof: v_body' = R_bi' · v_rel' = (R·R_ib)^T · R·v = R_bi · v = v_body
        // So per-panel forces in body frame are identical ⟹ a_body' = a_body
        // ⟹ a_inertial' = R_ib' · a_body = R · R_ib · a_body = R · a_inertial  ∎
        let panels = vec![
            SurfacePanel {
                area: 10.0,
                normal: Vector3::new(0.0, -1.0, 0.0),
                cd: 2.2,
                cp_offset: Vector3::new(1.0, 0.0, 0.0),
            },
            SurfacePanel::at_com(5.0, Vector3::new(1.0, 0.0, 0.0), 2.0),
        ];
        let drag = mock_drag(SpacecraftShape::panels(panels), 1e-12);

        // Original state with non-trivial attitude
        let mut s1 = iss_state();
        s1.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 1.0, 0.0), 0.5);
        let l1 = drag.loads(0.0, &s1, None);

        // Apply arbitrary rotation R (37° about (1,2,3))
        let q_r = quat_from_axis_angle(Vector3::new(1.0, 2.0, 3.0), 37.0_f64.to_radians());
        let r_mat = AttitudeState {
            quaternion: q_r,
            angular_velocity: Vector3::zeros(),
        }
        .rotation_matrix();

        let s2 = SpacecraftState {
            orbit: State {
                position: r_mat * s1.orbit.position,
                velocity: r_mat * s1.orbit.velocity,
            },
            attitude: AttitudeState {
                quaternion: quat_compose(&q_r, &s1.attitude.quaternion),
                angular_velocity: s1.attitude.angular_velocity,
            },
            mass: s1.mass,
        };
        let l2 = drag.loads(0.0, &s2, None);

        // a' should equal R · a
        let a1_rotated = r_mat * l1.acceleration_inertial;
        let a_rel = (l2.acceleration_inertial - a1_rotated).magnitude()
            / l1.acceleration_inertial.magnitude();
        assert!(
            a_rel < 1e-10,
            "Acceleration should transform as R·a: relative error = {a_rel:.3e}"
        );
    }

    #[test]
    fn equivariance_torque_under_inertial_rotation() {
        // Body-frame torque should be invariant under inertial rotation,
        // since v_body is unchanged and all panel calculations happen in body frame.
        let panels = vec![
            SurfacePanel {
                area: 10.0,
                normal: Vector3::new(0.0, -1.0, 0.0),
                cd: 2.2,
                cp_offset: Vector3::new(1.0, 0.0, 0.0),
            },
            SurfacePanel {
                area: 8.0,
                normal: Vector3::new(1.0, 0.0, 0.0),
                cd: 2.0,
                cp_offset: Vector3::new(0.0, 0.0, 0.5),
            },
        ];
        let drag = mock_drag(SpacecraftShape::panels(panels), 1e-12);

        let mut s1 = iss_state();
        s1.attitude.quaternion = quat_from_axis_angle(Vector3::new(0.0, 1.0, 0.0), 0.8);
        let l1 = drag.loads(0.0, &s1, None);

        // Multiple arbitrary rotations
        let rotations = [
            (Vector3::new(1.0, 0.0, 0.0), 45.0_f64),
            (Vector3::new(0.0, 1.0, 0.0), 120.0),
            (Vector3::new(1.0, 2.0, 3.0), 37.0),
            (Vector3::new(-1.0, 0.5, 0.3), 200.0),
        ];

        for (axis, angle_deg) in &rotations {
            let q_r = quat_from_axis_angle(*axis, angle_deg.to_radians());
            let r_mat = AttitudeState {
                quaternion: q_r,
                angular_velocity: Vector3::zeros(),
            }
            .rotation_matrix();

            let s2 = SpacecraftState {
                orbit: State {
                    position: r_mat * s1.orbit.position,
                    velocity: r_mat * s1.orbit.velocity,
                },
                attitude: AttitudeState {
                    quaternion: quat_compose(&q_r, &s1.attitude.quaternion),
                    angular_velocity: s1.attitude.angular_velocity,
                },
                mass: s1.mass,
            };
            let l2 = drag.loads(0.0, &s2, None);

            let tau_rel = (l2.torque_body - l1.torque_body).magnitude()
                / l1.torque_body.magnitude();
            assert!(
                tau_rel < 1e-10,
                "Body-frame torque should be invariant under {angle_deg}° about {axis:?}: \
                 relative error = {tau_rel:.3e}"
            );
        }
    }

    #[test]
    fn convention_anchor_yaw_positive_backface() {
        // Convention anchor: distinguishes R_bi from R_ib (would fail under transpose).
        //
        // Panel normal n_b = (1,0,0). Flow +y inertial.
        // +90° yaw about z: R_ib maps body_x → inertial_y.
        //   → Correct R_bi: v_body = R_bi * (0,v,0) = (v,0,0)
        //     cos θ = n_b · (-v̂_body) = (1,0,0)·(-1,0,0) = -1 → backface → ZERO
        //   → Wrong (R_ib): v_body = R_ib * (0,v,0) = (-v,0,0)
        //     cos θ = (1,0,0)·(1,0,0) = +1 → FULL drag
        let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2);
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), 1e-12);

        let mut state = iss_state();
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2);

        let loads = drag.loads(0.0, &state, None);
        assert!(
            loads.acceleration_inertial.magnitude() < 1e-20,
            "Convention anchor: +90° yaw with n_b=(1,0,0) should be backface (zero drag), \
             got {:.3e}. This indicates R_ib/R_bi swap.",
            loads.acceleration_inertial.magnitude()
        );
    }

    #[test]
    fn convention_anchor_yaw_negative_full_drag() {
        // Complement of the above: -90° yaw → front face → full drag.
        //   R_ib maps body_x → inertial -y.
        //   Correct R_bi: v_body = R_bi * (0,v,0) = (-v,0,0)
        //     cos θ = (1,0,0)·(1,0,0) = +1 → full drag
        let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2);
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), 1e-12);

        let mut state = iss_state();
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), -std::f64::consts::FRAC_PI_2);

        let loads = drag.loads(0.0, &state, None);
        assert!(
            loads.acceleration_inertial.magnitude() > 1e-20,
            "Convention anchor: -90° yaw with n_b=(1,0,0) should be full drag"
        );

        // Verify magnitude matches the identity case for a -y normal panel
        // (which faces the +y flow at identity). Both should give same exposure.
        let panel_y = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag_y = mock_drag(SpacecraftShape::panels(vec![panel_y]), 1e-12);
        let ref_loads = drag_y.loads(0.0, &iss_state(), None);

        let rel = (loads.acceleration_inertial.magnitude()
            - ref_loads.acceleration_inertial.magnitude())
            .abs()
            / ref_loads.acceleration_inertial.magnitude();
        assert!(
            rel < 1e-10,
            "Full-drag magnitudes should match: relative diff = {rel:.3e}"
        );
    }

    #[test]
    fn quaternion_sign_invariance() {
        // q and -q represent the same rotation.
        // PanelDrag should produce identical forces and torques.
        let panels = vec![
            SurfacePanel {
                area: 10.0,
                normal: Vector3::new(0.0, -1.0, 0.0),
                cd: 2.2,
                cp_offset: Vector3::new(1.0, 0.0, 0.0),
            },
            SurfacePanel::at_com(5.0, Vector3::new(1.0, 0.0, 0.0), 2.0),
        ];
        let drag = mock_drag(SpacecraftShape::panels(panels), 1e-12);

        let mut s1 = iss_state();
        s1.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 2.0, 3.0), 0.7);

        let mut s2 = s1.clone();
        s2.attitude.quaternion = -s1.attitude.quaternion; // -q

        let l1 = drag.loads(0.0, &s1, None);
        let l2 = drag.loads(0.0, &s2, None);

        assert!(
            (l1.acceleration_inertial - l2.acceleration_inertial).magnitude() < 1e-15,
            "q and -q should give identical acceleration"
        );
        assert!(
            (l1.torque_body - l2.torque_body).magnitude() < 1e-15,
            "q and -q should give identical torque"
        );
    }

    #[test]
    fn density_linearity() {
        // a ∝ ρ: doubling density doubles acceleration and torque
        let panels = vec![SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, -1.0, 0.0),
            cd: 2.2,
            cp_offset: Vector3::new(1.0, 0.0, 0.0),
        }];

        let drag1 = mock_drag(SpacecraftShape::panels(panels.clone()), 1e-12);
        let drag2 = mock_drag(SpacecraftShape::panels(panels), 2e-12);
        let state = iss_state();

        let l1 = drag1.loads(0.0, &state, None);
        let l2 = drag2.loads(0.0, &state, None);

        let a_ratio = l2.acceleration_inertial.magnitude() / l1.acceleration_inertial.magnitude();
        assert!(
            (a_ratio - 2.0).abs() < 1e-10,
            "Acceleration should scale linearly with density: ratio = {a_ratio:.6}"
        );

        let tau_ratio = l2.torque_body.magnitude() / l1.torque_body.magnitude();
        assert!(
            (tau_ratio - 2.0).abs() < 1e-10,
            "Torque should scale linearly with density: ratio = {tau_ratio:.6}"
        );
    }

    #[test]
    fn velocity_squared_scaling() {
        // a ∝ |v|² (at constant density, same direction)
        // Use mock to eliminate altitude-dependent density changes
        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), 1e-12);

        let s1 = iss_state();
        let mut s2 = iss_state();
        // Scale velocity by 2x (keep position same → same density with mock)
        s2.orbit.velocity = s1.orbit.velocity * 2.0;

        let a1 = drag.loads(0.0, &s1, None).acceleration_inertial.magnitude();
        let a2 = drag.loads(0.0, &s2, None).acceleration_inertial.magnitude();

        // a ∝ |v|² → ratio should be 4
        let ratio = a2 / a1;
        assert!(
            (ratio - 4.0).abs() < 1e-10,
            "Acceleration should scale as |v|²: ratio = {ratio:.6} (expected 4.0)"
        );
    }

    #[test]
    fn absolute_magnitude_analytic() {
        // For single panel at CoM with cos θ = 1 and constant density:
        //   |a| = ½ ρ Cd A |v|² / m   [m/s²]
        //   |a_km| = |a| / 1000       [km/s²]
        let area = 10.0; // m²
        let cd = 2.2;
        let mass = 500.0; // kg
        let rho = 1e-12; // kg/m³

        let panel = SurfacePanel::at_com(area, Vector3::new(0.0, -1.0, 0.0), cd);
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), rho);

        let state = iss_state();
        let loads = drag.loads(0.0, &state, None);

        // With mock (no co-rotation), v_rel = v
        let v_ms = state.orbit.velocity.magnitude() * 1000.0; // m/s
        let expected_a_ms2 = 0.5 * rho * cd * area * v_ms * v_ms / mass;
        let expected_a_kms2 = expected_a_ms2 / 1000.0;

        let actual = loads.acceleration_inertial.magnitude();
        let rel_err = (actual - expected_a_kms2).abs() / expected_a_kms2;
        assert!(
            rel_err < 1e-10,
            "Absolute acceleration: expected {expected_a_kms2:.6e}, got {actual:.6e}, \
             rel_err = {rel_err:.3e}"
        );
    }

    // ======== SpacecraftDynamics integration ========

    #[test]
    fn panels_integrable_with_rk4() {
        use nalgebra::Matrix3;
        use orts_integrator::{Integrator, OdeState, Rk4};
        use orts_orbits::gravity::PointMass;
        use orts_orbits::constants::MU_EARTH;
        use super::super::SpacecraftDynamics;

        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_load(Box::new(drag));

        let result = Rk4.integrate(&dyn_sc, iss_state(), 0.0, 60.0, 1.0, |_, _| {});
        assert!(result.is_finite(), "State should remain finite after 60s integration");
        assert!(result.orbit.position.magnitude() > 0.0);
    }

    #[test]
    fn panels_drag_reduces_orbital_energy() {
        use nalgebra::Matrix3;
        use orts_integrator::{Integrator, Rk4};
        use orts_orbits::gravity::PointMass;
        use orts_orbits::constants::MU_EARTH;
        use super::super::SpacecraftDynamics;

        let panel = SurfacePanel::at_com(10.0, Vector3::new(0.0, -1.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_load(Box::new(drag));

        let s0 = iss_state();
        let e0 = 0.5 * s0.orbit.velocity.magnitude_squared()
            - MU_EARTH / s0.orbit.position.magnitude();

        let s1 = Rk4.integrate(&dyn_sc, s0, 0.0, 300.0, 1.0, |_, _| {});
        let e1 = 0.5 * s1.orbit.velocity.magnitude_squared()
            - MU_EARTH / s1.orbit.position.magnitude();

        assert!(
            e1 < e0,
            "Drag should reduce orbital energy: e0={e0:.6e}, e1={e1:.6e}"
        );
    }

    #[test]
    fn tumbling_asymmetric_panels_varying_drag() {
        use nalgebra::Matrix3;
        use orts_integrator::{Integrator, Rk4};
        use orts_orbits::gravity::PointMass;
        use orts_orbits::constants::MU_EARTH;
        use super::super::SpacecraftDynamics;

        // Asymmetric panel: only one face, so drag depends on orientation
        let panel = SurfacePanel::at_com(20.0, Vector3::new(1.0, 0.0, 0.0), 2.2);
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_load(Box::new(drag));

        // Give it a tumble
        let mut state = iss_state();
        state.attitude.angular_velocity = Vector3::new(0.0, 0.0, 0.05); // slow tumble about z

        // Collect drag magnitude at several steps to verify it varies
        let mut magnitudes = Vec::new();
        let _ = Rk4.integrate(&dyn_sc, state, 0.0, 60.0, 1.0, |_t, s| {
            let loads = dyn_sc.load_breakdown(0.0, s);
            if let Some((_, el)) = loads.first() {
                magnitudes.push(el.acceleration_inertial.magnitude());
            }
        });

        // Should have varying magnitudes (not all the same)
        let min = magnitudes.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = magnitudes.iter().cloned().fold(0.0_f64, f64::max);
        assert!(
            max > min * 1.01 || min == 0.0,
            "Tumbling should cause varying drag: min={min:.3e}, max={max:.3e}"
        );
    }

    #[test]
    fn cannonball_integrable_with_spacecraft_dynamics() {
        use nalgebra::Matrix3;
        use orts_integrator::{Integrator, OdeState, Rk4};
        use orts_orbits::gravity::PointMass;
        use orts_orbits::constants::MU_EARTH;
        use super::super::SpacecraftDynamics;

        let drag = PanelDrag::for_earth(SpacecraftShape::cannonball(0.01));
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 10.0, 10.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_load(Box::new(drag));

        let result = Rk4.integrate(&dyn_sc, iss_state(), 0.0, 60.0, 1.0, |_, _| {});
        assert!(result.is_finite());
    }
}
