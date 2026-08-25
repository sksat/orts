use std::ops::ControlFlow;

use arika::body::KnownBody;
use arika::frame::{SimpleEci, Vec3};
use orts::OrbitalState;
use orts::group::{IndependentGroup, IntegratorConfig};
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
use crate::satellite::OrbitSpec;
use crate::sim::params::SimParams;

/// Apply the config-file validation rules to the direct-CLI argument path.
///
/// `SimParams::from_sim_args` builds the same `SimParams` as
/// `SimParams::from_config` but skips `SimConfig::validate`, so without this
/// `--dt 0` hangs, `--dt nan` panics in step-size control, and
/// `--dt 10 --output-interval 1` panics inside `clamp`.
pub(crate) fn validate_sim_args(sim: &SimArgs) {
    let checks = crate::config::validate_time_params(
        sim.dt,
        sim.output_interval,
        sim.stream_interval,
        sim.duration,
    )
    .and_then(|()| crate::config::validate_tolerances(sim.integrator, sim.atol, sim.rtol));
    if let Err(e) = checks {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

pub fn run_simulation_cmd(sim: &SimArgs, output: Option<&str>, format: OutputFormat, json: bool) {
    let mut params = if let Some(config_path) = &sim.config {
        let config = crate::config::SimConfig::load(std::path::Path::new(config_path))
            .unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
        SimParams::from_config(&config)
    } else if sim.has_orbit_args() {
        // The direct-CLI path bypasses `SimConfig::validate`, so apply the
        // same time/tolerance checks here rather than letting a bad `--dt`
        // hang the propagation loop or panic inside step-size control.
        validate_sim_args(sim);
        SimParams::from_sim_args(sim, false)
    } else {
        // Auto-detect orts.toml in the current directory
        let config_path = std::path::Path::new("orts.toml");
        if config_path.exists() {
            let config = crate::config::SimConfig::load(config_path).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            SimParams::from_config(&config)
        } else {
            eprintln!("Error: no simulation configuration found.");
            eprintln!("Provide --config <path>, place orts.toml in the current directory,");
            eprintln!("or specify an orbit with --sat / --tle / --norad-id.");
            std::process::exit(1);
        }
    };
    // Resolve and validate the stdout/output contract before running the
    // (potentially long) simulation, so usage errors fail fast.
    let sink = resolve_data_sink(output, format);
    validate_output_contract(&sink, format, json);

    // CLI backend flags always override config-file defaults so
    // `orts run --config … --plugin-backend=sync|async` works.
    params.plugin_backend_choice = sim.plugin_backend;
    params.plugin_backend_threshold = sim.plugin_backend_threshold;
    params.plugin_backend_async_mode = sim.plugin_backend_async_mode;

    // 全衛星が controller 付きなら制御ループへディスパッチ。
    let has_controller = !params.satellites.is_empty()
        && params
            .satellites
            .iter()
            .all(|s| s.controller_config.is_some());
    let rec = if has_controller {
        run_controlled_simulation(&params, sim)
    } else {
        run_simulation(&params)
    };

    let artifact = write_simulation_output(&rec, &params, &sink, format);

    if json {
        let summary = build_run_summary(&params, &rec, artifact);
        serde_json::to_writer_pretty(std::io::stdout(), &summary).unwrap_or_else(|e| {
            eprintln!("Error serializing run summary: {e}");
            std::process::exit(1);
        });
        // Trailing newline so the JSON document is its own line on stdout.
        println!();
    }
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
fn validate_output_contract(sink: &DataSink, format: OutputFormat, json: bool) {
    if matches!(sink, DataSink::Stdout) && matches!(format, OutputFormat::Rrd) {
        eprintln!(
            "Error: cannot write .rrd data to stdout. Use --format csv or pass --output <path>."
        );
        std::process::exit(2);
    }
    if json && matches!(sink, DataSink::Stdout) {
        eprintln!(
            "Error: --json writes the run summary to stdout, so simulation data cannot also go \
             to stdout. Pass --output <path> for the data (e.g. --output result.csv)."
        );
        std::process::exit(2);
    }
}

/// Write the recording to the resolved sink and return the file artifact (if
/// any) for inclusion in the JSON summary.
fn write_simulation_output(
    rec: &Recording,
    params: &SimParams,
    sink: &DataSink,
    format: OutputFormat,
) -> Option<Artifact> {
    match (sink, format) {
        (DataSink::Stdout, OutputFormat::Csv) => {
            print_recording_as_csv(rec, params);
            None
        }
        // Rejected earlier by validate_output_contract.
        (DataSink::Stdout, OutputFormat::Rrd) => {
            unreachable!("rrd-to-stdout is rejected by validate_output_contract")
        }
        (DataSink::File(path), OutputFormat::Csv) => {
            let mut file = std::fs::File::create(path).unwrap_or_else(|e| {
                eprintln!("Error creating {path}: {e}");
                std::process::exit(1);
            });
            write_recording_as_csv(&mut file, rec, Some(params)).unwrap_or_else(|e| {
                eprintln!("Error writing {path}: {e}");
                std::process::exit(1);
            });
            eprintln!("Saved to {path}");
            Some(Artifact {
                kind: "recording",
                format: "csv",
                path: (*path).to_string(),
            })
        }
        (DataSink::File(path), OutputFormat::Rrd) => {
            orts::record::rerun_export::save_as_rrd(rec, "orts", path).unwrap_or_else(|e| {
                eprintln!("Error saving .rrd: {e}");
                std::process::exit(1);
            });
            eprintln!("Saved to {path}");
            Some(Artifact {
                kind: "recording",
                format: "rrd",
                path: (*path).to_string(),
            })
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
        warnings: Vec::new(),
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

/// Run the simulation and return a Recording.
pub fn run_simulation(params: &SimParams) -> Recording {
    use crate::sim::core::sat_params;
    use orts::setup::{build_orbital_system, default_third_bodies};

    let mut rec = Recording::new();
    let body_path = EntityPath::parse(&format!("/world/{}", params.body.properties().name));

    rec.log_static(&body_path, &GravitationalParameter(params.mu));
    rec.log_static(&body_path, &BodyRadius(params.body.properties().radius));

    // Build integrator config
    let config = match params.integrator {
        IntegratorChoice::Rk4 => IntegratorConfig::Rk4 { dt: params.dt },
        IntegratorChoice::Dp45 => IntegratorConfig::Dp45 {
            dt: params.dt,
            tolerances: params.tolerances.clone(),
        },
        IntegratorChoice::Dop853 => IntegratorConfig::Dop853 {
            dt: params.dt,
            tolerances: params.tolerances.clone(),
        },
    };

    // Build event checker (collision + atmospheric entry)
    let props = params.body.properties();
    let body_radius = props.radius;
    let atmosphere_altitude = props.atmosphere_altitude;
    let event_checker = move |_t: f64, state: &OrbitalState| -> ControlFlow<String> {
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
    };

    // Build group with all satellites
    let mut group = IndependentGroup::new(config).with_event_checker(event_checker);

    // Track entity paths per satellite for recording
    let sat_paths: Vec<EntityPath> = params.satellites.iter().map(|s| s.entity_path()).collect();

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
            .unwrap_or_else(|e| panic!("satellite '{}': {e}", sat.id));

        group = group.add_satellite_until(sat.id.as_str(), initial, sat.period, system);
    }

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
        let os = RecordOrbitalState::new(*entry.state.position(), *entry.state.velocity());
        rec.log_orbital_state(&sat_paths[i], &tp, &os);
        steps[i] = 1;
    }
    if let Some(monitors) = visibility.as_mut() {
        feed_visibility(
            monitors,
            &mut vis_last_t,
            group
                .satellites_with_dynamics()
                .map(|(e, _)| (e.t, *e.state.position())),
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

        let outcome = if let Some(monitors) = visibility.as_mut() {
            group
                .propagate_to_with(t, |id, ts, state| {
                    let Some(&i) = sat_index.get(AsRef::<str>::as_ref(id)) else {
                        return;
                    };
                    if ts > vis_last_t[i] {
                        monitors[i].update(ts, &Vec3::from_raw(*state.position()));
                        vis_last_t[i] = ts;
                    }
                })
                .unwrap()
        } else {
            group.propagate_to(t).unwrap()
        };

        // Record states for satellites that reached this output time
        for (i, (entry, _)) in group.satellites_with_dynamics().enumerate() {
            if !entry.terminated && entry.t >= t - 1e-9 {
                let tp = TimePoint::new().with_sim_time(entry.t).with_step(steps[i]);
                let os = RecordOrbitalState::new(*entry.state.position(), *entry.state.velocity());
                rec.log_orbital_state(&sat_paths[i], &tp, &os);
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
                let os = RecordOrbitalState::new(*entry.state.position(), *entry.state.velocity());
                rec.log_orbital_state(&sat_paths[i], &tp, &os);
                steps[i] += 1;
            }
        }
    }

    // Record final states for satellites that finished at end_time
    // (covers the case where period doesn't align with output_interval)
    for (i, (entry, _)) in group.satellites_with_dynamics().enumerate() {
        if !entry.terminated && (entry.t - last_output_t[i]) > 1e-9 {
            let tp = TimePoint::new().with_sim_time(entry.t).with_step(steps[i]);
            let os = RecordOrbitalState::new(*entry.state.position(), *entry.state.velocity());
            rec.log_orbital_state(&sat_paths[i], &tp, &os);
        }
    }

    if let Some(monitors) = visibility.take() {
        report_contact_windows(params, monitors);
    }

    // Use first satellite for metadata (backward compatibility)
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
    rec.metadata = orts::record::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        epoch_iso: params.epoch.map(|e| e.to_datetime().to_string()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        altitude: first_sat.map(|s| s.altitude(&params.body)),
        period: first_sat.map(|s| s.period),
        orbit_description: orbit_desc,
    };

    rec
}

/// Print a Recording as CSV to stdout.
pub fn print_recording_as_csv(rec: &Recording, params: &SimParams) {
    let mut stdout = std::io::stdout().lock();
    write_recording_as_csv(&mut stdout, rec, Some(params)).unwrap();
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
fn run_controlled_simulation(params: &SimParams, sim: &SimArgs) -> Recording {
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
            orts::plugin::wasm::WasmPluginCache::new_with_async_mode(async_mode).unwrap_or_else(
                |e| {
                    eprintln!("Error initializing WASM plugin cache: {e}");
                    std::process::exit(1);
                },
            )
        }
        #[cfg(not(feature = "plugin-wasm-async"))]
        {
            orts::plugin::wasm::WasmPluginCache::new().unwrap_or_else(|e| {
                eprintln!("Error initializing WASM plugin cache: {e}");
                std::process::exit(1);
            })
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
            let sat =
                build_controlled_satellite(spec, params.epoch, &mut ctx).unwrap_or_else(|e| {
                    eprintln!("Error building controlled satellite '{}': {e}", spec.id);
                    std::process::exit(1);
                });
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
                eprintln!("command targets unknown satellite '{}'", cmd.sat);
                std::process::exit(1);
            };
            let message = cmd.to_message(idx).unwrap_or_else(|e| {
                eprintln!("invalid command: {e}");
                std::process::exit(1);
            });
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
    let dt_ctrl = satellites
        .iter()
        .map(|sat| sat.controller.sample_period())
        .fold(f64::INFINITY, f64::min);
    // `fold` also yields `INFINITY` for an empty fleet, which the loop below
    // cannot step with either.
    if let Err(e) = crate::config::validate_sample_period(dt_ctrl) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
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

        if parallel_step {
            use rayon::prelude::*;
            satellites.par_iter_mut().for_each(|sat| {
                step_controlled(sat, t, dt, dt_ode, params.epoch.as_ref()).unwrap_or_else(|e| {
                    eprintln!("Simulation error at t={t:.3}: {e}");
                    std::process::exit(1);
                });
            });
        } else {
            for sat in &mut satellites {
                step_controlled(sat, t, dt, dt_ode, params.epoch.as_ref()).unwrap_or_else(|e| {
                    eprintln!("Simulation error at t={t:.3}: {e}");
                    std::process::exit(1);
                });
            }
        }

        // FSW からの downlink（テレメトリ / ISL）を回収してログに出す。
        // controller が msg-io 未対応なら default 実装で空。
        for (i, sat) in satellites.iter_mut().enumerate() {
            for m in sat.controller.take_outbound() {
                // Metadata at info; the full payload only at debug — payloads
                // can be large (binary / file-transfer), so logging them every
                // tick at info would be noisy and IO-heavy.
                log::info!(
                    "downlink t={:.3} sat={} kind={}",
                    t + dt,
                    params.satellites[i].id,
                    m.kind
                );
                log::debug!(
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
    rec.metadata = orts::record::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        epoch_iso: params.epoch.map(|e| e.to_datetime().to_string()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        altitude: first_sat.map(|s| s.altitude(&params.body)),
        period: first_sat.map(|s| s.period),
        orbit_description: orbit_desc,
    };

    rec
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
