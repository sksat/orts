//! A system reports the times at which its loads switch.
//!
//! `ScheduledBurn` answers a throttle for the instant it is asked about, so a
//! window that falls between an integrator's stage times contributes nothing:
//! measured with a 1 s RK4 step, `[0.1, 0.2)` produced exactly zero ΔV. A
//! propagation loop can end its step at a window edge instead, and these tests
//! cover what it needs in order to: the times themselves, reported through
//! `Model` and aggregated by `SpacecraftDynamics`.
//!
//! Issue #446 tracks the propagation loops consuming these.

use nalgebra::{Matrix3, Vector3};
use orts::model::Model;
use orts::orbital::gravity::PointMass;
use orts::spacecraft::{
    BurnWindow, ScheduledBurn, SpacecraftDynamics, SpacecraftState, ThrustProfile, Thruster,
};
use utsuroi::DynamicalSystem;

fn thruster_with(windows: Vec<BurnWindow>) -> Thruster {
    Thruster::new(10.0, 300.0, Vector3::x()).with_profile(Box::new(ScheduledBurn { windows }))
}

fn dynamics_with(thrusters: Vec<Thruster>) -> SpacecraftDynamics<PointMass> {
    let mut d = SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity());
    for t in thrusters {
        d = d.with_model(t);
    }
    d
}

/// Both ends of a window are edges: the throttle steps up at `start` and back
/// down at `end`.
#[test]
fn a_window_reports_both_of_its_edges() {
    let burn = ScheduledBurn {
        windows: vec![BurnWindow::full(0.1, 0.2)],
    };
    assert_eq!(burn.next_throttle_jump_after(0.0), Some(0.1));
    assert_eq!(burn.next_throttle_jump_after(0.1), Some(0.2));
    assert_eq!(burn.next_throttle_jump_after(0.15), Some(0.2));
    // Past the last edge there is nothing left to report.
    assert_eq!(burn.next_throttle_jump_after(0.2), None);
}

/// The edge asked about is never the answer, so a loop that steps to one and
/// asks again from there makes progress instead of stopping on it.
#[test]
fn the_time_asked_about_is_not_reported() {
    let burn = ScheduledBurn {
        windows: vec![BurnWindow::full(1.0, 2.0)],
    };
    assert_eq!(burn.next_throttle_jump_after(1.0), Some(2.0));
    assert_eq!(burn.next_throttle_jump_after(2.0), None);
}

/// Windows out of order still report their edges in time order.
///
/// `ScheduledBurn.windows` is public, so a caller can append to it after
/// construction; the answer cannot come from an index fixed at build time.
#[test]
fn unordered_windows_report_their_earliest_edge() {
    let burn = ScheduledBurn {
        windows: vec![
            BurnWindow::full(5.0, 6.0),
            BurnWindow::full(1.0, 2.0),
            BurnWindow::full(3.0, 4.0),
        ],
    };
    let mut t = 0.0;
    let mut edges = Vec::new();
    while let Some(next) = burn.next_throttle_jump_after(t) {
        edges.push(next);
        t = next;
    }
    assert_eq!(edges, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

/// A window abutting the next one shares an edge, and the two report as one
/// time rather than as a pair a loop would step to twice.
#[test]
fn abutting_windows_share_one_edge() {
    let burn = ScheduledBurn {
        windows: vec![
            BurnWindow::full(0.0, 1.0),
            BurnWindow {
                start: 1.0,
                end: 2.0,
                throttle: 0.5,
            },
        ],
    };
    let mut t = -1.0;
    let mut edges = Vec::new();
    while let Some(next) = burn.next_throttle_jump_after(t) {
        edges.push(next);
        t = next;
    }
    assert_eq!(edges, vec![0.0, 1.0, 2.0]);
}

/// A throttle that reads the state reports nothing.
///
/// When the throttle jumps is then a property of the trajectory, which no
/// schedule knows in advance. Issue #446 keeps those as state events.
#[test]
fn a_state_dependent_profile_reports_no_time() {
    struct BurnWhileHeavy;
    impl ThrustProfile for BurnWhileHeavy {
        fn throttle(
            &self,
            _t: f64,
            state: &SpacecraftState,
            _e: Option<&arika::epoch::Epoch>,
        ) -> f64 {
            if state.mass > 50.0 { 1.0 } else { 0.0 }
        }
    }

    let thruster = Thruster::new(10.0, 300.0, Vector3::x()).with_profile(Box::new(BurnWhileHeavy));
    assert_eq!(
        Model::<SpacecraftState>::next_discontinuity_after(&thruster, 0.0, None),
        None
    );
    assert_eq!(
        dynamics_with(vec![thruster]).next_discontinuity_after(0.0),
        None
    );
}

/// A system whose loads are continuous reports nothing, so a propagation loop
/// over one steps exactly as it does today.
#[test]
fn a_system_without_a_schedule_reports_no_time() {
    let plain: SpacecraftDynamics<PointMass> =
        SpacecraftDynamics::new(398600.4418, PointMass, Matrix3::identity());
    assert_eq!(plain.next_discontinuity_after(0.0), None);
}

/// The system reports the earliest edge among its models.
#[test]
fn the_system_reports_the_earliest_edge_across_its_thrusters() {
    let d = dynamics_with(vec![
        thruster_with(vec![BurnWindow::full(3.0, 4.0)]),
        thruster_with(vec![BurnWindow::full(1.0, 8.0)]),
    ]);
    // 1.0 opens the second thruster, 3.0 and 4.0 are the first one's window,
    // 8.0 closes the second.
    let mut t = 0.0;
    let mut edges = Vec::new();
    while let Some(next) = d.next_discontinuity_after(t) {
        edges.push(next);
        t = next;
    }
    assert_eq!(edges, vec![1.0, 3.0, 4.0, 8.0]);
}

/// Two thrusters whose windows share an edge report it once.
#[test]
fn thrusters_sharing_an_edge_report_it_once() {
    let d = dynamics_with(vec![
        thruster_with(vec![BurnWindow::full(2.0, 5.0)]),
        thruster_with(vec![BurnWindow::full(2.0, 9.0)]),
    ]);
    assert_eq!(d.next_discontinuity_after(0.0), Some(2.0));
    assert_eq!(d.next_discontinuity_after(2.0), Some(5.0));
}

/// A group passes on the earliest boundary among its satellites.
///
/// The group propagation paths integrate the composite system, not the
/// satellite's own, so a composite that answered `None` would hide every window
/// its satellites carry.
#[test]
fn a_group_forwards_the_boundaries_of_its_satellites() {
    use orts::group::dynamics::IndependentGroupDynamics;

    let group = IndependentGroupDynamics::new(vec![
        dynamics_with(vec![thruster_with(vec![BurnWindow::full(4.0, 6.0)])]),
        dynamics_with(vec![thruster_with(vec![BurnWindow::full(2.0, 9.0)])]),
    ]);

    let mut t = 0.0;
    let mut edges = Vec::new();
    while let Some(next) = group.next_discontinuity_after(t) {
        edges.push(next);
        t = next;
    }
    assert_eq!(edges, vec![2.0, 4.0, 6.0, 9.0]);
}

/// A group of satellites that carry no schedule reports nothing.
#[test]
fn a_group_without_schedules_reports_no_time() {
    use orts::group::dynamics::IndependentGroupDynamics;

    let group = IndependentGroupDynamics::new(vec![dynamics_with(vec![]), dynamics_with(vec![])]);
    assert_eq!(group.next_discontinuity_after(0.0), None);
}

/// An epoch-scheduled burn reports its edges in integration time.
///
/// `ConstantThrust` bounds its burn with two epochs while a propagation loop
/// works in integration time. The system owns `epoch_0`, so it passes the epoch
/// at `t` and the model converts. Its doc used to tell the caller to split the
/// integration at the boundary by hand.
#[test]
fn an_epoch_scheduled_burn_reports_its_edges_in_integration_time() {
    use arika::epoch::Epoch;
    use orts::orbital::system::OrbitalSystem;
    use orts::perturbations::ConstantThrust;

    let epoch_0 = Epoch::j2000();
    let burn = ConstantThrust::new(
        "burn",
        epoch_0.add_si_seconds(100.0),
        epoch_0.add_si_seconds(160.0),
        arika::frame::Vec3::from_raw(Vector3::new(1e-6, 0.0, 0.0)),
    );
    let system: OrbitalSystem = OrbitalSystem::new(398600.4418, Box::new(PointMass))
        .with_epoch(epoch_0)
        .with_model(burn);

    // 100 s and 160 s after t = 0, which is where epoch_0 sits.
    assert_eq!(system.next_discontinuity_after(0.0), Some(100.0));
    assert_eq!(system.next_discontinuity_after(100.0), Some(160.0));
    assert_eq!(system.next_discontinuity_after(160.0), None);
    // Asked from inside the burn, the answer is its end.
    assert_eq!(system.next_discontinuity_after(120.0), Some(160.0));
}

/// Without an epoch there is no mapping from the burn's epochs to integration
/// time, and nothing is reported.
#[test]
fn an_epoch_scheduled_burn_without_an_epoch_reports_no_time() {
    use arika::epoch::Epoch;
    use orts::orbital::system::OrbitalSystem;
    use orts::perturbations::ConstantThrust;

    let epoch_0 = Epoch::j2000();
    let burn = ConstantThrust::new(
        "burn",
        epoch_0.add_si_seconds(100.0),
        epoch_0.add_si_seconds(160.0),
        arika::frame::Vec3::from_raw(Vector3::new(1e-6, 0.0, 0.0)),
    );
    let system: OrbitalSystem =
        OrbitalSystem::new(398600.4418, Box::new(PointMass)).with_model(burn);
    assert_eq!(system.next_discontinuity_after(0.0), None);
}

/// A non-finite edge is left out rather than handed to a loop as a target.
#[test]
fn a_non_finite_edge_is_not_reported() {
    let burn = ScheduledBurn {
        windows: vec![BurnWindow::full(1.0, f64::INFINITY)],
    };
    assert_eq!(burn.next_throttle_jump_after(0.0), Some(1.0));
    assert_eq!(burn.next_throttle_jump_after(1.0), None);
}
