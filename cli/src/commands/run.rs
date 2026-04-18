use std::ops::ControlFlow;

use orts::OrbitalState;
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::orbital::kepler::KeplerianElements;
use orts::record::archetypes::OrbitalState as RecordOrbitalState;
use orts::record::components::{
    AngularVelocity3D, BodyRadius, GravitationalParameter, MtqCommand3D, Quaternion4D,
    RwMomentum3D, RwTorqueCommand3D,
};
use orts::record::entity_path::EntityPath;
use orts::record::recording::Recording;
use orts::record::timeline::TimePoint;

use crate::cli::{IntegratorChoice, OutputFormat, SimArgs};
use crate::satellite::OrbitSpec;
use crate::sim::params::SimParams;

pub fn run_simulation_cmd(sim: &SimArgs, output: &str, format: OutputFormat) {
    let mut params = if let Some(config_path) = &sim.config {
        let config = crate::config::SimConfig::load(std::path::Path::new(config_path))
            .unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
        SimParams::from_config(&config)
    } else {
        SimParams::from_sim_args(sim, false)
    };
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

    match (output, format) {
        ("stdout", OutputFormat::Csv) | (_, OutputFormat::Csv) => {
            print_recording_as_csv(&rec, &params);
        }
        ("stdout", OutputFormat::Rrd) => {
            eprintln!(
                "Error: cannot write .rrd format to stdout. Use --format csv or specify a file path."
            );
            std::process::exit(1);
        }
        (path, OutputFormat::Rrd) => {
            orts::record::rerun_export::save_as_rrd(&rec, "orts", path).unwrap_or_else(|e| {
                eprintln!("Error saving .rrd: {e}");
                std::process::exit(1);
            });
            eprintln!("Saved to {path}");
        }
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
        let initial = sat.initial_state(params.mu);

        group = group.add_satellite_until(sat.id.as_str(), initial, sat.period, system);
    }

    // Record initial states
    let mut steps: Vec<u64> = vec![0; params.satellites.len()];
    let mut last_output_t: Vec<f64> = vec![0.0; params.satellites.len()];
    for (i, (entry, _)) in group.satellites_with_dynamics().enumerate() {
        let tp = TimePoint::new().with_sim_time(0.0).with_step(0);
        let os = RecordOrbitalState::new(*entry.state.position(), *entry.state.velocity());
        rec.log_orbital_state(&sat_paths[i], &tp, &os);
        steps[i] = 1;
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

        let outcome = group.propagate_to(t).unwrap();

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

    // Use first satellite for metadata (backward compatibility)
    let first_sat = params.satellites.first();
    rec.metadata = orts::record::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        altitude: first_sat.map(|s| s.altitude(&params.body)),
        period: first_sat.map(|s| s.period),
    };

    rec
}

/// Print a Recording as CSV to stdout.
pub fn print_recording_as_csv(rec: &Recording, params: &SimParams) {
    println!("# Orts 2-body orbit propagation");
    println!("# mu = {} km^3/s^2", params.mu);
    if let Some(epoch) = params.epoch {
        println!("# epoch_jd = {}", epoch.jd());
        println!("# epoch = {}", epoch.to_datetime());
    }
    println!(
        "# central_body = {}",
        params.body.properties().name.to_lowercase()
    );
    println!(
        "# central_body_radius = {} km",
        params.body.properties().radius
    );

    if params.satellites.len() == 1 {
        // Single satellite: backward-compatible CSV format (no satellite_id column)
        let sat = &params.satellites[0];
        match &sat.orbit {
            OrbitSpec::Circular { altitude, r0, .. } => {
                println!(
                    "# Initial orbit: circular at {} km altitude (r = {} km)",
                    altitude, r0
                );
            }
            OrbitSpec::Tle { tle_data, elements } => {
                println!(
                    "# Initial orbit: from TLE (a = {:.1} km, e = {:.6}, i = {:.2}°)",
                    elements.semi_major_axis,
                    elements.eccentricity,
                    elements.inclination.to_degrees()
                );
                if let Some(name) = &tle_data.name {
                    println!("# satellite = {name}");
                }
            }
        }
        println!(
            "# Period = {:.1} s ({:.1} min)",
            sat.period,
            sat.period / 60.0
        );
        let sat_path = sat.entity_path();
        println!("{}", build_csv_header(rec, &sat_path, false));
        print_satellite_csv(rec, &sat_path, params.mu, false);
    } else {
        // Multi-satellite: add satellite_id as first column
        println!(
            "# satellites = {}",
            params
                .satellites
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        // Build header from first satellite's columns (all satellites share the same schema)
        if let Some(first_sat) = params.satellites.first() {
            let sat_path = first_sat.entity_path();
            println!("{}", build_csv_header(rec, &sat_path, true));
        }

        for sat in &params.satellites {
            println!(
                "# --- {} (period = {:.1} s) ---",
                sat.name.as_deref().unwrap_or(&sat.id),
                sat.period
            );
            let sat_path = sat.entity_path();
            print_satellite_csv(rec, &sat_path, params.mu, true);
        }
    }
}

pub fn print_satellite_csv(rec: &Recording, sat_path: &EntityPath, mu: f64, with_id: bool) {
    use orts::record::component::Component;
    use orts::record::components::{Position3D, Velocity3D};
    use orts::record::timeline::TimelineName;

    let store = match rec.entity(sat_path) {
        Some(s) => s,
        None => return,
    };
    let pos_col = match store.columns.get(&Position3D::component_name()) {
        Some(c) => c,
        None => return,
    };
    let vel_col = match store.columns.get(&Velocity3D::component_name()) {
        Some(c) => c,
        None => return,
    };
    let sim_times = match store.timelines.get(&TimelineName::SimTime) {
        Some(t) => t,
        None => return,
    };

    // Collect extra columns (everything except Position3D and Velocity3D), sorted by name
    let skip = [Position3D::component_name(), Velocity3D::component_name()];
    let mut extra_cols: Vec<_> = store
        .columns
        .iter()
        .filter(|(name, _)| !skip.contains(name))
        .collect();
    extra_cols.sort_by(|(a, _), (b, _)| a.cmp(b));

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

        // Append all extra columns dynamically
        for (_name, col) in &extra_cols {
            if let Some(row) = col.get_row(i) {
                for val in row {
                    line.push_str(&format!(",{:.10}", val));
                }
            }
        }

        println!("{line}");
    }
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
            let sat = build_controlled_satellite(spec, &mut ctx).unwrap_or_else(|e| {
                eprintln!("Error building controlled satellite '{}': {e}", spec.id);
                std::process::exit(1);
            });
            satellites.push(sat);
        }
    }

    // 初期状態を記録。
    for (i, sat) in satellites.iter().enumerate() {
        let tp = TimePoint::new().with_sim_time(0.0).with_step(0);
        log_controlled_state(&mut rec, &sat_paths[i], &tp, sat);
    }

    // 全衛星の sample_period の最小値をグローバル tick に使う。
    let dt_ctrl = satellites
        .iter()
        .map(|sat| sat.controller.sample_period())
        .fold(f64::INFINITY, f64::min);
    let dt_ode = params.dt.min(dt_ctrl);

    let mut t = 0.0;
    let mut step: u64 = 1;
    let mut next_output_t = params.output_interval;
    let mut last_output_t = 0.0;

    while t < duration - 1e-12 {
        let dt = dt_ctrl.min(duration - t);

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

        t += dt;

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

    rec.metadata = orts::record::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        ..Default::default()
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
    use orts::plugin::{MtqCommand as PluginMtqCommand, RwCommand as PluginRwCommand};
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
