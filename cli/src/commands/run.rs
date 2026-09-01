use std::ops::ControlFlow;

use arika::body::KnownBody;
use arika::frame::{SimpleEci, Vec3};
use orts::OrbitalState;
use orts::group::{HasPosition, IndependentGroup, IntegratorConfig};
use orts::orbital::kepler::KeplerianElements;
use orts::record::archetypes::OrbitalState as RecordOrbitalState;
use orts::record::components::{
    AngularVelocity3D, BodyRadius, GravitationalParameter, MtqCommand3D, Quaternion4D,
    RwMomentum3D, RwTorqueCommand3D, ThrusterThrottle3D,
};
use orts::record::entity_path::EntityPath;
use orts::record::recording::Recording;
use orts::record::timeline::TimePoint;
use orts::visibility::{StationContact, VisibilityMonitor};

use crate::cli::{IntegratorChoice, OutputFormat, SimArgs};
use crate::commands::CmdError;
use crate::satellite::OrbitSpec;
use crate::sim::mode::{
    SimMode, ensure_commands_deliverable, ensure_streams_unused, select_sim_mode,
    unhonored_config_warnings,
};
use crate::sim::params::SimParams;
use utsuroi::DynamicalSystem;

/// Apply the config-file validation rules to the direct-CLI argument path.
///
/// `SimParams::from_sim_args` builds the same `SimParams` as
/// `SimParams::from_config` but skips `SimConfig::validate`, so without this
/// `--dt 0` hangs, `--dt nan` panics in step-size control, and
/// `--dt 10 --output-interval 1` panics inside `clamp`.
pub(crate) fn validate_sim_args(sim: &SimArgs) -> Result<(), String> {
    crate::config::validate_time_params(
        sim.dt,
        sim.output_interval,
        sim.stream_interval,
        sim.duration,
    )?;
    crate::config::validate_tolerances(sim.integrator, sim.atol, sim.rtol)
}

pub fn run_simulation_cmd(
    sim: &SimArgs,
    output: Option<&str>,
    format: OutputFormat,
    json: bool,
) -> Result<(), CmdError> {
    let mut params = if let Some(config_path) = &sim.config {
        let config = crate::config::SimConfig::load(std::path::Path::new(config_path))?;
        SimParams::from_config(&config)
    } else if sim.has_orbit_args() {
        // The direct-CLI path bypasses `SimConfig::validate`, so apply the
        // same time/tolerance checks here rather than letting a bad `--dt`
        // hang the propagation loop or panic inside step-size control.
        validate_sim_args(sim)?;
        SimParams::from_sim_args(sim, false)
    } else {
        // Auto-detect orts.toml in the current directory
        let config_path = std::path::Path::new("orts.toml");
        if config_path.exists() {
            let config = crate::config::SimConfig::load(config_path)?;
            SimParams::from_config(&config)
        } else {
            return Err(CmdError::usage(
                "no simulation configuration found.\n\
                 Provide --config <path>, place orts.toml in the current directory,\n\
                 or specify an orbit with --sat / --tle / --norad-id.",
            ));
        }
    };
    // Resolve and validate the stdout/output contract before running the
    // (potentially long) simulation, so usage errors fail fast.
    let sink = resolve_data_sink(output, format);
    validate_output_contract(&sink, format, json)?;

    // CLI backend flags always override config-file defaults so
    // `orts run --config … --plugin-backend=sync|async` works.
    params.plugin_backend_choice = sim.plugin_backend;
    params.plugin_backend_threshold = sim.plugin_backend_threshold;
    params.plugin_backend_async_mode = sim.plugin_backend_async_mode;

    // どのダイナミクスで回すかは serve と共有の規則で決める。`run` だけが
    // orbit-only に落ちて姿勢・アクチュエータ設定を黙って捨てることがないように。
    let mode = select_sim_mode(&params.satellites).map_err(CmdError::usage)?;
    // 時刻指定コマンドは制御ループがなければ届かず、stream-io ストリームは
    // `orts run` にそもそも pump する transport がない。
    ensure_commands_deliverable(mode, params.commands.len()).map_err(CmdError::usage)?;
    ensure_streams_unused(&params.satellites).map_err(CmdError::usage)?;
    // 選択したモードで効かない設定は、無視する前に知らせる。
    let warnings = unhonored_config_warnings(&params.satellites, mode);
    for w in &warnings {
        eprintln!("Warning: {w}");
    }

    // Before the dispatch, so every mode gets the same check: an attitude
    // config that cannot be integrated is rejected here rather than failing
    // partway through the run (or in the controlled path, inside
    // `build_controlled_satellite`).
    for sat in &params.satellites {
        crate::sim::mode::validate_satellite_spec(sat).map_err(CmdError::usage)?;
    }

    let rec = match mode {
        SimMode::Controlled => run_controlled_simulation(&params, sim)?,
        SimMode::Spacecraft => run_spacecraft_simulation(&params)?,
        SimMode::OrbitOnly => run_simulation(&params)?,
    };

    let artifact = write_simulation_output(&rec, &params, &sink, format)?;

    if json {
        let summary = build_run_summary(&params, &rec, artifact, warnings);
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        serde_json::to_writer_pretty(&mut stdout, &summary)
            .map_err(|e| CmdError::failure(format!("serializing run summary: {e}")))?;
        // Trailing newline so the JSON document is its own line on stdout.
        // Written through the same locked writer so a failure is reported
        // instead of being swallowed by `println!`.
        stdout
            .write_all(b"\n")
            .map_err(|e| CmdError::failure(format!("writing run summary: {e}")))?;
    }
    Ok(())
}

/// stdout/output contract for `orts run`: stdout carries exactly one of the
/// simulation data or (with `--json`) the run summary; everything else
/// (progress, errors) goes to stderr.
enum DataSink<'a> {
    Stdout,
    File(&'a str),
}

/// `-` is the canonical stdout sentinel; `stdout` is kept as a legacy alias.
fn is_stdout_sentinel(s: &str) -> bool {
    s == "-" || s == "stdout"
}

/// Decide where the simulation data goes. With no `--output`, CSV defaults to
/// stdout (text) while RRD defaults to `output.rrd` (binary should not land on
/// a terminal).
fn resolve_data_sink(output: Option<&str>, format: OutputFormat) -> DataSink<'_> {
    match output {
        Some(s) if is_stdout_sentinel(s) => DataSink::Stdout,
        Some(path) => DataSink::File(path),
        None => match format {
            OutputFormat::Csv => DataSink::Stdout,
            OutputFormat::Rrd => DataSink::File("output.rrd"),
        },
    }
}

/// Reject contradictory stdout requests before the simulation runs.
fn validate_output_contract(
    sink: &DataSink,
    format: OutputFormat,
    json: bool,
) -> Result<(), CmdError> {
    if matches!(sink, DataSink::Stdout) && matches!(format, OutputFormat::Rrd) {
        return Err(CmdError::usage(
            "cannot write .rrd data to stdout. Use --format csv or pass --output <path>.",
        ));
    }
    if json && matches!(sink, DataSink::Stdout) {
        return Err(CmdError::usage(
            "--json writes the run summary to stdout, so simulation data cannot also go \
             to stdout. Pass --output <path> for the data (e.g. --output result.csv).",
        ));
    }
    Ok(())
}

/// Write the recording to the resolved sink and return the file artifact (if
/// any) for inclusion in the JSON summary.
fn write_simulation_output(
    rec: &Recording,
    params: &SimParams,
    sink: &DataSink,
    format: OutputFormat,
) -> Result<Option<Artifact>, CmdError> {
    match (sink, format) {
        (DataSink::Stdout, OutputFormat::Csv) => {
            // A reader that closes the pipe early (`orts run | head`) is an
            // I/O error, not a reason to panic out of the writer.
            print_recording_as_csv(rec, params)
                .map_err(|e| CmdError::failure(format!("writing CSV to stdout: {e}")))?;
            Ok(None)
        }
        // Rejected earlier by validate_output_contract.
        (DataSink::Stdout, OutputFormat::Rrd) => {
            unreachable!("rrd-to-stdout is rejected by validate_output_contract")
        }
        (DataSink::File(path), OutputFormat::Csv) => {
            let mut file = std::fs::File::create(path)
                .map_err(|e| CmdError::failure(format!("creating {path}: {e}")))?;
            write_recording_as_csv(&mut file, rec, Some(params))
                .map_err(|e| CmdError::failure(format!("writing {path}: {e}")))?;
            eprintln!("Saved to {path}");
            Ok(Some(Artifact {
                kind: "recording",
                format: "csv",
                path: (*path).to_string(),
            }))
        }
        (DataSink::File(path), OutputFormat::Rrd) => {
            orts::record::rerun_export::save_as_rrd(rec, "orts", path)
                .map_err(|e| CmdError::failure(format!("saving .rrd: {e}")))?;
            eprintln!("Saved to {path}");
            Ok(Some(Artifact {
                kind: "recording",
                format: "rrd",
                path: (*path).to_string(),
            }))
        }
    }
}

// --- Machine-readable run summary (`orts run --json`) ----------------------
//
// Stable, versioned JSON contract for coding agents and scripts: status,
// the resolved simulation parameters, each satellite's final state, and the
// output artifact. The `schema` field is a version tag, not a fetched URL.

#[derive(serde::Serialize)]
struct RunSummary {
    schema: &'static str,
    status: &'static str,
    command: &'static str,
    simulation: SimSummary,
    satellites: Vec<SatSummary>,
    artifacts: Vec<Artifact>,
    warnings: Vec<String>,
}

#[derive(serde::Serialize)]
struct SimSummary {
    body: String,
    epoch: Option<String>,
    dt_s: f64,
    output_interval_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_s: Option<f64>,
    integrator: IntegratorSummary,
}

#[derive(serde::Serialize)]
struct IntegratorSummary {
    #[serde(rename = "type")]
    kind: &'static str,
    /// Adaptive integrators only (dp45, dop853).
    #[serde(skip_serializing_if = "Option::is_none")]
    atol: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtol: Option<f64>,
}

#[derive(serde::Serialize)]
struct SatSummary {
    id: String,
    samples: usize,
    #[serde(rename = "final")]
    final_state: Option<FinalState>,
    // NOTE: early-termination reporting (e.g. atmospheric reentry detected by
    // the event checker) is logged to stderr but not yet threaded into this
    // summary. Rather than emit an always-null `termination` field that would
    // let consumers mistake an early stop for normal completion, it is omitted
    // from v1 and will be added once the reason is carried through accurately.
}

#[derive(serde::Serialize)]
struct FinalState {
    t_s: f64,
    position_km: [f64; 3],
    velocity_km_s: [f64; 3],
}

#[derive(serde::Serialize)]
struct Artifact {
    kind: &'static str,
    format: &'static str,
    path: String,
}

/// Assemble the JSON run summary from the resolved parameters and recording.
fn build_run_summary(
    params: &SimParams,
    rec: &Recording,
    artifact: Option<Artifact>,
    warnings: Vec<String>,
) -> RunSummary {
    let satellites = params
        .satellites
        .iter()
        .map(|s| {
            let (samples, final_state) = satellite_final_state(rec, &s.entity_path());
            SatSummary {
                id: s.id.clone(),
                samples,
                final_state,
            }
        })
        .collect();

    let (integrator_type, adaptive) = match params.integrator {
        IntegratorChoice::Rk4 => ("rk4", false),
        IntegratorChoice::Dp45 => ("dp45", true),
        IntegratorChoice::Dop853 => ("dop853", true),
    };

    RunSummary {
        schema: "orts.run-summary/v1",
        status: "ok",
        command: "run",
        simulation: SimSummary {
            body: params.body.properties().name.to_lowercase(),
            epoch: params.epoch.as_ref().map(|e| e.to_datetime().to_string()),
            dt_s: params.dt,
            output_interval_s: params.output_interval,
            duration_s: params.duration,
            integrator: IntegratorSummary {
                kind: integrator_type,
                atol: adaptive.then_some(params.tolerances.atol),
                rtol: adaptive.then_some(params.tolerances.rtol),
            },
        },
        satellites,
        artifacts: artifact.into_iter().collect(),
        warnings,
    }
}

/// Read a satellite's sample count and final position/velocity out of the
/// recording. Returns `(0, None)` when the satellite has no recorded samples.
fn satellite_final_state(rec: &Recording, sat_path: &EntityPath) -> (usize, Option<FinalState>) {
    use orts::record::component::Component;
    use orts::record::components::{Position3D, Velocity3D};
    use orts::record::timeline::{TimeIndex, TimelineName};

    let Some(store) = rec.entity(sat_path) else {
        return (0, None);
    };
    let Some(pos_col) = store.columns.get(&Position3D::component_name()) else {
        return (0, None);
    };
    let samples = pos_col.num_rows();
    if samples == 0 {
        return (0, None);
    }
    let i = samples - 1;

    let t_s = store
        .timelines
        .get(&TimelineName::SimTime)
        .and_then(|tl| tl.get(i))
        .map(|ti| match ti {
            TimeIndex::Seconds(s) => *s,
            _ => 0.0,
        })
        .unwrap_or(0.0);

    let vel_row = store
        .columns
        .get(&Velocity3D::component_name())
        .and_then(|c| c.get_row(i));
    let final_state = match (pos_col.get_row(i), vel_row) {
        (Some(pos), Some(vel)) => Some(FinalState {
            t_s,
            position_km: [pos[0], pos[1], pos[2]],
            velocity_km_s: [vel[0], vel[1], vel[2]],
        }),
        _ => None,
    };

    (samples, final_state)
}

/// Build one visibility monitor per satellite, or `None` when ground
/// stations are not configured / not applicable (non-Earth body, no epoch).
fn build_visibility_monitors(params: &SimParams) -> Option<Vec<VisibilityMonitor<SimpleEci>>> {
    if params.ground_stations.is_empty() {
        return None;
    }
    if params.body != KnownBody::Earth {
        eprintln!(
            "Warning: ground stations require Earth as the central body (got {}); \
             contact window detection disabled",
            params.body.properties().name
        );
        return None;
    }
    let Some(epoch) = params.epoch else {
        eprintln!("Warning: ground stations require an epoch; contact window detection disabled");
        return None;
    };
    Some(
        params
            .satellites
            .iter()
            .map(|_| VisibilityMonitor::new(epoch, (), params.ground_stations.clone()))
            .collect(),
    )
}

/// Feed per-satellite `(t, ECI position)` samples to the monitors.
///
/// `last_t` guards against re-feeding a satellite whose time did not advance
/// (finished or terminated), so call sites can pass every satellite each time.
fn feed_visibility(
    monitors: &mut [VisibilityMonitor<SimpleEci>],
    last_t: &mut [f64],
    states: impl Iterator<Item = (f64, nalgebra::Vector3<f64>)>,
) {
    for (i, (t, pos)) in states.enumerate() {
        if t > last_t[i] {
            monitors[i].update(t, &Vec3::from_raw(pos));
            last_t[i] = t;
        }
    }
}

/// Print all detected contact windows to stderr, chronologically by AOS.
///
/// AOS/LOS are linear interpolations between visibility samples (accepted
/// integrator steps on the uncontrolled path, control ticks on the
/// controlled path); passes shorter than one sample gap can still be missed.
fn report_contact_windows(params: &SimParams, monitors: Vec<VisibilityMonitor<SimpleEci>>) {
    let Some(epoch) = params.epoch else { return };
    let mut rows: Vec<(&str, StationContact)> = monitors
        .into_iter()
        .zip(&params.satellites)
        .flat_map(|(monitor, spec)| {
            monitor
                .finish()
                .into_iter()
                .map(move |contact| (spec.id.as_str(), contact))
        })
        .collect();
    if rows.is_empty() {
        eprintln!("Contact windows: none detected");
        return;
    }
    rows.sort_by(|a, b| a.1.window.aos.total_cmp(&b.1.window.aos));

    eprintln!("Contact windows ({}):", rows.len());
    let mut clipped = false;
    for (sat, contact) in &rows {
        let w = &contact.window;
        clipped |= w.open_start || w.open_end;
        eprintln!(
            "  {sat} × {}  AOS{} {} (t={:.1}s)  LOS{} {} (t={:.1}s)  max el {:.1}° @ t={:.1}s",
            contact.station,
            if w.open_start { "*" } else { "" },
            epoch.add_si_seconds(w.aos).to_datetime_normalized(),
            w.aos,
            if w.open_end { "*" } else { "" },
            epoch.add_si_seconds(w.los).to_datetime_normalized(),
            w.los,
            w.max_elevation.to_degrees(),
            w.max_elevation_time,
        );
    }
    if clipped {
        eprintln!("  (* = clipped by the simulation span)");
    }
}

/// Integrator selection for the `run` propagation loop.
fn integrator_config(params: &SimParams) -> IntegratorConfig {
    match params.integrator {
        IntegratorChoice::Rk4 => IntegratorConfig::Rk4 { dt: params.dt },
        IntegratorChoice::Dp45 => IntegratorConfig::Dp45 {
            dt: params.dt,
            tolerances: params.tolerances.clone(),
        },
        IntegratorChoice::Dop853 => IntegratorConfig::Dop853 {
            dt: params.dt,
            tolerances: params.tolerances.clone(),
        },
    }
}

/// Terminate a satellite on surface impact or atmospheric entry.
///
/// Generic over the state so the orbit-only and spacecraft paths share one
/// termination rule: both only need the position.
fn body_event_checker<S: HasPosition>(
    params: &SimParams,
) -> impl Fn(f64, &S) -> ControlFlow<String> + Send + 'static {
    let props = params.body.properties();
    let body_radius = props.radius;
    let atmosphere_altitude = props.atmosphere_altitude;
    move |_t: f64, state: &S| {
        let r = state.position().magnitude();
        if r < body_radius {
            ControlFlow::Break(format!("collision at {:.1} km altitude", r - body_radius))
        } else if let Some(atm_alt) = atmosphere_altitude {
            if r < body_radius + atm_alt {
                ControlFlow::Break(format!(
                    "atmospheric entry at {:.1} km altitude",
                    r - body_radius
                ))
            } else {
                ControlFlow::Continue(())
            }
        } else {
            ControlFlow::Continue(())
        }
    }
}

/// Run the orbit-only simulation and return a Recording.
pub fn run_simulation(params: &SimParams) -> Result<Recording, CmdError> {
    use crate::sim::core::sat_params;
    use orts::setup::{build_orbital_system, default_third_bodies};

    let mut group = IndependentGroup::new(integrator_config(params))
        .with_event_checker(body_event_checker::<OrbitalState>(params));

    let third_bodies = default_third_bodies(&params.body);
    for sat in &params.satellites {
        let system = build_orbital_system(
            &params.body,
            params.mu,
            params.epoch,
            &sat_params(sat),
            &third_bodies,
            params.build_atmosphere_model(),
        );
        let initial = sat
            .initial_state(params.mu, params.epoch)
            .map_err(|e| CmdError::failure(format!("satellite '{}': {e}", sat.id)))?;

        group = group.add_satellite_until(sat.id.as_str(), initial, sat.period, system);
    }

    propagate_and_record(params, group, |rec, entity, tp, state| {
        let os = RecordOrbitalState::new(*state.position(), *state.velocity());
        rec.log_orbital_state(entity, tp, &os);
    })
}

/// Run the orbit + attitude simulation (`[satellites.attitude]` without a
/// plugin controller) and return a Recording.
///
/// Builds the same dynamics `orts serve` builds in spacecraft mode
/// (`SpacecraftDynamics` plus the coupled gravity-gradient torque), so the
/// same config is propagated identically by both entry points. The recording
/// carries the attitude quaternion and body-frame angular velocity in
/// addition to the orbital state.
pub fn run_spacecraft_simulation(params: &SimParams) -> Result<Recording, CmdError> {
    use crate::sim::core::sat_params;
    use orts::attitude::CoupledGravityGradient;
    use orts::setup::{build_spacecraft_dynamics, default_third_bodies};
    use orts::spacecraft::SpacecraftState;

    let mut group =
        IndependentGroup::new(integrator_config(params)).with_event_checker(body_event_checker::<
            orts::effector::AugmentedState<SpacecraftState>,
        >(params));

    let third_bodies = default_third_bodies(&params.body);
    for sat in &params.satellites {
        // `run_simulation_cmd` validated every satellite before dispatching
        // here, so `build_spacecraft_dynamics` cannot be reached with an
        // inertia tensor it would fail to invert.
        let att = sat
            .attitude_config
            .as_ref()
            .expect("spacecraft mode requires attitude config on every satellite");
        let inertia = att.inertia_matrix();
        let mut dynamics = build_spacecraft_dynamics(
            &params.body,
            params.mu,
            params.epoch,
            &sat_params(sat),
            &third_bodies,
            inertia,
            params.build_atmosphere_model(),
        );
        dynamics = dynamics.with_model(CoupledGravityGradient::new(params.mu, inertia));

        let orbit = sat
            .initial_state(params.mu, params.epoch)
            .map_err(|e| CmdError::failure(format!("satellite '{}': {e}", sat.id)))?;
        let plant = SpacecraftState {
            orbit,
            attitude: orts::attitude::AttitudeState {
                quaternion: att.normalized_initial_quaternion(),
                angular_velocity: nalgebra::Vector3::from_row_slice(&att.initial_angular_velocity),
            },
            mass: att.mass,
        };
        let initial = dynamics.initial_augmented_state(plant);
        group = group.add_satellite_until(sat.id.as_str(), initial, sat.period, dynamics);
    }

    propagate_and_record(params, group, |rec, entity, tp, state| {
        let sc = &state.plant;
        let os = RecordOrbitalState::new(*sc.orbit.position(), *sc.orbit.velocity());
        let q = Quaternion4D(sc.attitude.quaternion);
        let w = AngularVelocity3D(sc.attitude.angular_velocity);
        rec.log_orbital_state_with_attitude(entity, tp, &os, Some(&q), Some(&w));
    })
}

/// Propagate an already-populated group in `output_interval` chunks and
/// record each satellite's state through `log_state`.
///
/// Shared by the orbit-only and spacecraft paths: everything except the
/// dynamics and what a sample contains is identical, so termination
/// reporting, ground-station visibility and the recording metadata have one
/// implementation instead of one per mode.
fn propagate_and_record<D>(
    params: &SimParams,
    mut group: IndependentGroup<D>,
    log_state: impl Fn(&mut Recording, &EntityPath, &TimePoint, &D::State),
) -> Result<Recording, CmdError>
where
    D: DynamicalSystem,
    D::State: HasPosition,
{
    let mut rec = Recording::new();
    let body_path = EntityPath::parse(&format!("/world/{}", params.body.properties().name));

    rec.log_static(&body_path, &GravitationalParameter(params.mu));
    rec.log_static(&body_path, &BodyRadius(params.body.properties().radius));

    // Track entity paths per satellite for recording
    let sat_paths: Vec<EntityPath> = params.satellites.iter().map(|s| s.entity_path()).collect();

    // Ground-station visibility monitors, fed from accepted integrator
    // steps via the propagation observer (independent of output_interval).
    let mut visibility = build_visibility_monitors(params);
    let mut vis_last_t: Vec<f64> = vec![f64::NEG_INFINITY; params.satellites.len()];
    let sat_index: std::collections::HashMap<&str, usize> = params
        .satellites
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    // Record initial states
    let mut steps: Vec<u64> = vec![0; params.satellites.len()];
    let mut last_output_t: Vec<f64> = vec![0.0; params.satellites.len()];
    for (i, (entry, _)) in group.satellites_with_dynamics().enumerate() {
        let tp = TimePoint::new().with_sim_time(0.0).with_step(0);
        log_state(&mut rec, &sat_paths[i], &tp, &entry.state);
        steps[i] = 1;
    }
    if let Some(monitors) = visibility.as_mut() {
        feed_visibility(
            monitors,
            &mut vis_last_t,
            group
                .satellites_with_dynamics()
                .map(|(e, _)| (e.t, e.state.position())),
        );
    }

    // Propagate in output_interval steps
    let max_period = params
        .satellites
        .iter()
        .map(|s| s.period)
        .fold(0.0_f64, f64::max);
    let mut t = 0.0_f64;

    while !group.all_finished() {
        t += params.output_interval;
        if t > max_period {
            t = max_period;
        }

        // Propagation errors are reported, not unwrapped: an invalid step size
        // or a stalled clock is a diagnosable condition, and panicking here
        // discards the `IntegrationError` that says which.
        let outcome = if let Some(monitors) = visibility.as_mut() {
            group
                .propagate_to_with(t, |id, ts, state| {
                    let Some(&i) = sat_index.get(AsRef::<str>::as_ref(id)) else {
                        return;
                    };
                    if ts > vis_last_t[i] {
                        monitors[i].update(ts, &Vec3::from_raw(state.position()));
                        vis_last_t[i] = ts;
                    }
                })
                .map_err(|e| format!("integration failed while advancing to t={t:.3}: {e}"))?
        } else {
            group
                .propagate_to(t)
                .map_err(|e| format!("integration failed while advancing to t={t:.3}: {e}"))?
        };

        // Record states for satellites that reached this output time
        for (i, (entry, _)) in group.satellites_with_dynamics().enumerate() {
            if !entry.terminated && entry.t >= t - 1e-9 {
                let tp = TimePoint::new().with_sim_time(entry.t).with_step(steps[i]);
                log_state(&mut rec, &sat_paths[i], &tp, &entry.state);
                steps[i] += 1;
                last_output_t[i] = entry.t;
            }
        }

        // Report terminations
        for term in &outcome.terminations {
            eprintln!(
                "Simulation terminated at t={:.2}s for {}: {}",
                term.t, term.satellite_id, term.reason
            );
            // Record final state for terminated satellites
            if let Some(i) = params
                .satellites
                .iter()
                .position(|s| s.id.as_str() == AsRef::<str>::as_ref(&term.satellite_id))
                && let Some(entry) = group.satellite(&term.satellite_id)
            {
                let tp = TimePoint::new().with_sim_time(entry.t).with_step(steps[i]);
                log_state(&mut rec, &sat_paths[i], &tp, &entry.state);
                steps[i] += 1;
            }
        }
    }

    // Record final states for satellites that finished at end_time
    // (covers the case where period doesn't align with output_interval)
    for (i, (entry, _)) in group.satellites_with_dynamics().enumerate() {
        if !entry.terminated && (entry.t - last_output_t[i]) > 1e-9 {
            let tp = TimePoint::new().with_sim_time(entry.t).with_step(steps[i]);
            log_state(&mut rec, &sat_paths[i], &tp, &entry.state);
        }
    }

    if let Some(monitors) = visibility.take() {
        report_contact_windows(params, monitors);
    }

    rec.metadata = sim_metadata(params);

    Ok(rec)
}

/// Recording metadata describing the run's central body, epoch and the first
/// satellite's orbit (kept for backward compatibility with single-satellite
/// consumers).
fn sim_metadata(params: &SimParams) -> orts::record::recording::SimMetadata {
    let first_sat = params.satellites.first();
    let orbit_desc = first_sat.map(|s| match &s.orbit {
        OrbitSpec::Circular { altitude, r0, .. } => {
            format!(
                "Initial orbit: circular at {} km altitude (r = {} km)",
                altitude, r0
            )
        }
        OrbitSpec::Omm { omm } => {
            format!(
                "Initial orbit: from TLE/OMM (a = {:.1} km, e = {:.6}, i = {:.2}°)",
                omm.semi_major_axis(params.mu),
                omm.fields().eccentricity,
                omm.fields().inclination.to_degrees()
            )
        }
    });
    orts::record::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        epoch_iso: params.epoch.map(|e| e.to_datetime().to_string()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        altitude: first_sat.map(|s| s.altitude(&params.body)),
        period: first_sat.map(|s| s.period),
        orbit_description: orbit_desc,
    }
}

/// Print a Recording as CSV to stdout.
pub fn print_recording_as_csv(rec: &Recording, params: &SimParams) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    write_recording_as_csv(&mut stdout, rec, Some(params))
}

/// Write a Recording as CSV to any writer.
///
/// If `params` is provided, satellite entity paths are taken from it;
/// otherwise they are discovered from the Recording. This is the single
/// source of truth for CSV output format.
pub fn write_recording_as_csv(
    w: &mut dyn std::io::Write,
    rec: &Recording,
    params: Option<&SimParams>,
) -> std::io::Result<()> {
    rec.metadata.write_csv_header(w)?;

    let mu = rec.metadata.mu.unwrap_or(398600.4418);

    // Get satellite entity paths + IDs
    let sat_entries: Vec<(EntityPath, String)> = if let Some(params) = params {
        params
            .satellites
            .iter()
            .map(|s| (s.entity_path(), s.id.clone()))
            .collect()
    } else {
        use orts::record::entity_path::EntityPath;
        let prefix = EntityPath::parse("/world/sat");
        let mut paths = rec.entities_under(&prefix);
        paths.sort_by_key(|p| p.to_string());
        paths
            .into_iter()
            .map(|p| {
                let id = p
                    .to_string()
                    .rsplit('/')
                    .next()
                    .unwrap_or("default")
                    .to_string();
                (p.clone(), id)
            })
            .collect()
    };

    let multi_sat = sat_entries.len() > 1;

    if multi_sat {
        writeln!(
            w,
            "# satellites = {}",
            sat_entries
                .iter()
                .map(|(_, id)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }

    if let Some((first_path, _)) = sat_entries.first() {
        writeln!(w, "{}", build_csv_header(rec, first_path, multi_sat))?;
    }

    for (sat_path, id) in &sat_entries {
        if multi_sat {
            writeln!(w, "# --- {id} ---")?;
        }
        write_satellite_csv(w, rec, sat_path, mu, multi_sat)?;
    }

    Ok(())
}

/// Write satellite CSV data to any writer.
pub fn write_satellite_csv(
    w: &mut dyn std::io::Write,
    rec: &Recording,
    sat_path: &EntityPath,
    mu: f64,
    with_id: bool,
) -> std::io::Result<()> {
    use orts::record::component::Component;
    use orts::record::components::{Position3D, Velocity3D};
    use orts::record::timeline::TimelineName;

    let store = match rec.entity(sat_path) {
        Some(s) => s,
        None => return Ok(()),
    };
    let pos_col = match store.columns.get(&Position3D::component_name()) {
        Some(c) => c,
        None => return Ok(()),
    };
    let vel_col = match store.columns.get(&Velocity3D::component_name()) {
        Some(c) => c,
        None => return Ok(()),
    };
    let sim_times = match store.timelines.get(&TimelineName::SimTime) {
        Some(t) => t,
        None => return Ok(()),
    };

    // Collect extra columns (everything except Position3D and Velocity3D), sorted by name
    let skip = [Position3D::component_name(), Velocity3D::component_name()];
    let mut extra_cols: Vec<_> = store
        .columns
        .iter()
        .filter(|(name, _)| !skip.contains(name))
        .collect();
    extra_cols.sort_by_key(|(a, _)| *a);

    let id = sat_path.to_string();
    let id = id.rsplit('/').next().unwrap_or("default");

    for i in 0..pos_col.num_rows() {
        let t = match sim_times.get(i) {
            Some(orts::record::timeline::TimeIndex::Seconds(s)) => *s,
            _ => 0.0,
        };
        let pos = pos_col.get_row(i).unwrap();
        let vel = vel_col.get_row(i).unwrap();
        let pos_vec = nalgebra::Vector3::new(pos[0], pos[1], pos[2]);
        let vel_vec = nalgebra::Vector3::new(vel[0], vel[1], vel[2]);
        let elements = KeplerianElements::from_state_vector(&pos_vec, &vel_vec, mu);

        let mut line = String::new();
        if with_id {
            line.push_str(&format!("{},", id));
        }
        line.push_str(&format!(
            "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.10},{:.10},{:.10},{:.10},{:.10}",
            t,
            pos[0],
            pos[1],
            pos[2],
            vel[0],
            vel[1],
            vel[2],
            elements.semi_major_axis,
            elements.eccentricity,
            elements.inclination,
            elements.raan,
            elements.argument_of_periapsis,
            elements.true_anomaly,
        ));

        for (_name, col) in &extra_cols {
            if let Some(row) = col.get_row(i) {
                for val in row {
                    line.push_str(&format!(",{:.10}", val));
                }
            }
        }

        writeln!(w, "{line}")?;
    }
    Ok(())
}

/// Build the CSV header line dynamically from the Recording's columns.
pub fn build_csv_header(rec: &Recording, sat_path: &EntityPath, with_id: bool) -> String {
    use orts::record::component::Component;
    use orts::record::components::{Position3D, Velocity3D};

    let mut header = String::new();
    header.push_str("# ");
    if with_id {
        header.push_str("satellite_id,");
    }
    header.push_str("t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s],a[km],e[-],i[rad],raan[rad],omega[rad],nu[rad]");

    if let Some(store) = rec.entity(sat_path) {
        let skip = [Position3D::component_name(), Velocity3D::component_name()];
        let mut extra_cols: Vec<_> = store
            .columns
            .keys()
            .filter(|name| !skip.contains(name))
            .collect();
        extra_cols.sort();

        for name in extra_cols {
            // Use component name as column prefix (strip "orts." prefix)
            let short = name.strip_prefix("orts.").unwrap_or(name);
            if let Some(col) = store.columns.get(name) {
                let n = col.scalars_per_row;
                if n == 1 {
                    header.push_str(&format!(",{short}"));
                } else {
                    // Look up field names from known components
                    let field_names = lookup_field_names(name, n);
                    for fname in field_names {
                        header.push_str(&format!(",{fname}"));
                    }
                }
            }
        }
    }

    header
}

fn lookup_field_names(component_name: &str, n: usize) -> Vec<String> {
    use orts::record::component::Component;

    // Known component types — return their field_names
    macro_rules! try_component {
        ($ty:ty) => {
            if <$ty>::component_name() == component_name {
                return <$ty>::field_names().iter().map(|s| s.to_string()).collect();
            }
        };
    }
    try_component!(Quaternion4D);
    try_component!(AngularVelocity3D);
    try_component!(MtqCommand3D);
    try_component!(RwTorqueCommand3D);
    try_component!(RwMomentum3D);
    try_component!(ThrusterThrottle3D);

    // Fallback: generate numbered names
    let short = component_name
        .strip_prefix("orts.")
        .unwrap_or(component_name);
    (0..n).map(|i| format!("{short}_{i}")).collect()
}

/// 制御付きシミュレーション（プラグインコントローラ + RW + センサ）。
fn run_controlled_simulation(params: &SimParams, sim: &SimArgs) -> Result<Recording, CmdError> {
    use crate::sim::controlled::{
        ControlledBuildContext, build_controlled_satellite, step_controlled,
    };
    let _ = sim; // Plugin backend is now stored in SimParams directly.

    let duration = params.duration.unwrap_or_else(|| {
        // フォールバック: 最初の衛星の軌道周期。
        params
            .satellites
            .first()
            .map(|s| s.period)
            .unwrap_or(3600.0)
    });

    let mut rec = Recording::new();
    let body_path = EntityPath::parse(&format!("/world/{}", params.body.properties().name));
    rec.log_static(&body_path, &GravitationalParameter(params.mu));
    rec.log_static(&body_path, &BodyRadius(params.body.properties().radius));

    let sat_paths: Vec<EntityPath> = params.satellites.iter().map(|s| s.entity_path()).collect();

    // WASM plugin cache（複数衛星で共有する engine + compiled component）。
    #[cfg(feature = "plugin-wasm")]
    let plugin_backend = params.resolve_plugin_backend();
    #[cfg(feature = "plugin-wasm-async")]
    let async_mode = params.resolve_async_mode();
    #[cfg(feature = "plugin-wasm")]
    let mut wasm_cache = {
        #[cfg(feature = "plugin-wasm-async")]
        {
            orts::plugin::wasm::WasmPluginCache::new_with_async_mode(async_mode)
                .map_err(|e| CmdError::failure(format!("initializing WASM plugin cache: {e}")))?
        }
        #[cfg(not(feature = "plugin-wasm-async"))]
        {
            orts::plugin::wasm::WasmPluginCache::new()
                .map_err(|e| CmdError::failure(format!("initializing WASM plugin cache: {e}")))?
        }
    };
    // Only use rayon parallelism for the sim loop when the user
    // explicitly asked for the throughput async backend. Deterministic
    // mode (and the sync backend) stay on the sequential `for` loop so
    // that oracle tests keep their bit-exact guarantees.
    #[cfg(feature = "plugin-wasm-async")]
    let parallel_step = matches!(
        plugin_backend,
        crate::sim::params::ResolvedPluginBackend::Async
    ) && async_mode == orts::plugin::wasm::AsyncMode::Throughput;
    #[cfg(not(feature = "plugin-wasm-async"))]
    let parallel_step = false;

    // 制御付き衛星を構築。
    let mut satellites = Vec::new();
    {
        let mut ctx = ControlledBuildContext {
            params,
            #[cfg(feature = "plugin-wasm")]
            wasm_cache: &mut wasm_cache,
            #[cfg(feature = "plugin-wasm")]
            plugin_backend,
        };
        for spec in &params.satellites {
            let sat = build_controlled_satellite(spec, params.epoch, &mut ctx).map_err(|e| {
                CmdError::failure(format!("building controlled satellite '{}': {e}", spec.id))
            })?;
            satellites.push(sat);
        }
    }

    // 各コントローラのノード identity を設定（msg-io outbound の src として
    // stamp される）。衛星はベクタ内の位置でアドレス付けする。
    for (i, sat) in satellites.iter_mut().enumerate() {
        sat.controller
            .set_node_id(orts::plugin::NodeId::Satellite(i as u32));
    }

    // config の時刻指定コマンド (`[[command]]`) を時刻順キューに積む。
    // host が tick ごとに due なものを配送する決定論的 transport adapter。
    let mut command_schedule = {
        use std::collections::HashMap;
        let id_to_index: HashMap<&str, usize> = params
            .satellites
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.as_str(), i))
            .collect();
        let mut scheduled = Vec::new();
        for cmd in &params.commands {
            let Some(&idx) = id_to_index.get(cmd.sat.as_str()) else {
                return Err(CmdError::usage(format!(
                    "command targets unknown satellite '{}'",
                    cmd.sat
                )));
            };
            let message = cmd
                .to_message(idx)
                .map_err(|e| CmdError::usage(format!("invalid command: {e}")))?;
            scheduled.push(crate::sim::command_schedule::ScheduledCommand {
                t: cmd.t,
                sat_index: idx,
                message,
            });
        }
        crate::sim::command_schedule::CommandSchedule::new(scheduled)
    };

    // 初期状態を記録。
    for (i, sat) in satellites.iter().enumerate() {
        let tp = TimePoint::new().with_sim_time(0.0).with_step(0);
        log_controlled_state(&mut rec, &sat_paths[i], &tp, sat);
    }

    // 地上局可視性 monitor（制御 tick ごとにサンプリング）。
    let mut visibility = build_visibility_monitors(params);
    let mut vis_last_t: Vec<f64> = vec![f64::NEG_INFINITY; params.satellites.len()];
    if let Some(monitors) = visibility.as_mut() {
        feed_visibility(
            monitors,
            &mut vis_last_t,
            satellites
                .iter()
                .map(|s| (0.0, *s.state.plant.orbit.position())),
        );
    }

    // 全衛星の sample_period の最小値をグローバル tick に使う。
    //
    // Validate per satellite *before* folding: `f64::min` returns the other
    // argument when one side is NaN, so a NaN period would be silently
    // discarded by the fold and never rejected.
    for (i, sat) in satellites.iter().enumerate() {
        crate::config::validate_sample_period(sat.controller.sample_period())
            .map_err(|e| CmdError::failure(format!("satellites[{i}]: {e}")))?;
    }
    let dt_ctrl = satellites
        .iter()
        .map(|sat| sat.controller.sample_period())
        .fold(f64::INFINITY, f64::min);
    // `fold` also yields `INFINITY` for an empty fleet, which the loop below
    // cannot step with either.
    crate::config::validate_sample_period(dt_ctrl).map_err(CmdError::failure)?;
    let dt_ode = params.dt.min(dt_ctrl);

    let mut t = 0.0;
    let mut step: u64 = 1;
    let mut next_output_t = params.output_interval;
    let mut last_output_t = 0.0;

    while t < duration - 1e-12 {
        let dt = dt_ctrl.min(duration - t);

        // 時刻指定コマンド: この制御 tick の終端 (t+dt) までに due なものを
        // 配送する。`src` は controller(host) が確定する。
        for sc in command_schedule.drain_due(t + dt) {
            satellites[sc.sat_index]
                .controller
                .deliver(sc.message.clone());
        }

        // `try_for_each` rather than `for_each` + exit: a rayon worker calling
        // `std::process::exit` tears the process down from inside the pool,
        // skipping every caller's cleanup. Collect the first error instead.
        if parallel_step {
            use rayon::prelude::*;
            satellites
                .par_iter_mut()
                .try_for_each(|sat| step_controlled(sat, t, dt, dt_ode, params.epoch.as_ref()))
                .map_err(|e| CmdError::failure(format!("simulation error at t={t:.3}: {e}")))?;
        } else {
            for sat in &mut satellites {
                step_controlled(sat, t, dt, dt_ode, params.epoch.as_ref())
                    .map_err(|e| CmdError::failure(format!("simulation error at t={t:.3}: {e}")))?;
            }
        }

        // FSW からの downlink（テレメトリ / ISL）を回収してログに出す。
        // controller が msg-io 未対応なら default 実装で空。
        for (i, sat) in satellites.iter_mut().enumerate() {
            for m in sat.controller.take_outbound() {
                // Both below the default filter: this runs per satellite per
                // outbound message per control tick. Payloads can be large
                // (binary / file-transfer), so they sit one level lower again.
                log::debug!(
                    "downlink t={:.3} sat={} kind={}",
                    t + dt,
                    params.satellites[i].id,
                    m.kind
                );
                log::trace!(
                    "downlink payload sat={} kind={}: {:?}",
                    params.satellites[i].id,
                    m.kind,
                    m.payload
                );
            }
        }

        t += dt;

        // 可視性は出力間引きと独立に、制御 tick ごとにサンプリングする。
        if let Some(monitors) = visibility.as_mut() {
            feed_visibility(
                monitors,
                &mut vis_last_t,
                satellites
                    .iter()
                    .map(|s| (t, *s.state.plant.orbit.position())),
            );
        }

        if t >= next_output_t - 1e-12 {
            for (i, sat) in satellites.iter().enumerate() {
                let tp = TimePoint::new().with_sim_time(t).with_step(step);
                log_controlled_state(&mut rec, &sat_paths[i], &tp, sat);
            }
            step += 1;
            last_output_t = t;
            next_output_t += params.output_interval;
        }
    }

    // 最終状態を記録（output_interval と duration が割り切れない場合）。
    if (t - last_output_t) > 1e-9 {
        for (i, sat) in satellites.iter().enumerate() {
            let tp = TimePoint::new().with_sim_time(t).with_step(step);
            log_controlled_state(&mut rec, &sat_paths[i], &tp, sat);
        }
    }

    if let Some(monitors) = visibility.take() {
        report_contact_windows(params, monitors);
    }

    rec.metadata = sim_metadata(params);

    Ok(rec)
}

/// Log controlled satellite state: orbit + attitude + commands + actuator telemetry.
fn log_controlled_state(
    rec: &mut Recording,
    entity: &EntityPath,
    tp: &TimePoint,
    sat: &crate::sim::controlled::ControlledSatellite,
) {
    use orts::plugin::{
        MtqCommand as PluginMtqCommand, RwCommand as PluginRwCommand,
        ThrusterCommand as PluginThrusterCommand,
    };
    use orts::spacecraft::ReactionWheelAssembly;

    let orbit = &sat.state.plant.orbit;
    let os = RecordOrbitalState::new(*orbit.position(), *orbit.velocity());
    let att = &sat.state.plant.attitude;
    let q = Quaternion4D(att.quaternion);
    let w = AngularVelocity3D(att.angular_velocity);
    rec.log_orbital_state_with_attitude(entity, tp, &os, Some(&q), Some(&w));

    // MTQ command (always log to keep row count aligned with orbital state).
    // TODO: distinguish Moments vs NormalizedMoments — currently both are
    // recorded as MtqCommand3D with A·m² labels. NormalizedMoments values
    // are [-1, 1] and should be scaled or use a separate component.
    if sat.has_mtq {
        let mtq_vec = sat
            .actuators
            .mtq_command()
            .and_then(|cmd| {
                let v = match cmd {
                    PluginMtqCommand::Moments(v) | PluginMtqCommand::NormalizedMoments(v) => v,
                };
                (v.len() >= 3).then(|| nalgebra::Vector3::new(v[0], v[1], v[2]))
            })
            .unwrap_or(nalgebra::Vector3::zeros());
        rec.log_temporal(entity, tp, &MtqCommand3D(mtq_vec));
    }

    // RW command (always log to keep row count aligned).
    // TODO: distinguish Torques vs Speeds — currently both are recorded
    // as RwTorqueCommand3D. Speeds (rad/s) should use a separate component.
    if sat.has_rw {
        let rw_vec = sat
            .actuators
            .rw_command()
            .and_then(|cmd| {
                let v = match cmd {
                    PluginRwCommand::Torques(v) | PluginRwCommand::Speeds(v) => v,
                };
                (v.len() >= 3).then(|| nalgebra::Vector3::new(v[0], v[1], v[2]))
            })
            .unwrap_or(nalgebra::Vector3::zeros());
        rec.log_temporal(entity, tp, &RwTorqueCommand3D(rw_vec));
    }

    // Thruster throttle per thruster (first 3 if >3). Always log when the
    // spacecraft has thrusters, so plotting tools can show burn intervals
    // without inferring them from orbit-element deltas.
    if !sat.thruster_specs.is_empty() {
        let throttle_vec = sat
            .actuators
            .thruster_command()
            .map(|cmd| {
                let PluginThrusterCommand::Throttles(v) = cmd;
                nalgebra::Vector3::new(
                    v.first().copied().unwrap_or(0.0),
                    v.get(1).copied().unwrap_or(0.0),
                    v.get(2).copied().unwrap_or(0.0),
                )
            })
            .unwrap_or_else(nalgebra::Vector3::zeros);
        rec.log_temporal(entity, tp, &ThrusterThrottle3D(throttle_vec));
    }

    // RW momentum telemetry
    if sat.has_rw
        && let Some(rw) = sat
            .dynamics
            .effector_by_name::<ReactionWheelAssembly>("reaction_wheels")
    {
        let momentum = rw.core().momentum_slice(&sat.state.aux);
        if momentum.len() >= 3 {
            rec.log_temporal(
                entity,
                tp,
                &RwMomentum3D(nalgebra::Vector3::new(
                    momentum[0],
                    momentum[1],
                    momentum[2],
                )),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Build `SimArgs` the way the CLI does, so clap's defaults are exercised
    /// too. `validate_sim_args` returning `Result` is what makes this testable
    /// at all — it used to `process::exit(1)` at the point of detection.
    fn args(extra: &[&str]) -> SimArgs {
        let mut argv = vec!["orts", "--sat", "altitude=500"];
        argv.extend_from_slice(extra);
        SimArgs::try_parse_from(argv).expect("test argv should parse")
    }

    #[test]
    fn default_args_are_valid() {
        assert!(validate_sim_args(&args(&[])).is_ok());
    }

    #[test]
    fn rejects_non_positive_dt() {
        // `--dt=-1` rather than `--dt -1`: clap treats a bare `-1` as an
        // unknown flag, so the negative value only reaches us through the
        // `=` form (or a config file).
        for dt in ["--dt=0", "--dt=-1", "--dt=nan", "--dt=inf"] {
            let e = validate_sim_args(&args(&[dt])).expect_err(&format!("{dt} should be rejected"));
            assert!(e.contains("dt"), "{dt} gave {e:?}");
        }
    }

    #[test]
    fn rejects_output_interval_below_dt() {
        let e = validate_sim_args(&args(&["--dt", "10", "--output-interval", "1"]))
            .expect_err("output_interval < dt");
        assert!(e.contains("output_interval"), "{e:?}");
    }

    #[test]
    fn rejects_unusable_tolerances_for_adaptive_integrators() {
        let e = validate_sim_args(&args(&[
            "--integrator",
            "dp45",
            "--atol",
            "0",
            "--rtol",
            "0",
        ]))
        .expect_err("both-zero tolerances with dp45");
        assert!(e.contains("atol") && e.contains("rtol"), "{e:?}");
    }

    #[test]
    fn accepts_unused_tolerances_for_rk4() {
        // RK4 never reads them, so zeros must not fail the run.
        assert!(
            validate_sim_args(&args(&[
                "--integrator",
                "rk4",
                "--atol",
                "0",
                "--rtol",
                "0"
            ]))
            .is_ok()
        );
    }

    #[test]
    fn rrd_to_stdout_is_a_usage_error() {
        let err = validate_output_contract(&DataSink::Stdout, OutputFormat::Rrd, false)
            .expect_err("rrd to stdout");
        assert_eq!(err.code, 2, "contradictory flags are a usage error");
        assert!(err.message.contains("stdout"), "{err:?}");
    }

    #[test]
    fn json_plus_stdout_data_is_a_usage_error() {
        let err = validate_output_contract(&DataSink::Stdout, OutputFormat::Csv, true)
            .expect_err("json + csv-to-stdout");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn compatible_output_contracts_are_accepted() {
        assert!(validate_output_contract(&DataSink::Stdout, OutputFormat::Csv, false).is_ok());
        assert!(
            validate_output_contract(&DataSink::File("out.csv"), OutputFormat::Csv, true).is_ok()
        );
        assert!(
            validate_output_contract(&DataSink::File("out.rrd"), OutputFormat::Rrd, false).is_ok()
        );
    }
}
