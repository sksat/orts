//! 離散制御 + ZOH 積分ループ。
//!
//! Config から宇宙機ダイナミクス + プラグインコントローラ + センサ + RW を
//! 組み立て、制御サンプル周期ごとに積分 -> センサ評価 -> プラグイン呼び出し ->
//! アクチュエータ更新 を繰り返す。`orts run` と `orts serve` の両方から使う。

use std::sync::Arc;

use arika::epoch::Epoch;
use orts::effector::AugmentedState;
use orts::orbital::gravity::GravityField;
use orts::plugin::{
    ActuatorBundle, ActuatorTelemetry, MtqCommand, PluginController, RwTelemetry, TickInput,
};
use orts::sensor::{Gyroscope, Magnetometer, SensorBundle, StarTracker};
use orts::setup::default_third_bodies;

use crate::sim::core::spacecraft_dynamics_for;
use core::ops::ControlFlow;
use nalgebra::Vector3;
use orts::boundary::{Boundaries, BoundaryWalk, HasBoundaries, Span, walk_to_target};
use orts::group::IntegratorConfig;
use orts::spacecraft::{
    MtqAssembly, ReactionWheelAssembly, SpacecraftDynamics, SpacecraftState, ThrusterAssembly,
    ThrusterAssemblyCore, ThrusterSpec,
};
use tobari::magnetic::igrf::Igrf;
use utsuroi::{
    Dop853, DormandPrince, IntegrationError, Integrator, Rk4, RootSearch, RootSlot, SegmentContext,
    Segments,
};

use crate::config::{ControllerConfig, MtqConfig, ReactionWheelConfig, SensorChoice};
use crate::satellite::SatelliteSpec;
#[cfg(feature = "plugin-wasm")]
use crate::sim::params::ResolvedPluginBackend;
use crate::sim::params::SimParams;

#[cfg(feature = "plugin-wasm")]
use orts::plugin::wasm::WasmPluginCache;

/// Shared build context for constructing multiple controlled satellites.
///
/// Holds resources that should be shared across all satellites in a
/// simulation (e.g. the WASM engine + compiled component cache), so
/// that 1000 satellites don't each pay the full WASM compilation cost.
pub struct ControlledBuildContext<'a> {
    pub params: &'a SimParams,
    #[cfg(feature = "plugin-wasm")]
    pub wasm_cache: &'a mut WasmPluginCache,
    /// Which WASM backend to build controllers with. Resolved once by
    /// the caller (based on `--plugin-backend` and fleet size).
    #[cfg(feature = "plugin-wasm")]
    pub plugin_backend: ResolvedPluginBackend,
}

/// Why a satellite stopped being propagated, and the time of the state it
/// stopped at.
///
/// `t` is where the condition was detected, which is one of two places: the
/// start of the span, when the state handed to [`advance_controlled`] already
/// satisfies it, or the end of the step that first landed past the boundary
/// (the surface, or the top of the atmosphere where the body has one).
/// Neither is the crossing itself — the same meaning the event checker has in
/// [`orts::group::IndependentGroup`].
#[derive(Debug, Clone)]
pub struct Termination {
    pub t: f64,
    pub reason: String,
}

/// 制御付き衛星の状態。
pub struct ControlledSatellite {
    pub dynamics: SpacecraftDynamics<Box<dyn GravityField>>,
    pub state: AugmentedState<SpacecraftState>,
    /// Sim time `state` belongs to [s].
    ///
    /// Starts at the time the satellite entered the simulation, and moves with
    /// `state`: to a span's end when the span completes, or to where the
    /// termination check stopped it. A terminated satellite keeps the time it
    /// stopped at.
    pub state_t: f64,
    /// Set when the termination check broke on this satellite. It is then not
    /// propagated, ticked, or sampled again.
    pub terminated: Option<Termination>,
    pub controller: Box<dyn PluginController>,
    pub sensors: SensorBundle,
    pub actuators: ActuatorBundle,
    /// RW effector が登録されているかどうか。
    pub has_rw: bool,
    /// MTQ model が登録されているかどうか。
    pub has_mtq: bool,
    /// MTQ per-axis max moment [A·m²] (for rebuilding the model).
    pub mtq_max_moment: f64,
    /// Central body this satellite orbits.
    ///
    /// A commanded MTQ is rebuilt on every tick that carries a command, and the
    /// rebuild has to pick the same field model the first build did — so it
    /// needs the body, not just the moment.
    pub body: arika::body::KnownBody,
    /// Thruster specs (空なら thruster なし)。ZOH 境界で ThrusterAssembly を
    /// 作り直すために保持する。
    pub thruster_specs: Vec<ThrusterSpec>,
    /// Sim time this satellite's controller schedule is anchored at [s]: where
    /// the satellite entered the simulation.
    tick_base_t: f64,
    /// Ticks this controller has already run since `tick_base_t`.
    ///
    /// The schedule belongs to the satellite because `sample_period` is fixed
    /// per controller and a fleet may mix rates. Output samples, stream flushes
    /// and the end of a run all cut the timeline at their own boundaries, and
    /// keeping the count here is what stops those cuts from moving the
    /// controller.
    ///
    /// A count rather than a running `next_tick_t += sample_period`: the sum
    /// drifts by a few ULPs per tick, which then has to be absorbed by a
    /// tolerance on every boundary comparison — and a tolerance wide enough to
    /// cover the drift also fires ticks that lie just past the boundary.
    ticks_done: u64,
}

impl ControlledSatellite {
    /// Assemble one for a test in another module of this crate, with no
    /// actuators and the tick schedule at its start.
    ///
    /// `build_controlled_satellite` needs a plugin guest, which a unit test
    /// cannot build, and the two schedule fields are private to this module —
    /// they are what decides when the controller runs, and nothing outside
    /// should set them.
    #[cfg(test)]
    pub(crate) fn for_test(
        dynamics: SpacecraftDynamics<Box<dyn GravityField>>,
        state: AugmentedState<SpacecraftState>,
        controller: Box<dyn PluginController>,
        body: arika::body::KnownBody,
    ) -> Self {
        Self {
            dynamics,
            state,
            state_t: 0.0,
            terminated: None,
            controller,
            sensors: SensorBundle::default(),
            actuators: ActuatorBundle::new(),
            has_rw: false,
            has_mtq: false,
            mtq_max_moment: 0.0,
            body,
            thruster_specs: Vec::new(),
            tick_base_t: 0.0,
            ticks_done: 0,
        }
    }

    /// Sim time of this satellite's next controller tick [s].
    pub fn next_tick_t(&self) -> f64 {
        self.tick_base_t + (self.ticks_done + 1) as f64 * self.controller.sample_period()
    }

    /// Whether the next tick lands at or before `t`.
    pub fn tick_due_at(&self, t: f64) -> bool {
        self.next_tick_t() <= t
    }
}

/// Reject a sample period too small to move the clock from `start_t`.
///
/// [`crate::config::validate_sample_period`] only asks for a positive finite
/// period, which `1e-16` satisfies — and `5.0 + 1e-16 == 5.0`, so the schedule
/// would never leave `start_t` and every loop waiting on it would spin.
///
/// What this guarantees is that the schedule leaves the anchor. A tick time is
/// `base + n · period`, and an ULP grows with the magnitude of the value, so a
/// period that clears one ULP at the anchor does not clear one at every later
/// tick: far enough out, `base + n · period` and `base + (n+1) · period` round
/// together. That is not a run anyone reaches — a 0.1 s period needs t of about
/// 1e15 s, some 32 million years, before an ULP catches up with it — and this
/// check does not rule it out. What it does rule out is a period that cannot
/// separate the first two ticks from each other at the start.
fn validate_tick_advances(start_t: f64, sample_period: f64) -> Result<(), String> {
    // At least one ULP, so the ticks this schedule generates near the start
    // land on different f64 values. Later ticks are the runtime guard's
    // business: the ULP grows with the time, and no check here can bound
    // that (see the doc above).
    //
    // Sampling the first few ticks instead is not enough. `start_t + period >
    // start_t` accepts a period that separates the first tick from the start
    // but not the ticks from each other (measured: at `start_t = 5.0` and
    // `period = 5e-16`, ticks 1 and 2 both round to 5.000000000000001).
    // Checking two ticks is not enough either: `3 · f64::EPSILON` is 0.75 ULP
    // there, so ticks 1 and 2 do advance while tick 3 rounds back onto tick 2
    // (5.000000000000002 twice), and the controller would run twice at one
    // instant.
    //
    // The ULP grows with the magnitude of the time, so a period accepted here
    // can still collide far enough into a run. That is the case the doc above
    // describes and does not rule out: a 0.1 s period needs t of about 1e15 s.
    // The resolution at the anchor, and at the first tick: crossing a binade
    // doubles the ULP, so a period equal to the anchor's can be half of one
    // just above it. Measured: at `start_t = 8.0f64.next_down()` with
    // `period = 8.0 - start_t`, ticks 1 and 2 both land on 8.0.
    let ulp = start_t.next_up() - start_t;
    let first = start_t + sample_period;
    let ulp_at_first = first.next_up() - first;
    if sample_period >= ulp && sample_period >= ulp_at_first {
        Ok(())
    } else {
        // Name the resolution that the period actually fell short of: at a
        // binade boundary the anchor's is met and the first tick's is not.
        let needed = ulp.max(ulp_at_first);
        Err(format!(
            "controller sample period {sample_period} is below the sim clock's \
             resolution around t={start_t} ({needed}), so consecutive ticks \
             would land on the same instant"
        ))
    }
}

/// Install a spacecraft's propellant and the assembly that draws on it.
///
/// The propellant is the spacecraft's, so the pool is registered with
/// [`with_propellant`](SpacecraftDynamics::with_propellant) and the assembly
/// with [`with_propulsion`](SpacecraftDynamics::with_propulsion): that pairing
/// is what stops the burn once the tank is empty, where `with_model` would keep
/// it thrusting. A function rather than two lines inside the builder, so that
/// the case walking a depletion (`the_controlled_loop_stops_a_burn_at_the_propellant_floor`)
/// covers the wiring a run uses instead of a copy of it — the builder itself
/// needs a plugin guest, which a unit test cannot load.
fn install_thrusters(
    dynamics: SpacecraftDynamics<Box<dyn GravityField>>,
    specs: Vec<ThrusterSpec>,
    dry_mass: f64,
) -> SpacecraftDynamics<Box<dyn GravityField>> {
    let core = ThrusterAssemblyCore::new(specs);
    dynamics
        .with_propellant(orts::spacecraft::PropellantPool::new(dry_mass))
        .with_propulsion(ThrusterAssembly::new(core))
}

/// Config からプラグイン制御付き衛星を構築する。
///
/// `start_t` is the sim time this satellite starts at [s]: 0 for a fleet built
/// before the run, the current sim time for one added to a running `serve`. It
/// sets the phase of the satellite's controller schedule.
///
/// 複数衛星をループで構築する場合は、[`ControlledBuildContext`] 内の
/// `wasm_cache` を使い回すことで WASM コンポーネントのコンパイルが
/// 1 ファイルにつき 1 回だけで済む。
/// `initial_epoch` is the wall-clock instant at which the orbital initial
/// state is evaluated: the simulation epoch for a satellite present from the
/// start, or the simulation epoch advanced by the current sim time for a
/// dynamic add (so a TLE/OMM is propagated to the moment it enters). The
/// dynamics themselves use `params.epoch` as the `t = 0` reference, so
/// time-dependent force models stay aligned regardless of when the satellite
/// is added.
pub fn build_controlled_satellite(
    spec: &SatelliteSpec,
    initial_epoch: Option<Epoch>,
    start_t: f64,
    ctx: &mut ControlledBuildContext<'_>,
) -> Result<ControlledSatellite, String> {
    let params = ctx.params;

    let att = spec
        .attitude_config
        .as_ref()
        .ok_or("controller requires attitude config")?;
    let ctrl_config = spec
        .controller_config
        .as_ref()
        .ok_or("controlled satellite requires controller config")?;

    let third_bodies = default_third_bodies(&params.body)
        .map_err(|e| format!("central body {}: {e}", params.body.properties().name))?;

    // Dynamics を構築。
    let mut dynamics = spacecraft_dynamics_for(spec, att, params, &third_bodies)?;

    // RW を追加。
    let has_rw = spec.rw_config.is_some();
    if let Some(rw_config) = &spec.rw_config {
        let rw = match rw_config {
            ReactionWheelConfig::ThreeAxis {
                inertia,
                max_momentum,
                max_torque,
                speed_control_gain,
            } => {
                let mut rw =
                    ReactionWheelAssembly::three_axis(*inertia, *max_momentum, *max_torque);
                if let Some(gain) = speed_control_gain {
                    rw.speed_control_gain = *gain;
                }
                rw
            }
        };
        dynamics = dynamics.with_effector(rw);
    }

    // MTQ を追加。
    let has_mtq = spec.mtq_config.is_some();
    let mtq_max_moment = match &spec.mtq_config {
        Some(MtqConfig::ThreeAxis { max_moment }) => {
            warn_no_field_model(params.body, "magnetorquer", "its torque is zero", &spec.id);
            dynamics = dynamics.with_model(mtq_for_body(params.body, *max_moment, None));
            *max_moment
        }
        None => 0.0,
    };

    // Thruster を追加。
    let thruster_specs = if let Some(cfg) = &spec.thruster_config {
        let specs: Vec<ThrusterSpec> = cfg
            .thrusters
            .iter()
            .map(|t| {
                let mut s = ThrusterSpec::new(
                    t.thrust_n,
                    t.isp_s,
                    Vector3::from_row_slice(&t.direction_body),
                );
                if let Some(off) = t.offset_body {
                    s = s.with_offset(Vector3::from_row_slice(&off));
                }
                s
            })
            .collect();
        dynamics = install_thrusters(dynamics, specs.clone(), cfg.dry_mass);
        specs
    } else {
        Vec::new()
    };

    // 初期状態。`initial_epoch` で評価（動的追加なら epoch + current_t）。
    let orbit = spec.initial_state(params.mu, initial_epoch)?;
    let plant = SpacecraftState {
        orbit,
        attitude: orts::attitude::AttitudeState {
            quaternion: att.normalized_initial_quaternion(),
            angular_velocity: nalgebra::Vector3::from_row_slice(&att.initial_angular_velocity),
        },
        mass: att.mass,
    };
    let state = dynamics.initial_augmented_state(plant);

    // コントローラを構築（cache 経由）。宣言された stream-io stream も
    // ここで配線される（serve が WS endpoint として公開する）。
    let controller = build_controller(ctrl_config, &spec.id, &spec.streams, ctx)?;

    // センサを構築。
    let sensors = build_sensor_bundle(spec.sensor_choices.as_deref(), params.body, &spec.id)?;

    let actuators = ActuatorBundle::new();
    let sample_period = controller.sample_period();
    crate::config::validate_sample_period(sample_period)?;
    validate_tick_advances(start_t, sample_period)?;

    Ok(ControlledSatellite {
        dynamics,
        state,
        state_t: start_t,
        terminated: None,
        controller,
        sensors,
        actuators,
        has_rw,
        has_mtq,
        mtq_max_moment,
        body: params.body,
        thruster_specs,
        tick_base_t: start_t,
        ticks_done: 0,
    })
}

/// Push the commands the actuator bundle currently holds into the dynamics.
///
/// Called right after a controller tick, so any span propagated afterwards is
/// pure integration under a held command. A command the controller did not name
/// keeps its previous value — the zero-order hold the plugin contract promises —
/// so this is also what carries a command across a span with no tick in it.
fn apply_held_commands(sat: &mut ControlledSatellite) -> Result<(), String> {
    // 前 tick のコマンドで RW を設定。
    if sat.has_rw
        && sat.actuators.has_rw_command()
        && let Some(rw) = sat
            .dynamics
            .effector_by_name_mut::<ReactionWheelAssembly>("reaction_wheels")
    {
        use orts::plugin::RwCommand;
        if let Some(rw_cmd) = sat.actuators.rw_command() {
            let cmd_len = match rw_cmd {
                RwCommand::Torques(v) | RwCommand::Speeds(v) => v.len(),
            };
            if cmd_len != rw.wheels().len() {
                return Err(format!(
                    "rw command length ({}) != wheel count ({})",
                    cmd_len,
                    rw.wheels().len()
                ));
            }
            rw.command = rw_cmd.clone();
        }
    }

    // 前 tick のコマンドで MTQ を設定（モデルを差し替え）。
    if sat.has_mtq
        && sat.actuators.has_mtq_command()
        && let Some(mtq_cmd) = sat.actuators.mtq_command()
    {
        let cmd_len = match mtq_cmd {
            MtqCommand::Moments(v) | MtqCommand::NormalizedMoments(v) => v.len(),
        };
        let num_mtqs = orts::spacecraft::MtqAssemblyCore::three_axis(sat.mtq_max_moment).num_mtqs();
        if cmd_len != num_mtqs {
            return Err(format!(
                "mtq command length ({cmd_len}) != MTQ count ({num_mtqs})"
            ));
        }
        // Same factory as the initial build, so the field model stays the one
        // this body has.
        let rebuilt = mtq_for_body(sat.body, sat.mtq_max_moment, Some(&mtq_cmd.clone()));
        sat.dynamics.replace_model("mtq_assembly", rebuilt);
    }

    // 前 tick のコマンドで Thruster を設定（モデルを差し替え）。
    // TODO: specs.clone() のコストが気になったら、
    // dynamics.model_by_name_mut::<ThrusterAssembly>() を追加して
    // in-place で command だけ書き換える方式に移行する（MTQ も同様）。
    if !sat.thruster_specs.is_empty()
        && sat.actuators.has_thruster_command()
        && let Some(thruster_cmd) = sat.actuators.thruster_command()
    {
        use orts::plugin::ThrusterCommand;
        let ThrusterCommand::Throttles(v) = thruster_cmd;
        if v.len() != sat.thruster_specs.len() {
            return Err(format!(
                "thruster command length ({}) != thruster count ({})",
                v.len(),
                sat.thruster_specs.len()
            ));
        }
        let core = ThrusterAssemblyCore::new(sat.thruster_specs.clone());
        let mut assembly = ThrusterAssembly::new(core);
        assembly.command = thruster_cmd.clone();
        if sat
            .dynamics
            .replace_model("thruster_assembly", Box::new(assembly))
            .is_none()
        {
            return Err("thruster_assembly model not registered in dynamics".into());
        }
    }
    Ok(())
}

/// The next moment anything happens in a fleet: the earliest controller tick
/// due, or `horizon` if none falls before it.
///
/// An empty fleet has no tick, so the horizon is the answer. Taking the
/// shortest `sample_period` in the fleet instead — one tick rate for everyone
/// — ran every slower controller at that rate.
pub fn next_fleet_event_t(sats: &[ControlledSatellite], horizon: f64) -> f64 {
    sats.iter()
        .filter(|sat| sat.terminated.is_none())
        .map(|sat| sat.next_tick_t())
        .fold(f64::INFINITY, f64::min)
        .min(horizon)
}

/// Advance one satellite across `[from, to]`.
///
/// Propagates to each tick due inside the span, ticks the controller there,
/// then propagates the rest of the span under the command that tick left. The
/// span is the caller's — `stream_interval` in `serve`, the gap between fleet
/// events in `run` — and has no reason to be a multiple of the controller's
/// period, so a span shorter than the period integrates without a tick.
///
/// Takes `params` rather than an integrator and an epoch so that both callers
/// read the same fields: the integrator selection lives in
/// [`SimParams::integrator_config`] and neither `run` nor `serve` can hand this
/// loop a different one. That is the shape the fix here needed — the selection
/// used to be the caller's to make, and the controlled callers made a different
/// one from the groups.
pub fn advance_controlled(
    sat: &mut ControlledSatellite,
    from: f64,
    to: f64,
    params: &SimParams,
) -> Result<Option<Termination>, String> {
    // Already terminated: nothing to propagate, and nothing new to report.
    // The caller distinguishes "stopped now" from "stopped earlier" by this.
    if sat.terminated.is_some() {
        return Ok(None);
    }

    let integrator = params.integrator_config();
    // The count `SimParams` derived came from the run's own span, and this
    // call's target can be further away than that (see `at_least_for_span`).
    let search = crate::sim::params::at_least_for_span(params.root_search, to - from);
    let epoch = params.epoch.as_ref();
    let check = crate::sim::core::body_event_checker::<AugmentedState<SpacecraftState>>(params);

    // A satellite can be added below the surface, and the steppers check their
    // own initial state — but only of the span they are given, so a span that
    // is never integrated (`to <= from`) would slip past.
    if let ControlFlow::Break(reason) = check(from, &sat.state) {
        let term = Termination { t: from, reason };
        sat.terminated = Some(term.clone());
        return Ok(Some(term));
    }

    let mut t = from;
    while sat.tick_due_at(to) {
        let tick_t = sat.next_tick_t();
        // The controller is not called at an instant the satellite did not
        // reach: termination wins over a tick at the same time.
        if let Some(term) = propagate_controlled(sat, t, tick_t, &integrator, search, &check)? {
            return Ok(Some(term));
        }
        tick_controller(sat, tick_t, epoch)?;
        t = tick_t;
    }
    propagate_controlled(sat, t, to, &integrator, search, &check)
}

/// Integrate `[t0, t1]` under the command the actuators already hold.
///
/// No controller call: `t1` is wherever the caller needs the state next — an
/// output sample, a stream flush, the end of the run — and those boundaries do
/// not have to be controller ticks. `PluginController::sample_period` is a
/// *fixed* period, so a controller runs on its own schedule via
/// [`tick_controller`] and nothing else may move it.
///
/// `integrator` carries the step the config asked for. For `Rk4` that is the
/// fixed step, capped by the span so it cannot reach past `t1`; for the adaptive
/// pair it is the first step the controller tries.
///
/// The adaptive steppers are rebuilt for each span rather than carried across
/// them, which is what `IndependentGroup` does for its own spans. Keeping a
/// stepper's own `dt` instead would drag the truncated last step of one span
/// into the next: `advance_to` computes `h = dt.min(t_target - t)` and then
/// stores `h * factor`, so a span boundary would shrink the step the next span
/// starts from.
pub fn propagate_controlled<E>(
    sat: &mut ControlledSatellite,
    t0: f64,
    t1: f64,
    integrator: &IntegratorConfig,
    search: RootSearch,
    event_check: &E,
) -> Result<Option<Termination>, String>
where
    E: Fn(f64, &AugmentedState<SpacecraftState>) -> ControlFlow<String>,
{
    // Anything that can say what went wrong: the solver's own errors, and the
    // system's refusal of the state a boundary walk was handed. The wording is
    // about the span rather than about integrating, since a refused state is
    // not something the solver failed at.
    let span = |e: &dyn std::fmt::Display| format!("propagation failed on [{t0:.3}, {t1:.3}]: {e}");

    // Before the no-op guard: `t1 <= t0` is true for a `t0` of `+inf`, so an
    // invalid span would be reported as one already covered. `Segments::new`
    // rejects it too, but it is reached only past this guard, and a span that
    // runs backwards stays the no-op it has always been rather than becoming
    // an error.
    if !t0.is_finite() || !t1.is_finite() {
        return Err(span(&IntegrationError::InvalidTimeSpan { t0, t_end: t1 }));
    }
    if t1 <= t0 {
        return Ok(None);
    }

    // One segment at a time, so that no switch of the right-hand side falls
    // strictly inside a step and the stage on a segment's end reads the mode
    // that held inside it. Without this, a burn window narrower than the
    // largest gap between adjacent stage times contributes nothing at all, and
    // one covering a whole step loses the weight of the stage on its exclusive
    // end. `Segments::new` also rejects a span no loop can walk: `t1 <= t0` is
    // false for a NaN, and so is a loop's own `t < t1`, so nothing here would
    // step and the solvers would never see the span.
    //
    // The state stays local until every segment has succeeded. A satellite
    // carries no integration time of its own, so committing each segment would
    // leave a failed span with the state at the last boundary while the caller
    // still holds `t0` — and serve pauses on such an error and can resume from
    // there, propagating a state that belongs to a later instant.
    let mut state = sat.state.clone();

    // The boundaries this satellite's effectors declare, and one guard each.
    // Both live across the segments below, so that a boundary reported at one
    // segment's end is not reported again at the next one's start.
    let boundaries = sat.dynamics.boundaries();
    let mut slots = vec![RootSlot::new(); boundaries.len()];

    for segment in Segments::new(&sat.dynamics, t0, t1).map_err(|e| span(&e))? {
        let t = segment.start();
        let segment_end = segment.end();
        let bound = segment.system();

        // A later segment starts from a state the check has already
        // accepted, and the steppers document that asking again about the same
        // `(t, state)` can change what a stateful predicate answers.
        let started_checked = segment.is_continuation();
        let segment_times = SegmentContext::new(t, segment_end);
        let mut observe = |_: f64, _: &AugmentedState<SpacecraftState>| {};

        // The walk rather than `try_integrate`: the same steps, but it takes
        // the event predicate and stops at the boundaries the effectors
        // declared. `integrate` panics on a bad step or a stalled clock, and
        // this path returns `Result` so serve can send the client an Error
        // down its graceful-halt path. A `dt` wider than the segment is not
        // clamped — the last step of a segment lands on `segment_end` itself,
        // which is what a segment ending at a switch of the right-hand side
        // needs.
        let (walk, reached_t, next_state) = match integrator {
            IntegratorConfig::Rk4 { dt } => walk_to_target(
                Boundaries {
                    system: &sat.dynamics,
                    declared: &boundaries,
                    slots: &mut slots,
                    search,
                    segment: Some(&segment_times),
                },
                Span {
                    from: t,
                    to: segment_end,
                    start_is_checked: started_checked,
                },
                state.clone(),
                |state, t, checked| {
                    let stepper = Rk4.stepper(bound, state, t, *dt);
                    if checked {
                        stepper.from_checked_state()
                    } else {
                        stepper
                    }
                },
                &mut observe,
                event_check,
            ),
            IntegratorConfig::Dp45 { dt, tolerances } => walk_to_target(
                Boundaries {
                    system: &sat.dynamics,
                    declared: &boundaries,
                    slots: &mut slots,
                    search,
                    segment: Some(&segment_times),
                },
                Span {
                    from: t,
                    to: segment_end,
                    start_is_checked: started_checked,
                },
                state.clone(),
                |state, t, checked| {
                    let stepper = DormandPrince.stepper(bound, state, t, *dt, tolerances.clone());
                    if checked {
                        stepper.from_checked_state()
                    } else {
                        stepper
                    }
                },
                &mut observe,
                event_check,
            ),
            IntegratorConfig::Dop853 { dt, tolerances } => walk_to_target(
                Boundaries {
                    system: &sat.dynamics,
                    declared: &boundaries,
                    slots: &mut slots,
                    search,
                    segment: Some(&segment_times),
                },
                Span {
                    from: t,
                    to: segment_end,
                    start_is_checked: started_checked,
                },
                state.clone(),
                |state, t, checked| {
                    let stepper = Dop853.stepper(bound, state, t, *dt, tolerances.clone());
                    if checked {
                        stepper.from_checked_state()
                    } else {
                        stepper
                    }
                },
                &mut observe,
                event_check,
            ),
        }
        .map_err(|e| span(&e))?;

        state = next_state;

        // An event is a normal early stop: commit where it stopped and leave
        // the remaining segments alone. No propagation to the segment end, and
        // no rounding of the time.
        if let BoundaryWalk::Stopped(reason) = walk {
            sat.state = state;
            sat.state_t = reached_t;
            let term = Termination {
                t: reached_t,
                reason,
            };
            sat.terminated = Some(term.clone());
            return Ok(Some(term));
        }
    }

    sat.state = state;
    sat.state_t = t1;
    Ok(None)
}

/// Run one controller tick at `t_next`: read the sensors, call the plugin, and
/// hand the command it returns to the dynamics.
///
/// The state must already have been propagated to `t_next` — the sensors are
/// read from it.
pub fn tick_controller(
    sat: &mut ControlledSatellite,
    t_next: f64,
    epoch: Option<&Epoch>,
) -> Result<(), String> {
    // The sensors would be read from a state that belongs to an earlier time.
    if sat.terminated.is_some() {
        return Ok(());
    }

    // センサ評価 + プラグイン呼び出し。
    let current_epoch = epoch.map(|e| e.add_si_seconds(t_next));
    let sensors = sat
        .sensors
        .evaluate(&sat.state.plant, &current_epoch.unwrap_or(Epoch::j2000()));
    let actuator_telemetry = ActuatorTelemetry {
        rw: if sat.has_rw {
            sat.dynamics
                .effector_by_name::<ReactionWheelAssembly>("reaction_wheels")
                .map(|rw| {
                    let core = rw.core();
                    let momentum = core.momentum_slice(&sat.state.aux);
                    RwTelemetry {
                        momentum: momentum.to_vec(),
                        speeds: momentum
                            .iter()
                            .zip(rw.wheels())
                            .map(|(h, w)| w.speed_from_momentum(*h))
                            .collect(),
                        realized_torques: core
                            .realized_torque_slice(&sat.state.aux)
                            .map(|s| s.to_vec()),
                    }
                })
        } else {
            None
        },
    };
    let input = TickInput {
        t: t_next,
        epoch: current_epoch.as_ref(),
        sensors: &sensors,
        actuators: &actuator_telemetry,
        spacecraft: &sat.state.plant,
    };
    if let Some(cmd) = sat
        .controller
        .update(&input)
        .map_err(|e| format!("controller error at t={t_next:.3}: {e}"))?
    {
        sat.actuators
            .apply(&cmd)
            .map_err(|e| format!("actuator error at t={t_next:.3}: {e}"))?;
    }
    // Only once the whole tick has landed: `apply_held_commands` can reject a
    // command whose length does not match the actuator, and a schedule advanced
    // past a tick that failed would resume on the wrong phase.
    apply_held_commands(sat)?;
    // The schedule has to advance, or the caller's `while sat.tick_due_at(to)`
    // would tick again at this instant and never finish. Construction refuses
    // a period below the clock's resolution there, but that resolution doubles
    // at each binade boundary, so a period wide enough at the anchor can be
    // too narrow further along. The invariant is checked where it has to hold
    // rather than only where the satellite was built.
    let taken = sat.next_tick_t();
    sat.ticks_done += 1;
    let following = sat.next_tick_t();
    // `partial_cmp` rather than `!(following > taken)`: a NaN either side is
    // incomparable, and that has to count as "did not advance" too.
    if following.partial_cmp(&taken) != Some(core::cmp::Ordering::Greater) {
        return Err(format!(
            "controller sample period {} cannot advance the schedule past \
             t={taken}: the next tick lands on the same instant",
            sat.controller.sample_period()
        ));
    }
    Ok(())
}

// builder helpers

fn build_controller(
    config: &ControllerConfig,
    label: &str,
    streams: &[String],
    ctx: &mut ControlledBuildContext<'_>,
) -> Result<Box<dyn PluginController>, String> {
    match config {
        #[cfg(feature = "plugin-wasm")]
        ControllerConfig::Wasm { path, config } => {
            // An omitted `[satellites.controller.config]` deserializes to
            // `Value::Null`, whose `to_string()` is `"null"` — not something a
            // guest can parse as its config struct. `Plugin::init` takes the
            // empty string to mean "use the defaults", which is what an absent
            // config block asks for.
            let config_str = if config.is_null() {
                String::new()
            } else {
                config.to_string()
            };
            let wasm_path = std::path::Path::new(path);
            match ctx.plugin_backend {
                ResolvedPluginBackend::Sync => {
                    let ctrl = ctx
                        .wasm_cache
                        .build_sync_controller_with_streams(
                            wasm_path,
                            label,
                            &config_str,
                            streams.to_vec(),
                            ctx.params.body,
                        )
                        .map_err(|e| format!("WasmController build failed: {e}"))?;
                    Ok(Box::new(ctrl))
                }
                #[cfg(feature = "plugin-wasm-async")]
                ResolvedPluginBackend::Async => {
                    let ctrl = ctx
                        .wasm_cache
                        .build_async_controller_with_streams(
                            wasm_path,
                            label,
                            &config_str,
                            streams.to_vec(),
                            ctx.params.body,
                        )
                        .map_err(|e| format!("AsyncWasmController build failed: {e}"))?;
                    Ok(Box::new(ctrl))
                }
            }
        }
        #[cfg(not(feature = "plugin-wasm"))]
        ControllerConfig::Wasm { .. } => {
            let _ = ctx;
            let _ = label;
            let _ = streams;
            Err("WASM controller requires the 'plugin-wasm' feature. \
             Rebuild with: cargo build --features plugin-wasm"
                .to_string())
        }
    }
}

/// Whether `body`'s magnetic field is modelled.
///
/// The same rule the WASM host uses for `magnetic-field-eci`, so a run's
/// devices and its plugin agree about what the field is.
fn body_field_is_modelled(body: arika::body::KnownBody) -> bool {
    orts::magnetic::field_is_modelled(body)
}

/// How many warnings [`warn_no_field_model`] has emitted on this thread.
///
/// The MTQ assembly is rebuilt on every tick that carries a command, so the
/// warning has to sit outside that path. A test drives the rebuild and reads
/// this back: the count stays where construction left it. Per-thread because
/// tests run in parallel in one process, and each warning happens on the
/// thread that built the device.
#[cfg(test)]
thread_local! {
    static FIELD_WARNINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Warn that `device` has no field model on `body`, once where it is built.
///
/// Around a body without a model there is nothing to evaluate, and the stand-in
/// is zero: the device is built either way — the same spacecraft definition can
/// be pointed at any body — and is inert there rather than driven by a field
/// measured somewhere else. `effect` says what that means for this device,
/// since a controller that steers on the field goes quiet without failing.
fn warn_no_field_model(body: arika::body::KnownBody, device: &str, effect: &str, sat_id: &str) {
    if body_field_is_modelled(body) {
        return;
    }
    #[cfg(test)]
    FIELD_WARNINGS.with(|n| n.set(n.get() + 1));
    log::warn!(
        "{sat_id}: {device} has no magnetic field model for {} (only Earth's is modelled), \
         so {effect}. Control that steers on the field, such as B-dot, has nothing to act on.",
        body.properties().name
    );
}

/// The magnetorquer assembly for `body`, with `command` already applied.
///
/// The assembly holds its field model as a type parameter, so the two cases are
/// two types; boxing them here keeps the choice in one place. Both the initial
/// build and the per-command rebuild go through this, so a magnetorquer cannot
/// end up on Earth's field after the first command. Silent: the rebuild runs on
/// every tick that carries a command, and the warning belongs where the device
/// is built.
fn mtq_for_body(
    body: arika::body::KnownBody,
    max_moment: f64,
    command: Option<&MtqCommand>,
) -> Box<dyn orts::model::Model<orts::spacecraft::SpacecraftState>> {
    if body_field_is_modelled(body) {
        let mut mtq = MtqAssembly::three_axis(max_moment, Igrf::earth());
        if let Some(cmd) = command {
            mtq.command = cmd.clone();
        }
        Box::new(mtq)
    } else {
        let mut mtq = MtqAssembly::three_axis(max_moment, tobari::magnetic::NoField);
        if let Some(cmd) = command {
            mtq.command = cmd.clone();
        }
        Box::new(mtq)
    }
}

/// Build the declared sensors for a satellite about `body`.
///
/// The sun sensor's reading is a direction to the Sun, so it depends on the
/// central body the same way the solar force models do.
fn build_sensor_bundle(
    choices: Option<&[SensorChoice]>,
    body: arika::body::KnownBody,
    sat_id: &str,
) -> Result<SensorBundle, String> {
    let choices = match choices {
        Some(c) => c,
        None => return Ok(SensorBundle::new()),
    };

    let magnetometers = if choices.contains(&SensorChoice::Magnetometer) {
        warn_no_field_model(body, "magnetometer", "its reading is zero", sat_id);
        let field: Arc<dyn tobari::magnetic::MagneticFieldModel> = if body_field_is_modelled(body) {
            Arc::new(Igrf::earth())
        } else {
            Arc::new(tobari::magnetic::NoField)
        };
        vec![Magnetometer::new(field)]
    } else {
        vec![]
    };

    Ok(SensorBundle {
        magnetometers,
        gyroscopes: if choices.contains(&SensorChoice::Gyroscope) {
            vec![Gyroscope::new()]
        } else {
            vec![]
        },
        star_trackers: if choices.contains(&SensorChoice::StarTracker) {
            vec![StarTracker::new()]
        } else {
            vec![]
        },
        sun_sensors: if choices.contains(&SensorChoice::SunSensor) {
            vec![orts::sensor::SunSensor::for_body(body).map_err(|e| format!("sun sensor: {e}"))?]
        } else {
            vec![]
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::body::KnownBody;
    use orts::plugin::{Command, PluginError, TickInput};

    /// A controller that records the `t` of every tick it is given.
    ///
    /// The point of the tests below is *when* the controller runs, so it
    /// commands nothing and only keeps the schedule it saw.
    struct TickRecorder {
        period: f64,
        ticks: Arc<std::sync::Mutex<Vec<f64>>>,
    }

    impl PluginController for TickRecorder {
        fn name(&self) -> &str {
            "tick-recorder"
        }
        fn sample_period(&self) -> f64 {
            self.period
        }
        fn update(&mut self, input: &TickInput<'_>) -> Result<Option<Command>, PluginError> {
            self.ticks
                .lock()
                .expect("no panics in these tests")
                .push(input.t);
            Ok(None)
        }
    }

    /// Build a controlled satellite at 400 km with the given controller.
    ///
    /// Mirrors the dynamics `build_controlled_satellite` assembles, minus the
    /// actuators and the WASM plugin the real builder needs: what is under test
    /// is the tick schedule, and no command is ever issued.
    fn satellite_with(
        period: f64,
        start_t: f64,
    ) -> (ControlledSatellite, Arc<std::sync::Mutex<Vec<f64>>>) {
        use orts::orbital::OrbitalState;
        use orts::spacecraft::SpacecraftState;

        let ticks = Arc::new(std::sync::Mutex::new(Vec::new()));
        let controller = TickRecorder {
            period,
            ticks: Arc::clone(&ticks),
        };

        let body = arika::body::KnownBody::Earth;
        let mu = body.properties().mu;
        let inertia = nalgebra::Matrix3::identity() * 10.0;
        let dynamics = orts::setup::build_spacecraft_dynamics(
            &body,
            orts::setup::CentralGravity::Zonal { mu: mu },
            None,
            &orts::setup::SatelliteParams {
                has_drag: false,
                ballistic_coeff: None,
                srp_area_to_mass: None,
                srp_cr: None,
                // This test watches the controller's tick cadence, so the plant
                // carries no disturbance torque and no panels to make one.
                disturbances: orts::setup::DisturbanceTorques::default(),
                shape: None,
            },
            &[],
            inertia,
            None,
        )
        .expect("Earth has a Sun ephemeris");

        // Circular orbit 400 km up, in the equatorial plane.
        let r = body.properties().radius + 400.0;
        let v = (mu / r).sqrt();
        let plant = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0)),
            attitude: orts::attitude::AttitudeState {
                quaternion: nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 500.0,
        };
        let state = dynamics.initial_augmented_state(plant);

        let sat = ControlledSatellite {
            dynamics,
            state,
            state_t: 0.0,
            terminated: None,
            controller: Box::new(controller),
            sensors: SensorBundle::default(),
            actuators: ActuatorBundle::new(),
            has_rw: false,
            has_mtq: false,
            mtq_max_moment: 0.0,
            body: arika::body::KnownBody::Earth,
            thruster_specs: Vec::new(),
            tick_base_t: start_t,
            ticks_done: 0,
        };
        (sat, ticks)
    }

    /// The satellite from `satellite_with`, moved to 110 km and dropped
    /// straight down at 1 km/s: it crosses Earth's `atmosphere_altitude`
    /// (100 km, the Kármán line) about ten seconds later.
    fn falling_satellite(period: f64) -> (ControlledSatellite, Arc<std::sync::Mutex<Vec<f64>>>) {
        let (mut sat, ticks) = satellite_with(period, 0.0);
        let r = KnownBody::Earth.properties().radius + 110.0;
        sat.state.plant.orbit = orts::orbital::OrbitalState::new(
            Vector3::new(r, 0.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
        );
        (sat, ticks)
    }

    /// The search the caller chose reaches this path.
    ///
    /// Not through the state it ends on: the total is conserved, so the body's
    /// rate follows from the wheel's momentum whenever the bound was located,
    /// and the wheel is on its bound by the end of the span either way. What
    /// the tolerance decides is the instant in between, which this path
    /// reports to nobody. So the forwarding is shown the other way round —
    /// with a search no walk accepts, which fails only if it arrives.
    #[test]
    fn the_search_the_caller_chose_reaches_the_controlled_path() {
        use orts::spacecraft::{ReactionWheelAssembly, RwCommand};

        let saturating = || {
            let (mut sat, _ticks) = satellite_with(1000.0, 0.0);
            let mut rw = ReactionWheelAssembly::three_axis(0.01, 0.53, 0.1);
            rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, 0.1)));
            sat.dynamics = std::mem::replace(
                &mut sat.dynamics,
                orts::spacecraft::SpacecraftDynamics::new(
                    arika::body::KnownBody::Earth.properties().mu,
                    Box::new(orts::orbital::gravity::PointMass) as Box<dyn GravityField>,
                    nalgebra::Matrix3::identity(),
                ),
            )
            .with_effector(rw);
            sat.state = sat
                .dynamics
                .initial_augmented_state(sat.state.plant.clone());
            sat
        };
        let walk = |search: RootSearch| {
            propagate_controlled(
                &mut saturating(),
                0.0,
                20.0,
                &IntegratorConfig::Rk4 { dt: 0.25 },
                search,
                &|_: f64, _: &AugmentedState<SpacecraftState>| ControlFlow::Continue(()),
            )
        };

        assert!(walk(RootSearch::default()).is_ok(), "the default walks");

        // A width no bisection reaches. `validate_root_t_tolerance` refuses it
        // at config time; a state assembled by hand can still carry it, and
        // the walk is where it stops.
        let err = walk(RootSearch {
            t_tolerance: 0.0,
            ..RootSearch::default()
        })
        .expect_err("a search no walk accepts is refused where the walk starts");
        assert!(
            err.contains("tolerance"),
            "the error names what it could not use: {err}"
        );
    }

    /// The controlled path walks with the boundaries the effectors declare, so
    /// a wheel that saturates inside a span is held at its limit and what it
    /// stops taking stays with the spacecraft. Stepping without that handling
    /// would carry the wheel past its limit for the rest of the span, and the
    /// body would keep the reaction.
    #[test]
    fn a_saturating_wheel_is_held_on_the_controlled_path() {
        use orts::spacecraft::{ReactionWheelAssembly, RwCommand};

        const MAX_MOMENTUM: f64 = 0.53;
        const MAX_TORQUE: f64 = 0.1;
        const BODY_INERTIA: f64 = 10.0;

        // The z wheel takes a torque about z alone, and reaches its limit at
        // t = 5.3 s: between the ticks of the 0.25 s grid below.
        let (mut sat, _ticks) = satellite_with(1000.0, 0.0);
        let mut rw = ReactionWheelAssembly::three_axis(0.01, MAX_MOMENTUM, MAX_TORQUE);
        rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, MAX_TORQUE)));
        sat.dynamics = std::mem::replace(
            &mut sat.dynamics,
            orts::spacecraft::SpacecraftDynamics::new(
                arika::body::KnownBody::Earth.properties().mu,
                Box::new(orts::orbital::gravity::PointMass) as Box<dyn GravityField>,
                nalgebra::Matrix3::identity(),
            ),
        )
        .with_effector(rw);
        sat.state = sat
            .dynamics
            .initial_augmented_state(sat.state.plant.clone());

        // Body-frame total: the body's own plus the three wheels', which spin
        // about x, y and z. With an isotropic inertia and one axis driven, the
        // vector itself is constant.
        let total = |state: &AugmentedState<SpacecraftState>| {
            BODY_INERTIA * state.plant.attitude.angular_velocity
                + Vector3::new(state.aux[0], state.aux[1], state.aux[2])
        };
        let started_with = total(&sat.state);

        propagate_controlled(
            &mut sat,
            0.0,
            20.0,
            &IntegratorConfig::Rk4 { dt: 0.25 },
            RootSearch::default(),
            &|_: f64, _: &AugmentedState<SpacecraftState>| ControlFlow::Continue(()),
        )
        .expect("the span is finite");

        assert!(
            (sat.state.aux[2] + MAX_MOMENTUM).abs() < 1e-6,
            "the z wheel ends held at its lower bound, not at {}",
            sat.state.aux[2]
        );
        let lost = (total(&sat.state) - started_with).magnitude();
        assert!(
            lost < 1e-9,
            "{lost:.3e} N·m·s of the body-frame total went missing"
        );
    }

    #[test]
    fn every_integrator_stops_a_falling_satellite() {
        // The detection time is the first step end past the line, so it is the
        // step size that decides it: 1 s for Rk4, and whatever the adaptive
        // pair accepted for the others.
        for (integrator, expected_t) in [
            (crate::cli::IntegratorChoice::Rk4, 10.0),
            (crate::cli::IntegratorChoice::Dp45, 12.0),
            (crate::cli::IntegratorChoice::Dop853, 12.0),
        ] {
            let params = params_with(integrator, 1.0, 1e-9);
            let (mut sat, ticks) = falling_satellite(4.0);
            let term = advance_controlled(&mut sat, 0.0, 60.0, &params)
                .expect("the span is finite everywhere")
                .unwrap_or_else(|| panic!("{integrator:?} propagated past the Kármán line"));

            assert!(
                term.reason.contains("atmospheric entry"),
                "{integrator:?}: {}",
                term.reason
            );
            assert!(
                (term.t - expected_t).abs() < 1e-9,
                "{integrator:?} stopped at {} s, expected {expected_t}",
                term.t
            );
            assert_eq!(sat.state_t, term.t, "{integrator:?}: state and time agree");
            let alt =
                sat.state.plant.orbit.position().magnitude() - KnownBody::Earth.properties().radius;
            assert!(
                (0.0..100.0).contains(&alt),
                "{integrator:?} committed the state from {alt} km, which is not inside the atmosphere"
            );

            // A tick lands on 12 s, which is where the adaptive pair stops: the
            // controller must not be called at an instant the satellite did not
            // reach.
            let ticks = ticks.lock().expect("no panics in these tests").clone();
            assert!(
                ticks.iter().all(|&tick| tick < term.t),
                "{integrator:?} ticked at or past the termination: {ticks:?}"
            );
        }
    }

    #[test]
    fn an_event_in_a_middle_segment_leaves_the_later_ones_unrun() {
        use orts::spacecraft::{BurnWindow, ScheduledBurn, Thruster};

        // A burn window at [1, 3) cuts the span into [0,1), [1,3), [3,30].
        let (mut sat, _) = falling_satellite(100.0);
        sat.state.plant.orbit = orts::orbital::OrbitalState::new(
            Vector3::new(KnownBody::Earth.properties().radius + 110.0, 0.0, 0.0),
            Vector3::new(-6.0, 0.0, 0.0),
        );
        sat.dynamics = std::mem::replace(
            &mut sat.dynamics,
            orts::spacecraft::SpacecraftDynamics::new(
                arika::earth::MU,
                Box::new(orts::orbital::gravity::PointMass),
                nalgebra::Matrix3::identity(),
            ),
        )
        .with_model(
            Thruster::new(10.0, 300.0, Vector3::x()).with_profile(Box::new(ScheduledBurn {
                windows: vec![BurnWindow::full(1.0, 3.0)],
            })),
        );

        let params = params_with(crate::cli::IntegratorChoice::Rk4, 1.0, 1e-9);
        let term = advance_controlled(&mut sat, 0.0, 30.0, &params)
            .expect("the burn and the fall are finite")
            .expect("6 km/s down from 110 km crosses the line inside [1, 3)");

        assert!(term.t > 1.0 && term.t < 3.0, "stopped at {} s", term.t);
        assert_eq!(sat.state_t, term.t, "the committed time is the crossing's");
        let alt =
            sat.state.plant.orbit.position().magnitude() - KnownBody::Earth.properties().radius;
        assert!(
            (0.0..100.0).contains(&alt),
            "the state came from {alt} km, so a later segment ran"
        );
    }

    #[test]
    fn a_satellite_below_the_surface_stops_before_integrating() {
        let params = params_with(crate::cli::IntegratorChoice::Rk4, 1.0, 1e-9);
        let (mut sat, ticks) = falling_satellite(4.0);
        let inside = KnownBody::Earth.properties().radius * 0.5;
        sat.state.plant.orbit = orts::orbital::OrbitalState::new(
            Vector3::new(inside, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
        );

        let term = advance_controlled(&mut sat, 0.0, 60.0, &params)
            .expect("no integration to fail")
            .expect("a state below the surface has already terminated");
        assert!(term.reason.contains("collision"), "{}", term.reason);
        assert_eq!(term.t, 0.0, "it stopped where the span started");
        assert_eq!(sat.state_t, 0.0, "nothing was propagated");
        assert!(
            ticks.lock().expect("no panics in these tests").is_empty(),
            "the controller ran for a satellite that never flew"
        );
    }

    #[test]
    fn a_terminated_satellite_is_not_advanced_again() {
        let params = params_with(crate::cli::IntegratorChoice::Rk4, 1.0, 1e-9);
        let (mut sat, ticks) = falling_satellite(4.0);
        advance_controlled(&mut sat, 0.0, 60.0, &params)
            .expect("integrates")
            .expect("stops");
        let state_t = sat.state_t;
        let position = *sat.state.plant.orbit.position();
        let ticks_before = ticks.lock().expect("no panics in these tests").len();

        let again = advance_controlled(&mut sat, 60.0, 120.0, &params).expect("integrates");
        assert!(again.is_none(), "a termination must be reported once");
        assert_eq!(sat.state_t, state_t, "the state stayed where it stopped");
        assert_eq!(*sat.state.plant.orbit.position(), position);
        assert_eq!(
            ticks.lock().expect("no panics in these tests").len(),
            ticks_before,
            "the controller ran after the satellite stopped"
        );
    }

    #[test]
    fn the_fleet_clock_ignores_terminated_satellites() {
        let (mut dead, _) = satellite_with(1.0, 0.0);
        dead.terminated = Some(Termination {
            t: 0.5,
            reason: "atmospheric entry".to_string(),
        });
        let (alive, _) = satellite_with(7.0, 0.0);
        // The dead satellite's next tick is at 1 s, the live one's at 7 s.
        assert_eq!(next_fleet_event_t(&[dead, alive], 100.0), 7.0);
    }

    /// A predicate that lets every state through: these tests are about the
    /// integration, not about when a satellite stops.
    fn never_ends(_t: f64, _s: &AugmentedState<SpacecraftState>) -> ControlFlow<String> {
        ControlFlow::Continue(())
    }

    /// A `SimParams` carrying just the fields the controlled loop reads.
    ///
    /// Built from the CLI defaults so it is the same shape a run gets, then
    /// overridden field by field.
    fn params_with(integrator: crate::cli::IntegratorChoice, dt: f64, tol: f64) -> SimParams {
        use clap::Parser;
        let args = crate::cli::SimArgs::parse_from(["orts"]);
        let mut params = SimParams::from_sim_args(&args, false).expect("default args are valid");
        params.integrator = integrator;
        params.dt = dt;
        params.tolerances = utsuroi::Tolerances {
            atol: tol,
            rtol: tol,
        };
        params
    }

    /// Step one satellite the way a caller does — through the same function
    /// `serve` and `run` call, so an error in that loop fails these tests.
    fn advance(sat: &mut ControlledSatellite, from: f64, to: f64, params_dt: f64) {
        let params = params_with(crate::cli::IntegratorChoice::Rk4, params_dt, 1e-9);
        advance_controlled(sat, from, to, &params).expect("integrates and ticks");
    }

    /// A span shorter than the sample period does not tick the controller.
    ///
    /// This is the serve case: `stream_interval` cuts the timeline every
    /// 0.01 s while the controller asks for 0.1 s. The old loop called the
    /// controller once per cut with `dt = 0.01`, so a 10 Hz controller ran at
    /// 100 Hz and held each command for a tenth of the time it asked for.
    #[test]
    fn spans_shorter_than_the_period_do_not_tick_the_controller() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);

        // 1 s of sim time, cut into 0.01 s spans.
        let mut t = 0.0;
        for _ in 0..100 {
            advance(&mut sat, t, t + 0.01, 0.01);
            t += 0.01;
        }

        let seen = ticks.lock().unwrap().clone();
        assert_eq!(
            seen.len(),
            10,
            "10 Hz over 1 s is 10 ticks, got {} at {seen:?}",
            seen.len()
        );
        for (i, tick_t) in seen.iter().enumerate() {
            let expected = 0.1 * (i + 1) as f64;
            assert!(
                (tick_t - expected).abs() < 1e-9,
                "tick {i} at {tick_t}, expected {expected}"
            );
        }
    }

    /// Ticks stay on the controller's own phase across spans that do not
    /// divide it.
    ///
    /// 0.03 s spans never land on a 0.1 s tick, so every tick falls inside a
    /// span. Truncating the remainder instead of carrying it would drift the
    /// schedule.
    #[test]
    fn a_span_that_does_not_divide_the_period_keeps_the_phase() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);

        let mut t = 0.0;
        for _ in 0..10 {
            advance(&mut sat, t, t + 0.03, 0.01);
            t += 0.03;
        }
        // 0.3 s of sim time: ticks at 0.1, 0.2, 0.3.
        let seen = ticks.lock().unwrap().clone();
        assert_eq!(seen.len(), 3, "expected 3 ticks in 0.3 s, got {seen:?}");
        assert!((seen[2] - 0.3).abs() < 1e-9, "third tick at {}", seen[2]);
    }

    /// A satellite added mid-run takes its first tick one period after it
    /// enters, not one period after `t = 0`.
    ///
    /// Pins the phase rule, not the wiring: the fixture below anchors the
    /// schedule the same way `build_controlled_satellite` does, so a caller
    /// passing the wrong `start_t` would still pass here. Reaching the real
    /// builder needs a WASM plugin to construct the controller from.
    #[test]
    fn a_satellite_starting_late_phases_its_ticks_from_its_start() {
        let (sat, _) = satellite_with(0.1, 5.0);
        assert!(
            (sat.next_tick_t() - 5.1).abs() < 1e-9,
            "first tick at {}, expected 5.1",
            sat.next_tick_t()
        );
    }

    /// A span that stops just short of the next tick does not run it.
    ///
    /// The end of a run, or a stream flush that lands a hair before a tick,
    /// must leave that tick for the next span. Comparing with a tolerance —
    /// which a drifting `next_tick_t += period` needs — would fire it here and
    /// integrate past the boundary the caller asked for.
    #[test]
    fn a_span_ending_just_before_a_tick_does_not_run_it() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);

        advance(&mut sat, 0.0, 0.1 - 1e-10, 0.01);
        assert!(
            ticks.lock().unwrap().is_empty(),
            "a tick 1e-10 s past the span's end was run: {:?}",
            ticks.lock().unwrap()
        );

        // And it is still there for the span that does contain it.
        advance(&mut sat, 0.1 - 1e-10, 0.15, 0.01);
        let seen = ticks.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "the deferred tick should run next: {seen:?}");
        assert!((seen[0] - 0.1).abs() < 1e-12, "at {}", seen[0]);
    }

    /// Repeated ticks do not drift off the period.
    ///
    /// `tick_base_t + n · period` is one multiply; summing `period` a thousand
    /// times is not, and the accumulated error is what a boundary tolerance
    /// would have had to cover.
    #[test]
    fn the_thousandth_tick_is_still_on_the_period() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);
        advance(&mut sat, 0.0, 100.0, 1.0);

        let seen = ticks.lock().unwrap().clone();
        assert_eq!(seen.len(), 1000, "100 s at 10 Hz is 1000 ticks");
        assert_eq!(
            seen[999], 100.0,
            "the last tick should be exactly 100.0, got {}",
            seen[999]
        );
    }

    /// A period the sim clock cannot resolve is refused, not spun on.
    ///
    /// `validate_sample_period` accepts any positive finite period, and
    /// `5.0 + 1e-16 == 5.0`: a schedule anchored there would never advance and
    /// every loop waiting on the next tick would run forever.
    #[test]
    fn a_period_below_the_clock_resolution_is_rejected() {
        validate_tick_advances(5.0, 1e-16)
            .expect_err("a period that cannot move the clock must be refused");
        // Separates the first tick from the start, but not the ticks from each
        // other: `5.0 + 1*5e-16` and `5.0 + 2*5e-16` both round to
        // 5.000000000000001, so accepting it would tick twice at one instant.
        assert_ne!(5.0 + 5e-16, 5.0, "precondition: the first tick does move");
        assert_eq!(
            5.0 + 5e-16,
            5.0 + 2.0 * 5e-16,
            "precondition: the first two ticks land on the same f64"
        );
        validate_tick_advances(5.0, 5e-16)
            .expect_err("a period that cannot separate two ticks must be refused");
        // 0.75 ULP at 5.0: the first two ticks advance, and the third rounds
        // back onto the second. Sampling a fixed number of ticks would accept
        // this; requiring one ULP refuses it.
        let three_eps = 3.0 * f64::EPSILON;
        assert_ne!(5.0 + three_eps, 5.0, "precondition: tick 1 moves");
        assert_ne!(
            5.0 + 2.0 * three_eps,
            5.0 + three_eps,
            "precondition: tick 2 moves"
        );
        assert_eq!(
            5.0 + 3.0 * three_eps,
            5.0 + 2.0 * three_eps,
            "precondition: tick 3 lands back on tick 2"
        );
        validate_tick_advances(5.0, three_eps).expect_err("a period below one ULP must be refused");
        // One ULP exactly is the smallest period that keeps the ticks apart.
        validate_tick_advances(5.0, 5.0f64.next_up() - 5.0)
            .expect("one ULP separates every consecutive pair");
        // Just below a binade boundary the ULP doubles on the way up, so a
        // period equal to the anchor's own ULP is half of one above it.
        let below_eight = 8.0f64.next_down();
        let one_ulp_there = 8.0 - below_eight;
        assert_eq!(
            below_eight + one_ulp_there,
            below_eight + 2.0 * one_ulp_there,
            "precondition: ticks 1 and 2 both land on 8.0"
        );
        let err = validate_tick_advances(below_eight, one_ulp_there)
            .expect_err("a period that collides across the binade must be refused");
        // The message has to name the bound that failed: here the anchor's own
        // resolution is met and the first tick's is not.
        assert!(
            err.contains(&format!("{}", 8.0f64.next_up() - 8.0)),
            "the message should report the first tick's resolution: {err}"
        );
        // The same period is fine from zero, where it is representable.
        validate_tick_advances(0.0, 1e-16).expect("representable at t=0");
        validate_tick_advances(5.0, 0.1).expect("an ordinary period is fine");
    }

    /// The configured integrator is the one that runs the controlled loop.
    ///
    /// Each arm propagates one orbit of the same satellite with a requested step
    /// of a whole period. For `Rk4` that is the step, and a single step cannot
    /// follow the orbit; for the adaptive pair it is only the first guess, which
    /// the error control then subdivides. The reference is the same span under
    /// `Rk4` at 0.5 s, a step 11,000 times finer.
    ///
    /// The propagation runs through `advance_controlled`, the function `run` and
    /// `serve` both call, so what this measures is the whole path from
    /// `SimParams` to the integrator — not `propagate_controlled` alone.
    /// `propagate_controlled` used to take a bare `dt` and hardcode `Rk4`, and
    /// the selection was the caller's to make: `[integrator]` reached the
    /// orbit-only and spacecraft groups while the controlled callers passed
    /// `params.dt` on its own. Every arm then returned the coarse `Rk4` answer.
    #[test]
    fn the_configured_integrator_reaches_the_controlled_loop() {
        use crate::cli::IntegratorChoice;

        let period = 2.0
            * std::f64::consts::PI
            * (6778.0f64.powi(3) / arika::body::KnownBody::Earth.properties().mu).sqrt();
        let after_one_orbit = |choice: IntegratorChoice, dt: f64| {
            // A controller period past the span, so no tick interrupts it and
            // the integrator is the only thing under test.
            let (mut sat, _) = satellite_with(period * 2.0, 0.0);
            let params = params_with(choice, dt, 1e-10);
            advance_controlled(&mut sat, 0.0, period, &params).expect("integrates");
            *sat.state.plant.orbit.position()
        };

        let reference = after_one_orbit(IntegratorChoice::Rk4, 0.5);
        let coarse_rk4 = (after_one_orbit(IntegratorChoice::Rk4, period) - reference).norm();
        let dop853 = (after_one_orbit(IntegratorChoice::Dop853, period) - reference).norm();
        let dp45 = (after_one_orbit(IntegratorChoice::Dp45, period) - reference).norm();

        // Measured: one RK4 step over the period lands 5.95e4 km from the
        // reference, while DOP853 lands 2.8e-9 km from it and DP45 4.7e-6 km.
        // The bound is 1 km, which both sides clear by four orders or more.
        assert!(
            coarse_rk4 > 1.0,
            "one RK4 step over a whole period cannot follow the orbit, \
             got {coarse_rk4:.3e} km from the reference"
        );
        assert!(
            dop853 < 1.0,
            "DOP853 subdivides the span, got {dop853:.3e} km from the reference"
        );
        assert!(
            dp45 < 1.0,
            "DP45 subdivides the span, got {dp45:.3e} km from the reference"
        );
        // The two adaptive arms are told apart, so serving one where the config
        // asked for the other fails here. At the same tolerance the 8th-order
        // method holds a tighter error than the 5th: measured, 2.8e-9 km
        // against 4.7e-6 km, a factor of 1700. The bound is 10.
        assert!(
            dop853 * 10.0 < dp45,
            "DOP853 should hold a tighter error than DP45 at the same tolerance: \
             {dop853:.3e} km against {dp45:.3e} km"
        );
    }

    /// Two controllers at different rates each run at their own.
    ///
    /// `orts run` used to drive the fleet on the shortest period, so the 1.0 s
    /// controller here was called every 0.1 s — the very case the streams path
    /// rejects outright rather than mis-simulate.
    #[test]
    fn a_mixed_rate_fleet_ticks_each_controller_at_its_own_period() {
        let (fast, fast_ticks) = satellite_with(0.1, 0.0);
        let (slow, slow_ticks) = satellite_with(1.0, 0.0);

        // The event times come from the same function the run loop calls, so
        // an error in that choice fails this test.
        let mut fleet = vec![fast, slow];
        let mut t = 0.0;
        while t < 1.0 - 1e-12 {
            let next_t = next_fleet_event_t(&fleet, 1.0);
            for sat in &mut fleet {
                propagate_controlled(
                    sat,
                    t,
                    next_t,
                    &IntegratorConfig::Rk4 { dt: 0.01 },
                    RootSearch::default(),
                    &never_ends,
                )
                .expect("integrates");
                if sat.tick_due_at(next_t) {
                    tick_controller(sat, next_t, None).expect("ticks");
                }
            }
            t = next_t;
        }

        assert_eq!(
            fast_ticks.lock().unwrap().len(),
            10,
            "the 0.1 s controller should tick 10 times in 1 s"
        );
        let slow_seen = slow_ticks.lock().unwrap().clone();
        assert_eq!(
            slow_seen.len(),
            1,
            "the 1.0 s controller should tick once in 1 s, got {slow_seen:?}"
        );
        assert!((slow_seen[0] - 1.0).abs() < 1e-9, "at {}", slow_seen[0]);
    }

    /// A satellite at `position`, its body axes aligned with the inertial ones.
    fn state_at(position: nalgebra::Vector3<f64>) -> orts::spacecraft::SpacecraftState {
        SpacecraftState {
            orbit: orts::orbital::OrbitalState::new(
                position,
                nalgebra::Vector3::new(0.0, 3.0, 0.0),
            ),
            attitude: orts::attitude::AttitudeState {
                quaternion: nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: nalgebra::Vector3::zeros(),
            },
            mass: 100.0,
        }
    }

    fn sun_direction_read_by(
        bundle: &mut SensorBundle,
        state: &orts::spacecraft::SpacecraftState,
        epoch: &Epoch,
    ) -> Option<nalgebra::Vector3<f64>> {
        match bundle.sun_sensors[0].measure(state, epoch) {
            orts::plugin::SunSensorOutput::Fine { direction, .. } => {
                direction.map(|d| d.into_inner().into_inner())
            }
            other => panic!("the CLI builds a fine sensor, got {other:?}"),
        }
    }

    fn angle_deg(a: &nalgebra::Vector3<f64>, b: &nalgebra::Vector3<f64>) -> f64 {
        a.normalize()
            .dot(&b.normalize())
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    /// The CLI's sun sensor reads the Sun from the central body.
    ///
    /// `SunSensor::for_body`'s own tests pass whichever way this line is
    /// wired, so a regression to `SunSensor::new()` — Earth's Sun, no shadow —
    /// would go unnoticed. Both halves are measured on 2026-03-20, when Mars'
    /// Sun direction is 152.8° from Earth's.
    #[test]
    fn the_cli_sun_sensor_reads_the_sun_from_the_central_body() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let mars_sun = arika::sun::sun_position_from_body(KnownBody::Mars, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner();
        let earth_sun = arika::sun::sun_position_eci(&epoch.to_tdb()).into_inner();

        // 20 000 km sunward of Mars, so nothing eclipses the satellite and the
        // direction to Mars' Sun is parallel to `mars_sun`.
        let sunward_of_mars = state_at(mars_sun.normalize() * 20_000.0);
        let mut on_mars = build_sensor_bundle(
            Some(&[SensorChoice::SunSensor]),
            KnownBody::Mars,
            "sat-test",
        )
        .expect("Mars has a Sun ephemeris");
        let read = sun_direction_read_by(&mut on_mars, &sunward_of_mars, &epoch)
            .expect("sunward of Mars, so lit");
        assert!(
            angle_deg(&read, &mars_sun) < 1.0e-4,
            "the reading follows Mars' Sun: {:.6}° away",
            angle_deg(&read, &mars_sun)
        );
        // What the geocentric wiring would have read instead.
        assert!(
            angle_deg(&read, &earth_sun) > 150.0,
            "Earth's Sun is nowhere near it: {:.3}°",
            angle_deg(&read, &earth_sun)
        );

        // On Earth the same wiring carries Earth's conical shadow, so a
        // satellite directly behind the Earth reads no direction at all.
        let behind_earth = state_at(-earth_sun.normalize() * 7000.0);
        let mut on_earth = build_sensor_bundle(
            Some(&[SensorChoice::SunSensor]),
            KnownBody::Earth,
            "sat-test",
        )
        .expect("Earth has a Sun ephemeris");
        assert!(
            sun_direction_read_by(&mut on_earth, &behind_earth, &epoch).is_none(),
            "eclipsed, so the sensor reports no Sun direction"
        );
    }

    /// A magnetometer on a body with no field model reads zero.
    ///
    /// `Igrf` and `TiltedDipole` are Earth's, and they are the only field
    /// models there are. The device is still built — the same spacecraft
    /// definition can be pointed at any body — and reads zero there instead of
    /// reporting a field the body does not have.
    #[test]
    fn a_magnetometer_reads_zero_where_no_field_is_modelled() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let state = state_at(nalgebra::Vector3::new(7000.0, 0.0, 0.0));

        for body in [KnownBody::Mars, KnownBody::Moon, KnownBody::Sun] {
            let mut bundle =
                build_sensor_bundle(Some(&[SensorChoice::Magnetometer]), body, "sat-test")
                    .unwrap_or_else(|e| panic!("{body:?} builds a magnetometer: {e}"));
            let reading = bundle.magnetometers[0]
                .measure(&state, &epoch)
                .into_inner()
                .into_inner();
            assert_eq!(
                reading,
                nalgebra::Vector3::zeros(),
                "{body:?} has no field model, so the reading is zero"
            );
        }

        let mut on_earth = build_sensor_bundle(
            Some(&[SensorChoice::Magnetometer]),
            KnownBody::Earth,
            "sat-test",
        )
        .expect("Earth's field is modelled");
        let earth_reading = on_earth.magnetometers[0]
            .measure(&state, &epoch)
            .into_inner()
            .into_inner();
        assert!(
            earth_reading.norm() > 0.0,
            "Earth's field is modelled, so the reading is not zero: {earth_reading:?}"
        );

        // A sensor that needs no field is unaffected whatever the body is.
        assert!(
            build_sensor_bundle(
                Some(&[SensorChoice::Gyroscope]),
                KnownBody::Mars,
                "sat-test"
            )
            .is_ok(),
            "a gyroscope needs no field"
        );
    }

    /// The field a magnetorquer gets is decided by its body, on both paths.
    ///
    /// `mtq_for_body` is what the initial build and the per-command rebuild
    /// both call, so this ties the body to the installed model rather than to a
    /// flag the test set itself. Measured through the torque the assembly
    /// reports for the same command: zero on Mars, non-zero on Earth.
    #[test]
    fn a_magnetorquer_takes_the_field_of_its_body_on_both_paths() {
        let command = MtqCommand::NormalizedMoments(vec![1.0, 0.0, 0.0]);

        // The state and dynamics are the same for both bodies; only the field
        // model differs, so the torque is what the choice decides.
        let torque_with = |model: Box<dyn orts::model::Model<SpacecraftState>>| -> f64 {
            let (mut sat, _ticks) = satellite_with(1.0, 0.0);
            sat.dynamics = sat
                .dynamics
                .with_epoch(Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0))
                .with_model(model);
            sat.dynamics
                .model_breakdown(0.0, &sat.state)
                .into_iter()
                .find(|(name, _)| *name == "mtq_assembly")
                .expect("the assembly is installed")
                .1
                .torque_body
                .inner()
                .norm()
        };

        assert_eq!(
            torque_with(mtq_for_body(KnownBody::Mars, 10.0, Some(&command))),
            0.0,
            "Mars has no field model, so a commanded magnetorquer makes no torque"
        );
        assert!(
            torque_with(mtq_for_body(KnownBody::Earth, 10.0, Some(&command))) > 0.0,
            "Earth's field is modelled, so the same command makes torque"
        );
    }

    /// The rebuild does not warn again.
    ///
    /// `apply_held_commands` rebuilds the assembly on every tick that carries a
    /// command, and a held command persists, so a warning inside that path
    /// would repeat for the whole run. Measured through the counter beside
    /// `warn_no_field_model`: ten rebuilds add nothing to it.
    #[test]
    fn rebuilding_a_commanded_magnetorquer_does_not_warn_again() {
        let (mut sat, _ticks) = satellite_with(1.0, 0.0);
        sat.body = KnownBody::Mars;
        sat.has_mtq = true;
        sat.mtq_max_moment = 10.0;
        sat.dynamics = sat
            .dynamics
            .with_epoch(Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0))
            .with_model(mtq_for_body(sat.body, 10.0, None));
        sat.actuators
            .apply(&Command::mtq_normalized(vec![1.0, 0.0, 0.0]))
            .expect("three moments for three MTQs");

        let before = FIELD_WARNINGS.with(|n| n.get());
        for _ in 0..10 {
            apply_held_commands(&mut sat).expect("the command length matches the MTQ count");
        }
        assert_eq!(
            FIELD_WARNINGS.with(|n| n.get()),
            before,
            "the rebuild is silent; the warning belongs where the device is built"
        );
    }

    /// The rebuild after a command goes through the same factory.
    ///
    /// Measured: a satellite whose body has no field model keeps zero torque
    /// after `apply_held_commands` installs the command. Pointing the rebuild
    /// at Earth's field fails this.
    #[test]
    fn a_commanded_magnetorquer_keeps_the_field_of_its_body() {
        let (mut sat, _ticks) = satellite_with(1.0, 0.0);
        sat.body = KnownBody::Mars;
        sat.has_mtq = true;
        sat.mtq_max_moment = 10.0;
        sat.dynamics = sat
            .dynamics
            .with_epoch(Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0))
            .with_model(mtq_for_body(sat.body, 10.0, None));

        sat.actuators
            .apply(&Command::mtq_normalized(vec![1.0, 0.0, 0.0]))
            .expect("three moments for three MTQs");
        apply_held_commands(&mut sat).expect("the command length matches the MTQ count");

        let torque = sat
            .dynamics
            .model_breakdown(0.0, &sat.state)
            .into_iter()
            .find(|(name, _)| *name == "mtq_assembly")
            .expect("the assembly is installed")
            .1
            .torque_body
            .inner()
            .norm();
        assert_eq!(
            torque, 0.0,
            "Mars has no field model, so the rebuilt assembly makes no torque"
        );
    }

    /// A magnetorquer on a body with no field model does not block the build.
    ///
    /// The assembly is built with `NoField` there, so its torque is `m × 0`.
    /// Built from a config, the way a user reaches it: the controller points at
    /// a path that does not exist and actuators are built first, so reaching
    /// the plugin error is what says the magnetorquer let the build through.
    #[test]
    fn a_magnetorquer_is_inert_where_no_field_is_modelled() {
        let config_for = |body: &str| -> crate::config::SimConfig {
            toml::from_str(&format!(
                r#"
dt = 1.0
body = "{body}"

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 400.0

[satellites.attitude]
inertia_diag = [10.0, 10.0, 10.0]
mass = 100.0

[satellites.magnetorquers]
type = "three_axis"
max_moment = 0.2

[satellites.controller]
type = "wasm"
path = "does-not-exist.wasm"
"#
            ))
            .expect("the config parses")
        };

        let build = |body: &str| {
            let config = config_for(body);
            let params = SimParams::from_config(&config).expect("valid test config");
            let spec = params.satellites[0].clone();
            #[cfg(feature = "plugin-wasm")]
            let mut cache =
                orts::plugin::wasm::WasmPluginCache::new().expect("a cache needs no plugin file");
            let mut ctx = ControlledBuildContext {
                params: &params,
                #[cfg(feature = "plugin-wasm")]
                wasm_cache: &mut cache,
                #[cfg(feature = "plugin-wasm")]
                plugin_backend: params.resolve_plugin_backend(),
            };
            build_controlled_satellite(&spec, None, 0.0, &mut ctx).map(|_| ())
        };

        // Every body reaches the plugin path, so no body is stopped by its
        // magnetorquer.
        for body in ["mars", "moon", "earth"] {
            let err = build(body).expect_err("the plugin path does not exist");
            assert!(
                !err.contains("magnetorquer"),
                "{body}: the magnetorquer should not stop the build: {err}"
            );
        }
    }

    /// The controlled loop stops a burn at the propellant floor, and the record
    /// says so.
    ///
    /// The wiring is the run's own: `install_thrusters` is what
    /// `build_controlled_satellite` calls, and this case calls it too, so a
    /// change there cannot leave the case green. (The builder itself needs a
    /// plugin guest, which a unit test cannot load — hence the seam.) The burn
    /// case above installs its thruster with `with_model` and has no pool at
    /// all, so nothing held this.
    ///
    /// 0.1 kg of propellant at 10 N and Isp 300 s lasts 29.4 s. The walk is
    /// given 60 s in 10 s steps, so the crossing falls inside a step.
    #[test]
    fn the_controlled_loop_stops_a_burn_at_the_propellant_floor() {
        use orts::spacecraft::G0;

        const THRUST_N: f64 = 10.0;
        const ISP_S: f64 = 300.0;
        const DRY_MASS: f64 = 500.0;
        const PROPELLANT: f64 = 0.1;

        let (mut sat, _) = satellite_with(1.0, 0.0);
        let bare = std::mem::replace(
            &mut sat.dynamics,
            orts::spacecraft::SpacecraftDynamics::new(
                arika::earth::MU,
                Box::new(orts::orbital::gravity::PointMass),
                nalgebra::Matrix3::identity(),
            ),
        );
        let specs = vec![ThrusterSpec::new(THRUST_N, ISP_S, Vector3::x())];
        sat.dynamics = install_thrusters(bare, specs.clone(), DRY_MASS);
        // An assembly fires what it is commanded to, and the loop takes that
        // from the actuators, replacing the model on every tick — so this is
        // also what holds `replace_model` putting the new assembly back in the
        // propulsion set rather than among the ordinary models.
        sat.thruster_specs = specs;
        sat.actuators
            .apply(&Command::thruster(vec![1.0]))
            .expect("one throttle for one thruster");
        // What a tick does with a held command; `propagate_controlled` on its
        // own only walks the span.
        apply_held_commands(&mut sat).expect("one throttle for one thruster");
        // The fixture's state was built before the pool was registered, so it
        // carries no mode for it.
        sat.state = sat
            .dynamics
            .initial_augmented_state(orts::spacecraft::SpacecraftState {
                mass: DRY_MASS + PROPELLANT,
                ..sat.state.plant.clone()
            });

        propagate_controlled(
            &mut sat,
            0.0,
            60.0,
            &IntegratorConfig::Rk4 { dt: 10.0 },
            RootSearch::default(),
            &never_ends,
        )
        .expect("the burn and the orbit are finite everywhere");

        assert!(
            (sat.state.plant.mass - DRY_MASS).abs() < 1e-6,
            "the mass ends on the floor, not at {}",
            sat.state.plant.mass
        );
        // 29.4 s of burning is what 0.1 kg at this thrust buys, so the floor is
        // crossed inside the third step rather than on its edge.
        let burn_time = PROPELLANT / (THRUST_N / (ISP_S * G0));
        assert!(
            (29.0..30.0).contains(&burn_time),
            "the burn lasts {burn_time} s, inside the 20..30 s step"
        );

        // And nothing burns after that: another minute leaves the mass where it
        // is. The ΔV against Tsiolkovsky is `propellant_floor.rs`'s case, in
        // free space — here the orbit turns under the thrust, so a velocity
        // difference is mostly the orbit.
        let on_the_floor = sat.state.plant.mass;
        propagate_controlled(
            &mut sat,
            60.0,
            120.0,
            &IntegratorConfig::Rk4 { dt: 10.0 },
            RootSearch::default(),
            &never_ends,
        )
        .expect("the walk succeeds");
        assert_eq!(
            sat.state.plant.mass, on_the_floor,
            "an empty tank spends nothing"
        );

        // The record keeps a column for the thruster and reports no thrust in
        // it, which is what a reader watching the burn end sees.
        let breakdown = sat.dynamics.model_breakdown(60.0, &sat.state);
        let (_, thruster) = breakdown
            .iter()
            .find(|(name, _)| *name == "thruster_assembly")
            .expect("the assembly keeps its entry after depletion");
        assert_eq!(thruster.mass_rate, 0.0);
        assert_eq!(
            thruster.acceleration_inertial,
            arika::frame::Vec3::zeros(),
            "no thrust once the tank is empty"
        );
    }

    /// The controlled path refuses a state below the floor rather than walking
    /// it, and says which constraint refused it.
    ///
    /// `serve` can replace a satellite's state while a run is going, so the
    /// check belongs where a boundary walk starts rather than only where a
    /// satellite is added. Measured before the check existed: the walk's own
    /// reconciliation settled the boundary and the mass rose to the dry mass,
    /// which is propellant the caller never gave it.
    #[test]
    fn the_controlled_loop_refuses_a_state_below_the_propellant_floor() {
        const THRUST_N: f64 = 10.0;
        const ISP_S: f64 = 300.0;
        const DRY_MASS: f64 = 500.0;
        const MISSING_KG: f64 = 0.5;

        let (mut sat, _) = satellite_with(1.0, 0.0);
        let bare = std::mem::replace(
            &mut sat.dynamics,
            orts::spacecraft::SpacecraftDynamics::new(
                arika::earth::MU,
                Box::new(orts::orbital::gravity::PointMass),
                nalgebra::Matrix3::identity(),
            ),
        );
        let specs = vec![ThrusterSpec::new(THRUST_N, ISP_S, Vector3::x())];
        sat.dynamics = install_thrusters(bare, specs.clone(), DRY_MASS);
        sat.thruster_specs = specs;
        // Built by hand, the way a restored or externally written state is: the
        // constructor would have refused this mass.
        sat.state = orts::effector::AugmentedState {
            plant: orts::spacecraft::SpacecraftState {
                mass: DRY_MASS - MISSING_KG,
                ..sat.state.plant.clone()
            },
            aux: sat.state.aux.clone(),
            aux_bounds: sat.state.aux_bounds.clone(),
            modes: vec![orts::effector::ConstraintMode::Free],
        };

        let err = propagate_controlled(
            &mut sat,
            0.0,
            60.0,
            &IntegratorConfig::Rk4 { dt: 10.0 },
            RootSearch::default(),
            &never_ends,
        )
        .expect_err("the state cannot start a boundary walk");

        assert!(
            err.contains("propellant_pool") && err.contains("below the dry mass"),
            "the error names the constraint that refused the state, not {err}"
        );
        assert_eq!(
            sat.state.plant.mass,
            DRY_MASS - MISSING_KG,
            "and the mass is left as it was: settling it would add {MISSING_KG} kg"
        );
    }

    /// A burn shorter than an integration step is flown by the controlled loop.
    ///
    /// `propagate_controlled` used to run the integrator from `t0` straight to
    /// `t1`, so a `ScheduledBurn` narrower than the largest gap between
    /// adjacent stage times fell between them and spent nothing. The oracle is
    /// the analytic propellant, `thrust / (Isp * g0)` times the burn's length,
    /// which the trajectory cannot change: the mass flow of a thruster at full
    /// throttle does not depend on the state.
    ///
    /// The window sits at `[0.15, 0.25)` so the segment before it is longer
    /// than the burn. With both 0.1 s long, the `h/6` a stage-time reading
    /// leaks into the earlier segment and the `h/6` it drops from the burn's
    /// last stage cancel exactly under RK4.
    ///
    /// The second case gives the burn's own segment several RK4 steps — a
    /// 0.4 s window at `dt = 0.1` — so the propellant also pins the multi-step
    /// path, where `try_integrate` accumulates its clock and covers the
    /// residual with one more step.
    #[test]
    fn a_burn_shorter_than_a_step_is_flown_by_the_controlled_loop() {
        use orts::spacecraft::G0;

        const THRUST_N: f64 = 10.0;
        const ISP_S: f64 = 300.0;

        for (window, dt) in
            [(0.15, 0.25, 1.0), (0.15, 0.55, 0.1)].map(|(start, end, dt)| ((start, end), dt))
        {
            let expected = THRUST_N / (ISP_S * G0) * (window.1 - window.0);
            run_burn_case(window, dt, expected);
        }
    }

    /// Fly one burn window with each integrator and check the propellant.
    fn run_burn_case(window: (f64, f64), dt: f64, expected: f64) {
        use orts::spacecraft::{BurnWindow, ScheduledBurn, Thruster};
        use utsuroi::Tolerances;

        const THRUST_N: f64 = 10.0;
        const ISP_S: f64 = 300.0;

        for (name, integrator) in [
            ("RK4", IntegratorConfig::Rk4 { dt }),
            (
                "DP45",
                IntegratorConfig::Dp45 {
                    dt,
                    tolerances: Tolerances::default(),
                },
            ),
            (
                "DOP853",
                IntegratorConfig::Dop853 {
                    dt,
                    tolerances: Tolerances::default(),
                },
            ),
        ] {
            let (mut sat, _) = satellite_with(1.0, 0.0);
            sat.dynamics = std::mem::replace(
                &mut sat.dynamics,
                orts::spacecraft::SpacecraftDynamics::new(
                    arika::earth::MU,
                    Box::new(orts::orbital::gravity::PointMass),
                    nalgebra::Matrix3::identity(),
                ),
            )
            .with_model(
                Thruster::new(THRUST_N, ISP_S, Vector3::x()).with_profile(Box::new(
                    ScheduledBurn {
                        windows: vec![BurnWindow::full(window.0, window.1)],
                    },
                )),
            );
            let mass_before = sat.state.plant.mass;

            propagate_controlled(
                &mut sat,
                0.0,
                10.0,
                &integrator,
                RootSearch::default(),
                &never_ends,
            )
            .expect("the burn and the orbit are finite everywhere");

            let spent = mass_before - sat.state.plant.mass;
            // The propellant is the difference of two masses near 500 kg, so it
            // carries the resolution of f64 there however exactly the mass flow
            // was integrated.
            let tol = 4.0 * mass_before * f64::EPSILON;
            assert!(
                (spent - expected).abs() < tol,
                "{name} at dt={dt} spent {spent} kg over [{}, {}), expected {expected} kg",
                window.0,
                window.1
            );
        }
    }

    /// A span the loop cannot walk is rejected rather than reported as done.
    ///
    /// The segment loop tests `t < t1` before it builds a stepper, so a
    /// non-finite bound takes no step and the solvers — which validate the span
    /// themselves — never see it.
    #[test]
    fn a_non_finite_span_is_rejected_by_the_controlled_loop() {
        for bound in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let (mut sat, _) = satellite_with(1.0, 0.0);
            assert!(
                propagate_controlled(
                    &mut sat,
                    0.0,
                    bound,
                    &IntegratorConfig::Rk4 { dt: 1.0 },
                    RootSearch::default(),
                    &never_ends
                )
                .is_err(),
                "a target of {bound} was accepted"
            );
            let (mut sat, _) = satellite_with(1.0, 0.0);
            assert!(
                propagate_controlled(
                    &mut sat,
                    bound,
                    10.0,
                    &IntegratorConfig::Rk4 { dt: 1.0 },
                    RootSearch::default(),
                    &never_ends
                )
                .is_err(),
                "a start of {bound} was accepted"
            );
        }
    }

    /// A span that fails partway leaves the satellite where it started.
    ///
    /// A `ControlledSatellite` carries no integration time of its own, so a
    /// state committed at a segment boundary belongs to an instant the caller
    /// has no record of: serve pauses on the error and can resume, propagating
    /// a state from the wrong time. The state has to move only when the whole
    /// span succeeds.
    #[test]
    fn a_span_that_fails_partway_leaves_the_state_untouched() {
        use arika::epoch::Epoch;
        use orts::model::{ExternalLoads, Model};
        use orts::spacecraft::SpacecraftState;

        /// Finite up to and including `breaks_at`, not after it — with the
        /// boundary declared, so the loop splits the span there and it is the
        /// *second* segment that fails. The stage on the first segment's end
        /// has to stay finite: this model keeps the default stage-time
        /// evaluation, so that stage reads `breaks_at` itself.
        struct BreaksAfter {
            breaks_at: f64,
        }

        impl Model<SpacecraftState> for BreaksAfter {
            fn name(&self) -> &str {
                "breaks_after"
            }

            fn eval(
                &self,
                t: f64,
                _state: &SpacecraftState,
                _epoch: Option<&Epoch>,
            ) -> ExternalLoads {
                let mut loads = ExternalLoads::zeros();
                if t > self.breaks_at {
                    loads.acceleration_inertial =
                        arika::frame::Vec3::from_raw(Vector3::new(f64::NAN, 0.0, 0.0));
                }
                loads
            }

            fn next_discontinuity_after(&self, t: f64, _epoch: Option<&Epoch>) -> Option<f64> {
                (self.breaks_at > t).then_some(self.breaks_at)
            }
        }

        let (mut sat, _) = satellite_with(1.0, 0.0);
        sat.dynamics = std::mem::replace(
            &mut sat.dynamics,
            orts::spacecraft::SpacecraftDynamics::new(
                arika::earth::MU,
                Box::new(orts::orbital::gravity::PointMass),
                nalgebra::Matrix3::identity(),
            ),
        )
        .with_model(BreaksAfter { breaks_at: 0.2 });
        let before = sat.state.clone();

        let err = propagate_controlled(
            &mut sat,
            0.0,
            1.0,
            &IntegratorConfig::Rk4 { dt: 1.0 },
            RootSearch::default(),
            &never_ends,
        )
        .expect_err("the second segment is not finite");
        assert!(err.contains("non-finite state"), "unexpected error: {err}");
        assert_eq!(
            sat.state.plant.orbit.position(),
            before.plant.orbit.position(),
            "the failed span moved the satellite"
        );
        assert_eq!(
            sat.state.plant.mass, before.plant.mass,
            "the failed span changed the mass"
        );
    }
}
