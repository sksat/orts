//! Benchmark: what the shadow geometry costs per derivative evaluation.
//!
//! `PanelSrp` and `PanelDrag` ask `lit_region` how much of each panel the
//! source reaches, which scans every other panel for every panel. The cost
//! depends on what the shape is, not only on how many panels it has, so the
//! cases below separate the parts:
//!
//! - `outline_free` — panels with an area and no boundary. They take no part in
//!   the geometry at all, which is the floor: the force law alone.
//! - `cube` — the six faces of a cube, which carry outlines and stand at right
//!   angles, so the scan runs and finds almost nothing.
//! - `bus_and_arrays` — the shape issue #407 measures on: a 1 m cube bus with a
//!   2 m x 1 m array either side, at a Sun angle where the bus face is partly
//!   shadowed. The full path, including the subtraction.
//! - `segmented_array` — the same bus with each array written as eight
//!   segments, 22 panels in all. Panels shadow each other in numbers here, so
//!   this is where the quadratic scan and the piece count show up.
//!
//! Run:
//!   cargo bench -p orts --bench panel_shadow

use criterion::{Criterion, criterion_group, criterion_main};
use nalgebra::Vector3;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::AttitudeState;
use orts::model::Model;
use orts::spacecraft::{PanelDrag, PanelOptics, PanelSrp, SpacecraftShape, SurfacePanel};

/// A 400 km circular orbit with the body axes on the inertial ones, so the Sun
/// direction in the body frame is whatever the ephemeris gives.
fn state() -> SpacecraftState {
    let r = arika::earth::R + 400.0;
    let v = (arika::earth::MU / r).sqrt();
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0)),
        attitude: AttitudeState::identity(),
        mass: 500.0,
    }
}

/// The same orbit with the velocity turned 45 deg in the x-y plane, so the
/// atmosphere reaches the arrays.
///
/// In `state` the spacecraft travels along `+y`, so the gas arrives from `+y`
/// and meets the arrays' `+x` normals edge-on: the force cutoff drops them
/// before any geometry runs, and the drag figures there say nothing about the
/// shadow work. Here the gas arrives from `+x -y`, which the arrays face. The
/// direction is what matters, and it puts the arrays upwind and downwind of the
/// bus rather than beside it, so the subtraction runs.
fn oblique_state() -> SpacecraftState {
    let r = arika::earth::R + 400.0;
    let v = (arika::earth::MU / r).sqrt();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    SpacecraftState {
        orbit: OrbitalState::new(
            Vector3::new(r * s, r * s, 0.0),
            Vector3::new(v * s, -v * s, 0.0),
        ),
        attitude: AttitudeState::identity(),
        mass: 500.0,
    }
}

fn optics() -> PanelOptics {
    PanelOptics::new(0.2, 0.1)
}

/// Six panels with an area and no boundary: no geometry, only the force law.
fn outline_free() -> Vec<SurfacePanel> {
    let axes = [
        Vector3::x(),
        -Vector3::x(),
        Vector3::y(),
        -Vector3::y(),
        Vector3::z(),
        -Vector3::z(),
    ];
    axes.iter()
        .map(|n| SurfacePanel::at_com(1.0, *n, 2.2, optics()).with_cp_offset(n * 0.5))
        .collect()
}

fn cube() -> Vec<SurfacePanel> {
    let SpacecraftShape::Panels(faces) = SpacecraftShape::cube(0.5, 2.2, optics()) else {
        unreachable!("cube is panelled")
    };
    faces
}

/// One solar array: a rectangle in the y-z plane, `segments` panels long,
/// reaching from `y = 0.6` to `y = 2.6` on the given side.
fn array(side: f64, segments: usize) -> Vec<SurfacePanel> {
    let span = 2.0 / segments as f64;
    (0..segments)
        .map(|i| {
            let centre = side * (0.6 + span * (i as f64 + 0.5));
            SurfacePanel::rectangle([span / 2.0, 0.5], Vector3::y(), Vector3::x(), 2.2, optics())
                .with_cp_offset(Vector3::new(0.0, centre, 0.0))
        })
        .collect()
}

fn bus_and_arrays(segments: usize) -> Vec<SurfacePanel> {
    let mut panels = cube();
    panels.extend(array(1.0, segments));
    panels.extend(array(-1.0, segments));
    panels
}

fn bench(c: &mut Criterion) {
    let state = state();
    let epoch = arika::epoch::Epoch::from_jd(2460000.5);

    let mut group = c.benchmark_group("panel_srp_eval");
    for (name, panels) in [
        ("outline_free", outline_free()),
        ("cube", cube()),
        ("bus_and_arrays", bus_and_arrays(1)),
        ("segmented_array", bus_and_arrays(8)),
    ] {
        let n = panels.len();
        let srp = PanelSrp::for_earth(SpacecraftShape::Panels(panels));
        group.bench_function(format!("{name}_{n}_panels"), |b| {
            b.iter(|| srp.eval(0.0, &state, Some(&epoch)))
        });
    }
    group.finish();

    // One derivative evaluation of the whole spacecraft, with the two panel
    // models installed alongside the gravity the integrator has to do anyway.
    // A per-model figure only says what it costs against this, so every shape
    // measured above is measured here too.
    let mut group = c.benchmark_group("spacecraft_derivatives");
    for (name, panels) in [
        ("outline_free", outline_free()),
        ("cube", cube()),
        ("bus_and_arrays", bus_and_arrays(1)),
        ("segmented_array", bus_and_arrays(8)),
    ] {
        let n = panels.len();
        let inertia = nalgebra::Matrix3::from_diagonal(&Vector3::new(10.0, 12.0, 8.0));
        let dynamics = orts::spacecraft::SpacecraftDynamics::new(
            arika::earth::MU,
            orts::orbital::gravity::PointMass,
            inertia,
        )
        .with_epoch(epoch)
        .with_model(PanelSrp::for_earth(SpacecraftShape::Panels(panels.clone())))
        .with_model(PanelDrag::for_earth(SpacecraftShape::Panels(panels)));
        let augmented = orts::effector::AugmentedState::from(state.clone());
        group.bench_function(format!("{name}_{n}_panels"), |b| {
            b.iter(|| utsuroi::DynamicalSystem::derivatives(&dynamics, 0.0, &augmented))
        });
    }
    group.finish();

    // What a sample costs beside the derivative above. `orts run` writes one
    // per output step and the torque breakdown evaluates every model again at
    // that state, so this is the price of the telemetry, measured against the
    // work the integrator does anyway.
    let mut group = c.benchmark_group("torque_breakdown");
    for (name, panels) in [
        ("outline_free", outline_free()),
        ("cube", cube()),
        ("bus_and_arrays", bus_and_arrays(1)),
        ("segmented_array", bus_and_arrays(8)),
    ] {
        let n = panels.len();
        let inertia = nalgebra::Matrix3::from_diagonal(&Vector3::new(10.0, 12.0, 8.0));
        let dynamics = orts::spacecraft::SpacecraftDynamics::new(
            arika::earth::MU,
            orts::orbital::gravity::PointMass,
            inertia,
        )
        .with_epoch(epoch)
        .with_model(PanelSrp::for_earth(SpacecraftShape::Panels(panels.clone())))
        .with_model(PanelDrag::for_earth(SpacecraftShape::Panels(panels)));
        group.bench_function(format!("{name}_{n}_panels"), |b| {
            b.iter(|| dynamics.torque_breakdown(0.0, &state))
        });
    }
    group.finish();

    let oblique = oblique_state();
    let mut group = c.benchmark_group("panel_drag_eval");
    for (name, panels, st) in [
        ("outline_free", outline_free(), &state),
        ("cube", cube(), &state),
        ("bus_and_arrays_edge_on_arrays", bus_and_arrays(1), &state),
        ("segmented_array_edge_on_arrays", bus_and_arrays(8), &state),
        ("bus_and_arrays_oblique", bus_and_arrays(1), &oblique),
        ("segmented_array_oblique", bus_and_arrays(8), &oblique),
    ] {
        let n = panels.len();
        let drag = PanelDrag::for_earth(SpacecraftShape::Panels(panels));
        group.bench_function(format!("{name}_{n}_panels"), |b| {
            b.iter(|| drag.eval(0.0, st, None))
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
