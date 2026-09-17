//! The state a boundary walk is handed has to be one the constraints allow.
//!
//! `initial_augmented_state` refuses a mass below the propellant floor, but
//! `AugmentedState`'s fields are public and `add_satellite` takes the state it
//! is given, so that refusal used to be reachable only through the
//! constructor. Measured on the state below (dry mass 100 kg, a hand-built
//! 99.5 kg): with the pool's mode at `Free` the walk's own reconciliation
//! settled the boundary and the mass *rose* to 100 kg — half a kilogram of
//! propellant the input never had — and with the mode at `Lower` the run
//! carried a mass below the floor from beginning to end. Both were silent.
//!
//! Now each constraint answers for the state it is handed, and a walk that
//! cannot start says so ([#523]).
//!
//! [#523]: https://github.com/sksat/orts/issues/523

use nalgebra::{Matrix3, Vector3};

use orts::attitude::AttitudeState;
use orts::boundary::{HasBoundaries, StartStateError};
use orts::effector::{AugmentedState, ConstraintMode};
use orts::group::IntegratorConfig;
use orts::group::independent::IndependentGroup;
use orts::orbital::OrbitalState;
use orts::orbital::gravity::PointMass;
use orts::spacecraft::reaction_wheel::Rw;
use orts::spacecraft::{
    G0, PropellantPool, ReactionWheelAssembly, SpacecraftDynamics, SpacecraftState, Thruster,
};

const ISP_S: f64 = 200.0;
const THRUST_N: f64 = 0.1 * ISP_S * G0; // 0.1 kg/s
const DRY_MASS: f64 = 100.0;
const MISSING_KG: f64 = 0.5;
const DT: f64 = 1.0;

fn with_a_pool() -> SpacecraftDynamics<PointMass> {
    // Free space: the mass is the only thing these cases are about.
    SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity())
        .with_propellant(PropellantPool::new(DRY_MASS))
        .with_propulsion(Thruster::new(THRUST_N, ISP_S, Vector3::x()))
}

fn plant_at(mass: f64) -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        attitude: AttitudeState::identity(),
        mass,
    }
}

/// Propagate one satellite for a step and report what the group said about it,
/// together with the state it ended on.
fn walked(
    state: AugmentedState<SpacecraftState>,
    dynamics: impl Fn() -> SpacecraftDynamics<PointMass>,
) -> (Option<String>, AugmentedState<SpacecraftState>) {
    let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> = IndependentGroup::new(
        IntegratorConfig::Rk4 { dt: DT },
    )
    .add_satellite("sat", state, dynamics());
    let outcome = group.propagate_to(DT).expect("the group answers");
    let ended = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();
    let reason = outcome.terminations.first().map(|t| t.reason.clone());
    (reason, ended)
}

/// The asymmetric case: settling a mass below the floor would *add* the
/// propellant the input was missing, so the walk refuses the state instead.
#[test]
fn a_mass_below_the_dry_mass_is_refused_rather_than_settled() {
    let start = AugmentedState {
        plant: plant_at(DRY_MASS - MISSING_KG),
        aux: vec![],
        aux_bounds: vec![],
        modes: vec![ConstraintMode::Free],
    };
    let (reason, ended) = walked(start, with_a_pool);

    let reason = reason.expect("the satellite is terminated rather than propagated");
    assert!(
        reason.contains("propellant_pool") && reason.contains("below the dry mass"),
        "the reason names what refused the state and what it read, not {reason}"
    );
    assert_eq!(
        ended.plant.mass,
        DRY_MASS - MISSING_KG,
        "and the mass is left as it was handed over: settling it would create {MISSING_KG} kg"
    );
}

/// A mode saying the tank is empty while the mass carries propellant would
/// silence the propulsion for a run that could burn.
#[test]
fn an_empty_mode_with_propellant_above_the_floor_is_refused() {
    let start = AugmentedState {
        plant: plant_at(DRY_MASS + 1.0),
        aux: vec![],
        aux_bounds: vec![],
        modes: vec![ConstraintMode::Lower],
    };
    let (reason, _) = walked(start, with_a_pool);

    let reason = reason.expect("the satellite is terminated rather than propagated");
    assert!(
        reason.contains("empty") && reason.contains("above the dry mass"),
        "the reason says the mode and the mass disagree, not {reason}"
    );
}

/// The floor itself is a legal start: a spacecraft can begin with an empty
/// tank, and the mode that says so is the one the constructor writes.
#[test]
fn a_mass_exactly_on_the_dry_mass_starts() {
    let start = AugmentedState {
        plant: plant_at(DRY_MASS),
        aux: vec![],
        aux_bounds: vec![],
        modes: vec![ConstraintMode::Lower],
    };
    let (reason, ended) = walked(start, with_a_pool);

    assert!(reason.is_none(), "an empty tank propagates: {reason:?}");
    assert_eq!(
        ended.plant.mass, DRY_MASS,
        "and nothing burns, so the mass stays on the floor"
    );
}

/// Every state the constructor produces is one the walk accepts, including the
/// two edges it decides between.
#[test]
fn the_constructor_agrees_with_the_walk() {
    for mass in [
        DRY_MASS,
        DRY_MASS + f64::EPSILON * DRY_MASS,
        DRY_MASS + 10.0,
    ] {
        let system = with_a_pool();
        let start = system.initial_augmented_state(plant_at(mass));
        let (reason, _) = walked(start, with_a_pool);
        assert!(
            reason.is_none(),
            "the state the constructor built for {mass} kg is refused: {reason:?}"
        );
    }
}

fn with_a_wheel(limit: f64) -> SpacecraftDynamics<PointMass> {
    let wheel = Rw::new(Vector3::x(), 0.01, limit, 0.1);
    SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity())
        .with_effector(ReactionWheelAssembly::new(vec![wheel]))
}

/// The wheel is the symmetric case — settling returns the overshoot to the
/// body, so nothing is created — but it would turn the spacecraft at a rate
/// the caller never asked for.
#[test]
fn a_wheel_handed_over_past_its_limit_is_refused() {
    const LIMIT: f64 = 1.0;
    let start = AugmentedState {
        plant: plant_at(DRY_MASS),
        aux: vec![1.5 * LIMIT],
        aux_bounds: vec![],
        modes: vec![ConstraintMode::Free],
    };
    let (reason, ended) = walked(start, || with_a_wheel(LIMIT));

    let reason = reason.expect("the satellite is terminated rather than propagated");
    assert!(
        reason.contains("past its limit"),
        "the reason names the wheel and its limit, not {reason}"
    );
    assert_eq!(
        ended.plant.attitude.angular_velocity,
        Vector3::zeros(),
        "and the body is left at rest: settling the wheel would spin it up"
    );
}

/// A mode claiming a wheel is held while its momentum is inside the limit
/// would freeze it at a value that is not its bound.
#[test]
fn a_wheel_held_by_its_mode_away_from_its_bound_is_refused() {
    const LIMIT: f64 = 1.0;
    let start = AugmentedState {
        plant: plant_at(DRY_MASS),
        aux: vec![0.25 * LIMIT],
        aux_bounds: vec![],
        modes: vec![ConstraintMode::Upper],
    };
    let (reason, _) = walked(start, || with_a_wheel(LIMIT));

    let reason = reason.expect("the satellite is terminated rather than propagated");
    assert!(
        reason.contains("is held at"),
        "the reason says the mode and the momentum disagree, not {reason}"
    );
}

/// A system with a floor of its own, for the path a spacecraft cannot take.
///
/// `CoupledGroup` needs a state an interaction force can be turned into
/// (`FromAcceleration`), which `AugmentedState` does not implement, so a
/// spacecraft with effectors cannot be in one today. What this checks is the
/// composite's delegation, and any constrained system reaches it.
struct Floor;

const FLOOR_X: f64 = 7000.0;

impl utsuroi::DynamicalSystem for Floor {
    type State = OrbitalState;
    fn derivatives(&self, _t: f64, state: &OrbitalState) -> OrbitalState {
        OrbitalState::from_derivative(*state.velocity(), Vector3::zeros())
    }
}

impl HasBoundaries for Floor {
    fn validate_boundary_walk_start(
        &self,
        _t: f64,
        state: &OrbitalState,
    ) -> Result<(), StartStateError> {
        let x = state.position().x;
        if x < FLOOR_X {
            return Err(StartStateError::new(format!(
                "{x} is below the floor at {FLOOR_X}"
            )));
        }
        Ok(())
    }
}

/// A coupled group walks its satellites as one composite state, so the
/// composite has to ask each of them: one answering for itself would let the
/// group start from exactly the states its children refuse, and the child that
/// refused would have no name in the reason.
#[test]
fn a_coupled_group_asks_each_satellite_about_its_own_state() {
    use orts::group::coupled::CoupledGroup;

    let above = OrbitalState::new(Vector3::new(FLOOR_X + 1.0, 0.0, 0.0), Vector3::zeros());
    let below = OrbitalState::new(Vector3::new(FLOOR_X - 1.0, 0.0, 0.0), Vector3::zeros());
    let mut group = CoupledGroup::new(IntegratorConfig::Rk4 { dt: DT })
        .add_satellite("above", above, Floor)
        .add_satellite("below", below, Floor);

    let outcome = group.propagate_to(DT).expect("the group answers");
    let termination = outcome
        .terminations
        .first()
        .expect("the group is terminated rather than propagated");
    let reason = termination.reason.clone();
    assert!(
        reason.contains("satellite 1") && reason.contains("below the floor"),
        "the reason says which satellite refused the state, not {reason}"
    );
    assert_eq!(
        termination.satellite_id,
        orts::group::SatId::from("below"),
        "and the record names the satellite that refused, which is the one to fix"
    );
    assert_eq!(
        group.group_state().states[1].position().x,
        FLOOR_X - 1.0,
        "and its state is left as it was handed over"
    );
}

/// A pair with no force between it: what makes two satellites one coupled
/// component here is the regime, and a force that pulls nothing leaves the
/// trajectories the straight lines these assertions read.
struct NoForce;

impl orts::group::coupled::InterSatelliteForce for NoForce {
    fn name(&self) -> &str {
        "none"
    }

    fn acceleration_pair(
        &self,
        _ctx: &orts::group::coupled::PairContext<'_>,
    ) -> (Vector3<f64>, Vector3<f64>) {
        (Vector3::zeros(), Vector3::zeros())
    }
}

/// Under a scheduler, a refusal takes the satellite that refused and leaves the
/// rest of its component running.
///
/// A coupled component is integrated as one composite state, so an integration
/// error puts every satellite in it on no trajectory and the scheduler drops
/// them all. A refused start state is the other case: nothing was stepped, so
/// the others still hold exactly the state they were handed. What that looks
/// like from outside is that they keep moving.
#[test]
fn a_scheduler_drops_only_the_satellite_that_refused() {
    use orts::group::RegimeConfig;
    use orts::group::scheduler::Scheduler;

    const SPEED: f64 = 1.0;

    // Close enough to be one coupled component, so the refusal happens inside
    // a composite walk rather than in a group of one. Both drift outward, away
    // from the floor.
    let moving =
        |x: f64| OrbitalState::new(Vector3::new(x, 0.0, 0.0), Vector3::new(SPEED, 0.0, 0.0));
    let mut sched: Scheduler<Floor> = Scheduler::new(
        RegimeConfig {
            couple_enter: 10.0,
            couple_exit: 20.0,
            sync_enter: 20.0,
            sync_exit: 30.0,
            sync_interval: 10.0,
            min_dwell_time: 0.0,
        },
        IntegratorConfig::Rk4 { dt: DT },
    )
    .add_satellite("above", moving(FLOOR_X + 1.0), Floor)
    .add_satellite("below", moving(FLOOR_X - 1.0), Floor)
    .add_interaction_fixed(
        "above",
        "below",
        orts::group::PairRegime::Coupled,
        std::sync::Arc::new(NoForce),
    );

    // The premise: one coupled component, walked as a single composite state.
    assert_eq!(
        sched.pair_regime(
            &orts::group::SatId::from("above"),
            &orts::group::SatId::from("below")
        ),
        Some(orts::group::PairRegime::Coupled),
        "the two are close enough to be integrated together"
    );

    let outcome = sched.propagate_to(DT).expect("the scheduler answers");
    let refused = outcome
        .terminations
        .first()
        .expect("the satellite below the floor is terminated");
    assert_eq!(
        refused.satellite_id,
        orts::group::SatId::from("below"),
        "the record names the satellite that refused"
    );

    sched
        .propagate_to(3.0 * DT)
        .expect("the scheduler answers again");
    let x_of = |id: &str| {
        sched
            .satellite_state(&orts::group::SatId::from(id))
            .expect("the satellite is still in the fleet")
            .position()
            .x
    };
    // The whole span, not the part after the refusal: the satellite that could
    // not start is dropped before anything is grouped, so the survivor is flown
    // over the same interval rather than being left behind at a time the
    // scheduler's clock has already passed.
    assert!(
        (x_of("above") - (FLOOR_X + 1.0 + 3.0 * DT * SPEED)).abs() < 1e-9,
        "the satellite whose state was fine flies the whole span, not {}",
        x_of("above")
    );
    assert!(
        (x_of("below") - (FLOOR_X - 1.0)).abs() < 1e-9,
        "and the one that refused stays where it was handed over, not {}",
        x_of("below")
    );
}

/// A system whose start state is about how fast it is going, so that a
/// velocity kick can be what makes the state unusable.
struct SpeedLimit;

const SPEED_LIMIT: f64 = 5.0;

impl utsuroi::DynamicalSystem for SpeedLimit {
    type State = OrbitalState;
    fn derivatives(&self, _t: f64, state: &OrbitalState) -> OrbitalState {
        OrbitalState::from_derivative(*state.velocity(), Vector3::zeros())
    }
}

impl HasBoundaries for SpeedLimit {
    fn validate_boundary_walk_start(
        &self,
        _t: f64,
        state: &OrbitalState,
    ) -> Result<(), StartStateError> {
        let speed = state.velocity().magnitude();
        if speed > SPEED_LIMIT {
            return Err(StartStateError::new(format!(
                "{speed} is over the speed limit of {SPEED_LIMIT}"
            )));
        }
        Ok(())
    }
}

/// A one-sided shove, for the pair that is kicked rather than integrated.
struct Shove(f64);

impl orts::group::coupled::InterSatelliteForce for Shove {
    fn name(&self) -> &str {
        "shove"
    }

    fn acceleration_pair(
        &self,
        _ctx: &orts::group::coupled::PairContext<'_>,
    ) -> (Vector3<f64>, Vector3<f64>) {
        (Vector3::new(self.0, 0.0, 0.0), Vector3::zeros())
    }
}

/// The scheduler's KDK path kicks velocities before it drifts, so what can
/// start has to be asked again after the kick.
///
/// The case needs three satellites: the kicked one shares a coupled component
/// with a second, and takes its kick from a third. Asking only before the kick
/// leaves the refusal to the composite walk, which integrates nothing — and
/// the component's other member would then sit at this interval's start while
/// the scheduler's clock moves to its end.
#[test]
fn a_satellite_a_kick_pushes_past_its_limit_is_dropped_before_the_drift() {
    use orts::group::RegimeConfig;
    use orts::group::scheduler::Scheduler;

    const SPEED: f64 = 1.0;
    // Half of the interval at this acceleration is 10, twice the limit.
    const SHOVE: f64 = 20.0 / DT;

    let at =
        |x: f64, vx: f64| OrbitalState::new(Vector3::new(x, 0.0, 0.0), Vector3::new(vx, 0.0, 0.0));
    let mut sched: Scheduler<SpeedLimit> = Scheduler::new(
        RegimeConfig {
            couple_enter: 10.0,
            couple_exit: 20.0,
            sync_enter: 20.0,
            sync_exit: 30.0,
            sync_interval: DT,
            min_dwell_time: 0.0,
        },
        IntegratorConfig::Rk4 { dt: DT },
    )
    .add_satellite("kicked", at(FLOOR_X, 0.0), SpeedLimit)
    .add_satellite("peer", at(FLOOR_X + 1.0, SPEED), SpeedLimit)
    .add_satellite("kicker", at(FLOOR_X + 1e6, 0.0), SpeedLimit)
    .add_interaction_fixed(
        "kicked",
        "peer",
        orts::group::PairRegime::Coupled,
        std::sync::Arc::new(NoForce),
    )
    .add_interaction_fixed(
        "kicked",
        "kicker",
        orts::group::PairRegime::Synchronized,
        std::sync::Arc::new(Shove(SHOVE)),
    );

    let outcome = sched.propagate_to(DT).expect("the scheduler answers");
    let refused = outcome
        .terminations
        .iter()
        .find(|t| t.satellite_id == orts::group::SatId::from("kicked"))
        .expect("the kicked satellite is over its limit once the half-kick lands");
    assert!(
        refused.reason.contains("over the speed limit"),
        "the reason is the speed the kick gave it, not {}",
        refused.reason
    );

    let x_of = |id: &str| {
        sched
            .satellite_state(&orts::group::SatId::from(id))
            .expect("the satellite is still in the fleet")
            .position()
            .x
    };
    assert!(
        (x_of("peer") - (FLOOR_X + 1.0 + DT * SPEED)).abs() < 1e-9,
        "its coupled peer flies the interval it was asked for, not {}",
        x_of("peer")
    );
}

/// The lengths come first, because every other path indexes the same offsets.
#[test]
fn a_state_whose_vectors_do_not_match_the_registry_is_refused() {
    let start = AugmentedState {
        plant: plant_at(DRY_MASS + 1.0),
        aux: vec![],
        aux_bounds: vec![],
        modes: vec![],
    };
    let (reason, _) = walked(start, with_a_pool);

    let reason = reason.expect("the satellite is terminated rather than propagated");
    assert!(
        reason.contains("0 modes") && reason.contains("declared 1"),
        "the reason compares what the state carries with what was registered, not {reason}"
    );
}
