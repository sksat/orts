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
    assert_eq!(burn.next_throttle_jump_after(0.0, None), Some(0.1));
    assert_eq!(burn.next_throttle_jump_after(0.1, None), Some(0.2));
    assert_eq!(burn.next_throttle_jump_after(0.15, None), Some(0.2));
    // Past the last edge there is nothing left to report.
    assert_eq!(burn.next_throttle_jump_after(0.2, None), None);
}

/// The edge asked about is never the answer, so a loop that steps to one and
/// asks again from there makes progress instead of stopping on it.
#[test]
fn the_time_asked_about_is_not_reported() {
    let burn = ScheduledBurn {
        windows: vec![BurnWindow::full(1.0, 2.0)],
    };
    assert_eq!(burn.next_throttle_jump_after(1.0, None), Some(2.0));
    assert_eq!(burn.next_throttle_jump_after(2.0, None), None);
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
    while let Some(next) = burn.next_throttle_jump_after(t, None) {
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
    while let Some(next) = burn.next_throttle_jump_after(t, None) {
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

/// A model that reports a fixed set of edges, for the systems whose own models
/// carry no schedule.
///
/// Reports the earliest of `edges` after `t`, so a system holding two of these
/// has a minimum to take and a caller can walk the sequence.
struct EdgeStub {
    name: &'static str,
    edges: Vec<f64>,
}

impl EdgeStub {
    fn new(name: &'static str, edges: Vec<f64>) -> Self {
        Self { name, edges }
    }
}

impl<S: orts::model::HasFrame> Model<S> for EdgeStub {
    fn name(&self) -> &str {
        self.name
    }

    fn eval(
        &self,
        _t: f64,
        _state: &S,
        _epoch: Option<&arika::epoch::Epoch>,
    ) -> orts::model::ExternalLoads<S::Frame> {
        orts::model::ExternalLoads::zeros()
    }

    fn next_discontinuity_after(
        &self,
        t: f64,
        _epoch: Option<&arika::epoch::Epoch>,
    ) -> Option<f64> {
        self.edges
            .iter()
            .copied()
            .filter(|e| *e > t)
            .min_by(f64::total_cmp)
    }
}

/// Walk a system's reported edges from `t0` until it stops reporting.
fn edges_of<D: DynamicalSystem>(system: &D, t0: f64) -> Vec<f64> {
    let mut t = t0;
    let mut edges = Vec::new();
    while let Some(next) = system.next_discontinuity_after(t) {
        edges.push(next);
        t = next;
    }
    edges
}

/// `AttitudeSystem` passes on its models' edges.
#[test]
fn the_attitude_system_forwards_its_models_edges() {
    use orts::attitude::AttitudeSystem;

    let bare = AttitudeSystem::new(Matrix3::identity());
    assert_eq!(bare.next_discontinuity_after(0.0), None);

    let system = AttitudeSystem::new(Matrix3::identity())
        .with_model(EdgeStub::new("a", vec![3.0, 7.0]))
        .with_model(EdgeStub::new("b", vec![1.0, 5.0]));
    assert_eq!(edges_of(&system, 0.0), vec![1.0, 3.0, 5.0, 7.0]);
}

/// `DecoupledAttitudeSystem` passes on its models' edges.
#[test]
fn the_decoupled_attitude_system_forwards_its_models_edges() {
    use orts::attitude::DecoupledAttitudeSystem;

    let bare =
        DecoupledAttitudeSystem::circular_orbit(Matrix3::identity(), 398600.4418, 7000.0, 100.0);
    assert_eq!(bare.next_discontinuity_after(0.0), None);

    let system =
        DecoupledAttitudeSystem::circular_orbit(Matrix3::identity(), 398600.4418, 7000.0, 100.0)
            .with_model(EdgeStub::new("a", vec![2.0, 8.0]))
            .with_model(EdgeStub::new("b", vec![4.0]));
    assert_eq!(edges_of(&system, 0.0), vec![2.0, 4.0, 8.0]);
}

/// `AugmentedAttitudeSystem` passes on its models' edges, independently of any
/// effector state it also carries.
#[test]
fn the_augmented_attitude_system_forwards_its_models_edges() {
    use orts::attitude::AugmentedAttitudeSystem;

    let bare =
        AugmentedAttitudeSystem::circular_orbit(Matrix3::identity(), 398600.4418, 7000.0, 100.0);
    assert_eq!(bare.next_discontinuity_after(0.0), None);

    let system =
        AugmentedAttitudeSystem::circular_orbit(Matrix3::identity(), 398600.4418, 7000.0, 100.0)
            .with_model(EdgeStub::new("a", vec![6.0]))
            .with_model(EdgeStub::new("b", vec![1.5, 9.0]));
    assert_eq!(edges_of(&system, 0.0), vec![1.5, 6.0, 9.0]);
}

/// `CoupledGroupDynamics` passes on the earliest edge across its satellites.
///
/// The inter-satellite forces are continuous, so the group switches exactly
/// where its satellites do.
#[test]
fn a_coupled_group_forwards_the_boundaries_of_its_satellites() {
    use orts::group::coupled::CoupledGroupDynamics;

    use orts::orbital::system::OrbitalSystem;

    // `CoupledGroupDynamics` needs a state it can add an interaction
    // acceleration to, which `OrbitalState` is and `AugmentedState` is not.
    let orbital = || -> OrbitalSystem { OrbitalSystem::new(398600.4418, Box::new(PointMass)) };

    let bare = CoupledGroupDynamics::new(vec![orbital(), orbital()], vec![]);
    assert_eq!(bare.next_discontinuity_after(0.0), None);

    let group = CoupledGroupDynamics::new(
        vec![
            orbital().with_model(EdgeStub::new("a", vec![5.0, 7.0])),
            orbital().with_model(EdgeStub::new("b", vec![3.0, 11.0])),
        ],
        vec![],
    );
    assert_eq!(edges_of(&group, 0.0), vec![3.0, 5.0, 7.0, 11.0]);
}

/// An epoch-scheduled profile reports its edges through `Thruster`.
///
/// `throttle` receives the epoch, so `next_throttle_jump_after` does too —
/// otherwise a profile that schedules itself in epochs could evaluate correctly
/// while never reporting an edge.
#[test]
fn an_epoch_scheduled_profile_reports_through_the_thruster() {
    use arika::epoch::Epoch;

    /// Fires for 60 s starting 100 s after its reference epoch.
    struct EpochBurn {
        reference: Epoch,
    }
    impl ThrustProfile for EpochBurn {
        fn throttle(&self, _t: f64, _s: &SpacecraftState, epoch: Option<&Epoch>) -> f64 {
            match epoch {
                Some(now) => {
                    let since = now.duration_since(&self.reference).as_si_seconds();
                    if (100.0..160.0).contains(&since) {
                        1.0
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            }
        }

        fn next_throttle_jump_after(&self, t: f64, epoch: Option<&Epoch>) -> Option<f64> {
            let now = epoch?;
            let since = now.duration_since(&self.reference).as_si_seconds();
            [100.0 - since, 160.0 - since]
                .iter()
                .map(|offset| t + offset)
                .filter(|edge| *edge > t)
                .min_by(f64::total_cmp)
        }
    }

    let epoch_0 = Epoch::j2000();
    let thruster = Thruster::new(10.0, 300.0, Vector3::x())
        .with_profile(Box::new(EpochBurn { reference: epoch_0 }));
    let system: SpacecraftDynamics<PointMass> =
        SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity())
            .with_epoch(epoch_0)
            .with_model(thruster);

    assert_eq!(edges_of(&system, 0.0), vec![100.0, 160.0]);
    // Without an epoch the profile has no reference to measure from.
    let no_epoch: SpacecraftDynamics<PointMass> =
        SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity()).with_model(
            Thruster::new(10.0, 300.0, Vector3::x())
                .with_profile(Box::new(EpochBurn { reference: epoch_0 })),
        );
    assert_eq!(no_epoch.next_discontinuity_after(0.0), None);
}

/// A scheduled effector reports through the systems that hold it.
///
/// Effectors are part of the right-hand side alongside the models, and their
/// `derivatives` receives `t` and `epoch`, so one driven by a schedule can
/// switch. Both systems that hold effectors take the minimum over models and
/// effectors together.
#[test]
fn a_scheduled_effector_reports_through_its_system() {
    use orts::effector::StateEffector;

    struct ScheduledEffector {
        edges: Vec<f64>,
    }

    impl<S: orts::model::HasFrame> StateEffector<S> for ScheduledEffector {
        fn name(&self) -> &str {
            "scheduled_effector"
        }

        fn state_dim(&self) -> usize {
            0
        }

        fn derivatives(
            &self,
            _t: f64,
            _state: &S,
            _aux: &[f64],
            _aux_rates: &mut [f64],
            _epoch: Option<&arika::epoch::Epoch>,
        ) -> orts::model::ExternalLoads<S::Frame> {
            orts::model::ExternalLoads::zeros()
        }

        fn next_discontinuity_after(
            &self,
            t: f64,
            _epoch: Option<&arika::epoch::Epoch>,
        ) -> Option<f64> {
            self.edges
                .iter()
                .copied()
                .filter(|e| *e > t)
                .min_by(f64::total_cmp)
        }
    }

    // A model and an effector on the same system: the minimum comes from
    // whichever is earlier at each step.
    let system: SpacecraftDynamics<PointMass> =
        SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity())
            .with_model(EdgeStub::new("model", vec![4.0]))
            .with_effector(ScheduledEffector {
                edges: vec![2.0, 6.0],
            });
    assert_eq!(edges_of(&system, 0.0), vec![2.0, 4.0, 6.0]);

    use orts::attitude::AugmentedAttitudeSystem;
    let attitude =
        AugmentedAttitudeSystem::circular_orbit(Matrix3::identity(), 398600.4418, 7000.0, 100.0)
            .with_effector(ScheduledEffector {
                edges: vec![1.0, 8.0],
            });
    assert_eq!(edges_of(&attitude, 0.0), vec![1.0, 8.0]);
}

/// A scheduled inter-satellite force reports through the coupled group.
///
/// `acceleration_pair` receives `PairContext::t`, so a force can carry a
/// schedule; the group's own doc used to claim these were continuous.
#[test]
fn a_scheduled_interaction_force_reports_through_the_group() {
    use orts::group::coupled::{
        CoupledGroupDynamics, InterSatelliteForce, InteractionPair, PairContext,
    };
    use orts::orbital::system::OrbitalSystem;
    use std::sync::Arc;

    struct ScheduledForce {
        edges: Vec<f64>,
    }
    impl InterSatelliteForce for ScheduledForce {
        fn name(&self) -> &str {
            "scheduled_force"
        }
        fn acceleration_pair(&self, _ctx: &PairContext<'_>) -> (Vector3<f64>, Vector3<f64>) {
            (Vector3::zeros(), Vector3::zeros())
        }
        fn next_discontinuity_after(&self, t: f64) -> Option<f64> {
            self.edges
                .iter()
                .copied()
                .filter(|e| *e > t)
                .min_by(f64::total_cmp)
        }
    }

    let orbital = || -> OrbitalSystem { OrbitalSystem::new(398600.4418, Box::new(PointMass)) };
    let group = CoupledGroupDynamics::new(
        vec![
            orbital().with_model(EdgeStub::new("child", vec![5.0])),
            orbital(),
        ],
        vec![InteractionPair {
            i: 0,
            j: 1,
            force: Arc::new(ScheduledForce {
                edges: vec![2.0, 9.0],
            }),
        }],
    );
    // 2.0 and 9.0 come from the force, 5.0 from the first satellite's model.
    assert_eq!(edges_of(&group, 0.0), vec![2.0, 5.0, 9.0]);
}

/// A non-finite edge is left out rather than handed to a loop as a target.
#[test]
fn a_non_finite_edge_is_not_reported() {
    let burn = ScheduledBurn {
        windows: vec![BurnWindow::full(1.0, f64::INFINITY)],
    };
    assert_eq!(burn.next_throttle_jump_after(0.0, None), Some(1.0));
    assert_eq!(burn.next_throttle_jump_after(1.0, None), None);
}
