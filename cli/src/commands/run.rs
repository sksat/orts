use std::ops::ControlFlow;

use orts::OrbitalState;
use orts::kepler::KeplerianElements;
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::record::archetypes::OrbitalState as RecordOrbitalState;
use orts::record::components::{BodyRadius, GravitationalParameter};
use orts::record::entity_path::EntityPath;
use orts::record::recording::Recording;
use orts::record::timeline::TimePoint;

use crate::cli::{IntegratorChoice, OutputFormat, SimArgs};
use crate::satellite::OrbitSpec;
use crate::sim::params::SimParams;

pub fn run_simulation_cmd(sim: &SimArgs, output: &str, format: OutputFormat) {
    let params = if let Some(config_path) = &sim.config {
        let config = crate::config::SimConfig::load(std::path::Path::new(config_path))
            .unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
        SimParams::from_config(&config)
    } else {
        SimParams::from_sim_args(sim, false)
    };

    // Determine effective format: stdout defaults to csv if format not explicitly set.
    let rec = run_simulation(&params);

    match (output, format) {
        ("stdout", OutputFormat::Csv) | (_, OutputFormat::Csv) => {
            print_recording_as_csv(&rec, &params);
        }
        ("stdout", OutputFormat::Rrd) => {
            eprintln!("Error: cannot write .rrd format to stdout. Use --format csv or specify a file path.");
            std::process::exit(1);
        }
        (path, OutputFormat::Rrd) => {
            orts::record::rerun_export::save_as_rrd(&rec, "orts", path)
                .unwrap_or_else(|e| {
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
    use orts::setup::build_orbital_system;

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

    for sat in &params.satellites {
        let system = build_orbital_system(
            &params.body,
            params.mu,
            params.epoch,
            &sat_params(sat),
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
                let tp = TimePoint::new()
                    .with_sim_time(entry.t)
                    .with_step(steps[i]);
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
                let tp = TimePoint::new()
                    .with_sim_time(entry.t)
                    .with_step(steps[i]);
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
            let tp = TimePoint::new()
                .with_sim_time(entry.t)
                .with_step(steps[i]);
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
                println!("# Initial orbit: circular at {} km altitude (r = {} km)", altitude, r0);
            }
            OrbitSpec::Tle { tle_data, elements } => {
                println!(
                    "# Initial orbit: from TLE (a = {:.1} km, e = {:.6}, i = {:.2}°)",
                    elements.semi_major_axis, elements.eccentricity, elements.inclination.to_degrees()
                );
                if let Some(name) = &tle_data.name {
                    println!("# satellite = {name}");
                }
            }
        }
        println!("# Period = {:.1} s ({:.1} min)", sat.period, sat.period / 60.0);
        println!("# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s],a[km],e[-],i[rad],raan[rad],omega[rad],nu[rad]");

        let sat_path = sat.entity_path();
        print_satellite_csv(rec, &sat_path, params.mu, false);
    } else {
        // Multi-satellite: add satellite_id as first column
        println!("# satellites = {}", params.satellites.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", "));
        println!("# satellite_id,t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s],a[km],e[-],i[rad],raan[rad],omega[rad],nu[rad]");

        for sat in &params.satellites {
            println!("# --- {} (period = {:.1} s) ---", sat.name.as_deref().unwrap_or(&sat.id), sat.period);
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

    // Extract satellite id from path (last segment)
    let id = sat_path.to_string();
    let id = id.rsplit('/').next().unwrap_or("default");

    for i in 0..pos_col.num_rows() {
        let t = match sim_times.get(i * 2) {
            Some(orts::record::timeline::TimeIndex::Seconds(s)) => *s,
            _ => 0.0,
        };
        let pos = pos_col.get_row(i).unwrap();
        let vel = vel_col.get_row(i).unwrap();
        let pos_vec = nalgebra::Vector3::new(pos[0], pos[1], pos[2]);
        let vel_vec = nalgebra::Vector3::new(vel[0], vel[1], vel[2]);
        let elements = KeplerianElements::from_state_vector(&pos_vec, &vel_vec, mu);
        if with_id {
            println!(
                "{},{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.10},{:.10},{:.10},{:.10},{:.10}",
                id, t, pos[0], pos[1], pos[2], vel[0], vel[1], vel[2],
                elements.semi_major_axis, elements.eccentricity,
                elements.inclination, elements.raan,
                elements.argument_of_periapsis, elements.true_anomaly,
            );
        } else {
            println!(
                "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.10},{:.10},{:.10},{:.10},{:.10}",
                t, pos[0], pos[1], pos[2], vel[0], vel[1], vel[2],
                elements.semi_major_axis, elements.eccentricity,
                elements.inclination, elements.raan,
                elements.argument_of_periapsis, elements.true_anomaly,
            );
        }
    }
}
