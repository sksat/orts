use std::f64::consts::PI;

use nalgebra::{Matrix3, Vector3};
use utsuroi::{Integrator, Rk4};

use arika::earth::{MU as MU_EARTH, R as R_EARTH};
use arika::epoch::Epoch;
use orts::attitude::{AttitudeState, BdotDetumbler, DecoupledAttitudeSystem};
use tobari::magnetic::TiltedDipole;

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

/// Rotational kinetic energy T = 0.5 * omega . (I . omega)
fn rotational_energy(state: &AttitudeState, inertia: &Matrix3<f64>) -> f64 {
    0.5 * state
        .angular_velocity
        .dot(&(inertia * state.angular_velocity))
}

fn test_epoch() -> Epoch {
    Epoch::j2000()
}

// ------
// Test 1: Magnetic field validation
// ------

#[test]
fn magnetic_field_magnitude_at_equatorial_leo() {
    use arika::earth::ellipsoid::WGS84_A;
    use arika::earth::geodetic::Geodetic;
    use tobari::magnetic::{MagneticFieldInput, MagneticFieldModel};

    let dipole = TiltedDipole::earth();
    let epoch = test_epoch();
    let input = MagneticFieldInput {
        geodetic: Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 7000.0 - WGS84_A,
        },
        utc: &epoch,
    };
    let b = dipole.field_ecef(&input);
    let b_micro_t = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt() * 1e6;

    assert!(
        b_micro_t > 20.0 && b_micro_t < 50.0,
        "|B| at 7000 km equatorial should be ~25-35 uT, got {b_micro_t:.2} uT"
    );
}

#[test]
fn magnetic_field_inverse_cube_law() {
    use arika::earth::ellipsoid::WGS84_A;
    use arika::earth::geodetic::Geodetic;
    use tobari::magnetic::{MagneticFieldInput, MagneticFieldModel};

    let dipole = TiltedDipole::earth();
    let epoch = test_epoch();
    let b_near = {
        let b = dipole.field_ecef(&MagneticFieldInput {
            geodetic: Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 7000.0 - WGS84_A,
            },
            utc: &epoch,
        });
        (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt()
    };
    let b_far = {
        let b = dipole.field_ecef(&MagneticFieldInput {
            geodetic: Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 14000.0 - WGS84_A,
            },
            utc: &epoch,
        });
        (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt()
    };

    let ratio = b_near / b_far;
    assert!(
        (ratio - 8.0).abs() < 0.1,
        "Expected ~1/r^3 ratio of ~8.0, got {ratio:.4}"
    );
}

// ------
// Test 2: B-dot angular velocity reduction (non-saturated)
// ------

#[test]
fn bdot_reduces_angular_velocity() {
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let altitude = 400.0; // km
    let radius = R_EARTH + altitude;
    let n = (MU_EARTH / radius.powi(3)).sqrt();
    let period = 2.0 * PI / n;

    let omega0 = Vector3::new(0.1, 0.2, 0.05);
    let omega0_mag = omega0.magnitude();
    let initial = AttitudeState::new(nalgebra::UnitQuaternion::identity(), omega0);

    let bdot = BdotDetumbler::new(1e6, Vector3::new(10.0, 10.0, 10.0), TiltedDipole::earth());
    let system = DecoupledAttitudeSystem::circular_orbit(inertia, MU_EARTH, radius, 500.0)
        .with_model(bdot)
        .with_epoch(test_epoch());

    let dt = 1.0;
    let t_end = 3.0 * period; // ~16500 s

    let e0 = rotational_energy(&initial, &inertia);

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

    let final_omega_mag = final_state.angular_velocity.magnitude();
    let final_energy = rotational_energy(&final_state, &inertia);

    // Assert: final |omega| < 0.5 * initial |omega|
    assert!(
        final_omega_mag < 0.5 * omega0_mag,
        "B-dot should reduce |omega| by at least half after 3 orbits: \
         initial={omega0_mag:.4}, final={final_omega_mag:.4}"
    );

    // Assert: final energy < initial energy
    assert!(
        final_energy < e0,
        "Final rotational energy should be less than initial: \
         E_0={e0:.6e}, E_f={final_energy:.6e}"
    );
}

// ------
// Test 3: B-dot energy dissipation (1 orbit)
// ------

#[test]
fn bdot_energy_dissipation_one_orbit() {
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let altitude = 400.0;
    let radius = R_EARTH + altitude;
    let n = (MU_EARTH / radius.powi(3)).sqrt();
    let period = 2.0 * PI / n;

    let omega0 = Vector3::new(0.1, 0.2, 0.05);
    let initial = AttitudeState::new(nalgebra::UnitQuaternion::identity(), omega0);

    let bdot = BdotDetumbler::new(1e6, Vector3::new(10.0, 10.0, 10.0), TiltedDipole::earth());
    let system = DecoupledAttitudeSystem::circular_orbit(inertia, MU_EARTH, radius, 500.0)
        .with_model(bdot)
        .with_epoch(test_epoch());

    let e0 = rotational_energy(&initial, &inertia);

    let dt = 1.0;
    let t_end = period; // 1 orbit

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

    let e_final = rotational_energy(&final_state, &inertia);

    assert!(
        e_final < e0,
        "Energy should decrease after 1 orbit: E_0={e0:.6e}, E_f={e_final:.6e}"
    );
}

// ------
// Test 4: Instantaneous torque opposes omega (Cauchy-Schwarz)
// ------

#[test]
fn bdot_instantaneous_torque_opposes_omega() {
    // For the unsaturated B-dot law:
    //   m = k (omega x B)
    //   tau = m x B = k [(omega x B) x B] = k [B(omega.B) - omega|B|^2]
    //   omega . tau = k [(omega.B)^2 - |omega|^2|B|^2] <= 0  by Cauchy-Schwarz
    //
    // This must hold for ANY orientation and position.

    let gain = 1e4;
    // Use large max_moment so nothing is clamped
    let ctrl = BdotDetumbler::new(
        gain,
        Vector3::new(100.0, 100.0, 100.0),
        TiltedDipole::earth(),
    );
    let epoch = test_epoch();

    // Test at several different orientations and positions
    let test_cases: Vec<(Vector3<f64>, Vector3<f64>)> = vec![
        (Vector3::new(0.1, 0.2, 0.05), Vector3::new(7000.0, 0.0, 0.0)),
        (Vector3::new(0.5, -0.3, 0.1), Vector3::new(0.0, 7000.0, 0.0)),
        (
            Vector3::new(-0.1, 0.0, 0.4),
            Vector3::new(4000.0, 4000.0, 3000.0),
        ),
        (
            Vector3::new(0.01, 0.01, 0.01),
            Vector3::new(6778.0, 0.0, 0.0),
        ),
    ];

    for (omega, pos) in &test_cases {
        // Test with a non-trivial orientation
        let axis = nalgebra::Unit::new_normalize(Vector3::new(1.0, 2.0, 3.0));
        let uq = nalgebra::UnitQuaternion::from_axis_angle(&axis, 0.7);

        let state = orts::attitude::DecoupledContext {
            attitude: AttitudeState::new(uq, *omega),
            orbit: orts::OrbitalState::new(*pos, Vector3::zeros()),
            mass: 100.0,
        };

        let loads = <BdotDetumbler as orts::model::Model<orts::attitude::DecoupledContext>>::eval(
            &ctrl,
            0.0,
            &state,
            Some(&epoch),
        );
        let dot = omega.dot(&(uq.inverse() * loads.torque_body.into_inner()));
        // Since torque is in body frame and omega is in body frame,
        // we can just dot them directly.
        let dot_body = state
            .attitude
            .angular_velocity
            .dot(&loads.torque_body.into_inner());
        assert!(
            dot_body <= 1e-20, // allow tiny positive due to floating point
            "omega . tau should be <= 0 for omega={omega:?}, pos={pos:?}: got {dot_body:.6e}"
        );

        // Also verify non-trivial torque when omega is non-zero
        if omega.magnitude() > 1e-10 {
            assert!(
                loads.torque_body.magnitude() > 0.0,
                "Torque should be non-zero for non-zero omega"
            );
        }

        // Unused variable cleanup
        let _ = dot;
    }
}
