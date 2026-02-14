use orts_integrator::{DynamicalSystem, State, StateDerivative};

use crate::gravity::GravityField;
use crate::perturbations::ForceModel;

/// Orbital dynamics system combining a gravity field model with perturbation forces.
pub struct OrbitalSystem {
    pub mu: f64,
    pub gravity: Box<dyn GravityField>,
    pub perturbations: Vec<Box<dyn ForceModel>>,
}

impl OrbitalSystem {
    pub fn new(mu: f64, gravity: Box<dyn GravityField>) -> Self {
        Self {
            mu,
            gravity,
            perturbations: Vec::new(),
        }
    }

    pub fn with_perturbation(mut self, p: Box<dyn ForceModel>) -> Self {
        self.perturbations.push(p);
        self
    }
}

impl DynamicalSystem for OrbitalSystem {
    fn derivatives(&self, t: f64, state: &State) -> StateDerivative {
        let mut accel = self.gravity.acceleration(self.mu, &state.position);
        for p in &self.perturbations {
            accel += p.acceleration(t, state);
        }
        StateDerivative {
            velocity: state.velocity,
            acceleration: accel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{J2_EARTH, MU_EARTH, R_EARTH};
    use crate::gravity::{PointMass, ZonalHarmonics};
    use crate::kepler::KeplerianElements;
    use crate::two_body::TwoBodySystem;
    use nalgebra::vector;
    use orts_integrator::Rk4;
    use std::f64::consts::PI;

    #[test]
    fn point_mass_matches_two_body_acceleration() {
        let two_body = TwoBodySystem { mu: MU_EARTH };
        let orbital = OrbitalSystem::new(MU_EARTH, Box::new(PointMass));

        let state = State {
            position: vector![6778.137, 0.0, 0.0],
            velocity: vector![0.0, 7.6693, 0.0],
        };

        let d1 = two_body.derivatives(0.0, &state);
        let d2 = orbital.derivatives(0.0, &state);

        assert_eq!(d1.velocity, d2.velocity);
        assert!((d1.acceleration - d2.acceleration).magnitude() < 1e-15);
    }

    #[test]
    fn point_mass_matches_two_body_propagation() {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let period = 2.0 * PI * (r.powi(3) / MU_EARTH).sqrt();
        let dt = 10.0;

        let initial = State {
            position: vector![r, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };

        let two_body = TwoBodySystem { mu: MU_EARTH };
        let final_tb = Rk4::integrate(&two_body, initial.clone(), 0.0, period, dt, |_, _| {});

        let orbital = OrbitalSystem::new(MU_EARTH, Box::new(PointMass));
        let final_os = Rk4::integrate(&orbital, initial, 0.0, period, dt, |_, _| {});

        // Should be bit-for-bit identical
        assert_eq!(final_tb.position, final_os.position);
        assert_eq!(final_tb.velocity, final_os.velocity);
    }

    fn earth_j2_system() -> OrbitalSystem {
        OrbitalSystem::new(
            MU_EARTH,
            Box::new(ZonalHarmonics {
                r_body: R_EARTH,
                j2: J2_EARTH,
            }),
        )
    }

    /// Propagate and return the final RAAN after duration seconds.
    fn propagate_raan(system: &OrbitalSystem, elements: &KeplerianElements, dt: f64, duration: f64) -> f64 {
        let (pos, vel) = elements.to_state_vector(MU_EARTH);
        let initial = State {
            position: pos,
            velocity: vel,
        };

        let final_state = Rk4::integrate(system, initial, 0.0, duration, dt, |_, _| {});

        let final_elements = KeplerianElements::from_state_vector(
            &final_state.position,
            &final_state.velocity,
            MU_EARTH,
        );
        final_elements.raan
    }

    #[test]
    fn j2_iss_raan_precession() {
        // ISS orbit: h=400km, i=51.6°, circular
        // Analytical RAAN precession rate:
        //   dΩ/dt = -(3/2) * n * J2 * (R_e/a)² * cos(i) / (1-e²)²
        let a = R_EARTH + 400.0; // 6778.137 km
        let i = 51.6_f64.to_radians();
        let n = (MU_EARTH / a.powi(3)).sqrt(); // mean motion [rad/s]
        let expected_rate = -1.5 * n * J2_EARTH * (R_EARTH / a).powi(2) * i.cos();
        // Convert to deg/day
        let expected_deg_per_day = expected_rate.to_degrees() * 86400.0;

        let system = earth_j2_system();
        let elements = KeplerianElements {
            semi_major_axis: a,
            eccentricity: 0.0001, // near-circular (avoid singularity)
            inclination: i,
            raan: 0.0,
            argument_of_periapsis: 0.0,
            true_anomaly: 0.0,
        };

        let duration = 86400.0; // 1 day
        let dt = 5.0; // 5s steps for accuracy
        let final_raan = propagate_raan(&system, &elements, dt, duration);

        // RAAN should have changed by approximately expected_deg_per_day degrees
        let raan_change_deg = final_raan.to_degrees();
        // Handle wrapping: if raan went negative, it wraps to ~360
        let raan_change_deg = if raan_change_deg > 180.0 {
            raan_change_deg - 360.0
        } else {
            raan_change_deg
        };

        // Expected ~-5.0 deg/day for ISS
        assert!(
            (raan_change_deg - expected_deg_per_day).abs() < 0.5,
            "ISS RAAN precession: expected≈{expected_deg_per_day:.2} deg/day, got={raan_change_deg:.2} deg/day"
        );
    }

    #[test]
    fn j2_sso_raan_precession() {
        // Sun-synchronous orbit: h=800km, i≈98.6°
        // RAAN precession rate should be ~+0.9856 deg/day (matches Earth's orbit around Sun)
        let a = R_EARTH + 800.0;
        let i = 98.6_f64.to_radians();
        let n = (MU_EARTH / a.powi(3)).sqrt();
        let expected_rate = -1.5 * n * J2_EARTH * (R_EARTH / a).powi(2) * i.cos();
        let expected_deg_per_day = expected_rate.to_degrees() * 86400.0;

        let system = earth_j2_system();
        let elements = KeplerianElements {
            semi_major_axis: a,
            eccentricity: 0.0001,
            inclination: i,
            raan: 0.0,
            argument_of_periapsis: 0.0,
            true_anomaly: 0.0,
        };

        let duration = 86400.0;
        let dt = 5.0;
        let final_raan = propagate_raan(&system, &elements, dt, duration);

        let raan_change_deg = final_raan.to_degrees();
        let raan_change_deg = if raan_change_deg > 180.0 {
            raan_change_deg - 360.0
        } else {
            raan_change_deg
        };

        // Should be close to +0.9856 deg/day
        assert!(
            (raan_change_deg - expected_deg_per_day).abs() < 0.3,
            "SSO RAAN precession: expected≈{expected_deg_per_day:.3} deg/day, got={raan_change_deg:.3} deg/day"
        );
        // Also verify it's positive (prograde precession for retrograde orbit)
        assert!(
            raan_change_deg > 0.0,
            "SSO RAAN should precess prograde, got={raan_change_deg:.3} deg/day"
        );
    }

    #[test]
    fn j2_dt_convergence() {
        // Verify RK4 4th-order convergence is maintained with J2
        let system = earth_j2_system();
        let a = R_EARTH + 400.0;
        let i = 51.6_f64.to_radians();
        let elements = KeplerianElements {
            semi_major_axis: a,
            eccentricity: 0.0001,
            inclination: i,
            raan: 0.0,
            argument_of_periapsis: 0.0,
            true_anomaly: 0.0,
        };
        let (pos, vel) = elements.to_state_vector(MU_EARTH);
        let initial = State {
            position: pos,
            velocity: vel,
        };

        let duration = 1000.0; // ~1/5 orbit

        // Propagate with three dt values
        let dt_coarse = 4.0;
        let dt_fine = 2.0;
        let dt_finest = 1.0;

        let final_coarse = Rk4::integrate(&system, initial.clone(), 0.0, duration, dt_coarse, |_, _| {});
        let final_fine = Rk4::integrate(&system, initial.clone(), 0.0, duration, dt_fine, |_, _| {});
        let final_finest = Rk4::integrate(&system, initial, 0.0, duration, dt_finest, |_, _| {});

        let err_coarse = (final_coarse.position - final_finest.position).magnitude();
        let err_fine = (final_fine.position - final_finest.position).magnitude();

        let ratio = err_coarse / err_fine;
        assert!(
            ratio > 10.0 && ratio < 25.0,
            "J2 dt convergence ratio = {ratio:.2}, expected ~16 for 4th-order (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }
}
