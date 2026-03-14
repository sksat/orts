use orts_integrator::{DynamicalSystem, State};

/// Two-body gravitational system.
///
/// Computes gravitational acceleration a = -μ/|r|³ * r
/// for a body orbiting a central mass with gravitational parameter μ.
pub struct TwoBodySystem {
    pub mu: f64,
}

impl DynamicalSystem for TwoBodySystem {
    type State = State;
    fn derivatives(&self, _t: f64, state: &State) -> State {
        let r = &state.position;
        let r_mag = r.magnitude();
        let acceleration = -self.mu / (r_mag * r_mag * r_mag) * r;
        State::from_derivative(state.velocity, acceleration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaname::constants::{MU_EARTH, R_EARTH};
    use nalgebra::vector;

    #[test]
    fn test_acceleration_direction() {
        // Acceleration should be antiparallel to position (pointing toward center)
        let system = TwoBodySystem { mu: MU_EARTH };
        let state = State {
            position: vector![6778.137, 0.0, 0.0],
            velocity: vector![0.0, 7.6693, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);

        // Dot product of acceleration and position should be negative (antiparallel)
        let dot = deriv.velocity.dot(&state.position);
        assert!(dot < 0.0, "acceleration should point toward center (dot={dot})");

        // Cross product should be approximately zero (parallel/antiparallel vectors)
        let cross = deriv.velocity.cross(&state.position);
        assert!(
            cross.magnitude() < 1e-10,
            "acceleration should be collinear with position (cross mag={})",
            cross.magnitude()
        );
    }

    #[test]
    fn test_acceleration_magnitude() {
        // |a| = μ/|r|² for a known position
        let system = TwoBodySystem { mu: MU_EARTH };
        let r = vector![6778.137, 0.0, 0.0];
        let state = State {
            position: r,
            velocity: vector![0.0, 7.6693, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);

        let r_mag = r.magnitude();
        let expected_mag = MU_EARTH / (r_mag * r_mag);
        let actual_mag = deriv.velocity.magnitude();

        let rel_err = (actual_mag - expected_mag).abs() / expected_mag;
        assert!(
            rel_err < 1e-12,
            "acceleration magnitude mismatch: expected={expected_mag}, actual={actual_mag}, rel_err={rel_err}"
        );
    }

    #[test]
    fn test_surface_gravity() {
        // At Earth's surface, |a| ≈ 9.798e-3 km/s²
        let system = TwoBodySystem { mu: MU_EARTH };
        let state = State {
            position: vector![R_EARTH, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);

        let g = deriv.velocity.magnitude();
        let expected_g = 9.798e-3; // km/s²
        assert!(
            (g - expected_g).abs() < 0.01e-3,
            "surface gravity mismatch: expected≈{expected_g}, actual={g}"
        );
    }

    // --- Propagation tests using RK4 ---

    use orts_integrator::{Integrator, Rk4};
    use std::f64::consts::PI;

    /// Helper: set up a circular ISS orbit initial state
    fn iss_circular_orbit() -> (TwoBodySystem, State) {
        let r = R_EARTH + 400.0; // 6778.137 km
        let v = (MU_EARTH / r).sqrt();
        let system = TwoBodySystem { mu: MU_EARTH };
        let state = State {
            position: vector![r, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };
        (system, state)
    }

    /// Helper: propagate for a given duration with step size dt
    fn propagate(system: &TwoBodySystem, initial: &State, dt: f64, duration: f64) -> Vec<State> {
        let n_steps = (duration / dt).round() as usize;
        let mut states = Vec::with_capacity(n_steps + 1);
        let mut state = initial.clone();
        let mut t = 0.0;
        states.push(state.clone());
        for _ in 0..n_steps {
            state = Rk4.step(system, t, &state, dt);
            t += dt;
            states.push(state.clone());
        }
        states
    }

    #[test]
    fn test_circular_orbit_period() {
        // Propagate for one full period T, satellite should return near start
        let (system, initial) = iss_circular_orbit();
        let r = initial.position.magnitude();
        let period = 2.0 * PI * (r.powi(3) / MU_EARTH).sqrt();
        let dt = 1.0; // 1 second steps

        let states = propagate(&system, &initial, dt, period);
        let final_state = states.last().unwrap();

        let pos_err = (final_state.position - initial.position).magnitude();
        let vel_err = (final_state.velocity - initial.velocity).magnitude();

        // RK4 with dt=1s over ~5554s should return to start within ~5 km
        // (cumulative 4th-order truncation error over ~5554 steps)
        assert!(
            pos_err < 5.0,
            "after one period, position error = {pos_err} km (should be <5 km)"
        );
        assert!(
            vel_err < 5e-3,
            "after one period, velocity error = {vel_err} km/s (should be <5e-3 km/s)"
        );
    }

    #[test]
    fn test_circular_orbit_constant_radius() {
        // For a circular orbit, |r| should remain approximately constant
        let (system, initial) = iss_circular_orbit();
        let r0 = initial.position.magnitude();
        let period = 2.0 * PI * (r0.powi(3) / MU_EARTH).sqrt();
        let dt = 10.0;

        let states = propagate(&system, &initial, dt, period);

        for (i, state) in states.iter().enumerate() {
            let r = state.position.magnitude();
            let rel_err = (r - r0).abs() / r0;
            assert!(
                rel_err < 1e-6,
                "step {i}: radius {r} deviates from {r0} by {rel_err} (relative)"
            );
        }
    }

    #[test]
    fn test_energy_conservation() {
        // Specific energy E = v²/2 - μ/r should be conserved
        let (system, initial) = iss_circular_orbit();
        let r0 = initial.position.magnitude();
        let period = 2.0 * PI * (r0.powi(3) / MU_EARTH).sqrt();
        let dt = 10.0;

        let compute_energy = |s: &State| -> f64 {
            let r = s.position.magnitude();
            let v = s.velocity.magnitude();
            v * v / 2.0 - MU_EARTH / r
        };

        let e0 = compute_energy(&initial);
        let states = propagate(&system, &initial, dt, period);

        for (i, state) in states.iter().enumerate() {
            let e = compute_energy(state);
            let rel_err = (e - e0).abs() / e0.abs();
            assert!(
                rel_err < 1e-9,
                "step {i}: energy {e} deviates from {e0} by {rel_err} (relative)"
            );
        }
    }

    #[test]
    fn test_angular_momentum_conservation() {
        // |h| = |r × v| should be conserved
        let (system, initial) = iss_circular_orbit();
        let r0 = initial.position.magnitude();
        let period = 2.0 * PI * (r0.powi(3) / MU_EARTH).sqrt();
        let dt = 10.0;

        let compute_h = |s: &State| -> f64 { s.position.cross(&s.velocity).magnitude() };

        let h0 = compute_h(&initial);
        let states = propagate(&system, &initial, dt, period);

        for (i, state) in states.iter().enumerate() {
            let h = compute_h(state);
            let rel_err = (h - h0).abs() / h0;
            assert!(
                rel_err < 1e-10,
                "step {i}: angular momentum {h} deviates from {h0} by {rel_err} (relative)"
            );
        }
    }

    #[test]
    fn test_dt_convergence() {
        // RK4 is 4th order: error ratio between dt and dt/2 should be ~16
        let (system, initial) = iss_circular_orbit();
        let duration = 100.0; // short propagation

        let dt_coarse = 2.0;
        let dt_fine = 1.0;
        let dt_finest = 0.5;

        let states_coarse = propagate(&system, &initial, dt_coarse, duration);
        let states_fine = propagate(&system, &initial, dt_fine, duration);
        let states_finest = propagate(&system, &initial, dt_finest, duration);

        let final_coarse = states_coarse.last().unwrap();
        let final_fine = states_fine.last().unwrap();
        let final_finest = states_finest.last().unwrap();

        // Use finest as reference
        let err_coarse = (final_coarse.position - final_finest.position).magnitude();
        let err_fine = (final_fine.position - final_finest.position).magnitude();

        // For RK4, halving dt should reduce error by factor ~16 (2^4)
        // err_coarse / err_fine should be around 16
        let ratio = err_coarse / err_fine;
        assert!(
            ratio > 10.0 && ratio < 25.0,
            "convergence ratio = {ratio}, expected ~16 for 4th-order method \
            (err_coarse={err_coarse}, err_fine={err_fine})"
        );
    }
}
