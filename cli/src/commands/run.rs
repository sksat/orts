use orts_datamodel::archetypes::OrbitalState;
use orts_datamodel::components::{BodyRadius, GravitationalParameter};
use orts_datamodel::entity_path::EntityPath;
use orts_datamodel::recording::Recording;
use orts_datamodel::timeline::TimePoint;
use orts_integrator::{AdvanceOutcome, DormandPrince, IntegrationOutcome, Integrator, Rk4, State};
use orts_orbits::{events, events::SimulationEvent, kepler::KeplerianElements};

use crate::cli::{SimArgs, OutputFormat, IntegratorChoice};
use crate::satellite::OrbitSpec;
use crate::sim::params::SimParams;

pub fn run_simulation_cmd(sim: &SimArgs, output: &str, format: OutputFormat) {
    let params = SimParams::from_sim_args(sim, false);

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
            orts_datamodel::rerun_export::save_as_rrd(&rec, "orts", path)
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
    use crate::sim::core::build_orbital_system;

    let mut rec = Recording::new();
    let body_path = EntityPath::parse(&format!("/world/{}", params.body.properties().name));

    rec.log_static(&body_path, &GravitationalParameter(params.mu));
    rec.log_static(&body_path, &BodyRadius(params.body.properties().radius));

    for sat in &params.satellites {
        let system = build_orbital_system(&params.body, params.mu, params.epoch, sat, params.atmosphere, params.f107, params.ap, params.space_weather_provider.as_ref());
        let initial = sat.initial_state(params.mu);
        let sat_path = sat.entity_path();

        let mut step: u64 = 0;
        let record_state = |rec: &mut Recording, t: f64, step: u64, state: &State| {
            let tp = TimePoint::new().with_sim_time(t).with_step(step);
            let os = OrbitalState::new(state.position, state.velocity);
            rec.log_orbital_state(&sat_path, &tp, &os);
        };

        record_state(&mut rec, 0.0, step, &initial);
        step += 1;

        let mut next_output_t = params.output_interval;
        let mut last_output_t = 0.0_f64;
        let props = params.body.properties();
        let body_radius = props.radius;
        let event_checker = events::collision_check(body_radius, props.atmosphere_altitude);

        let outcome: IntegrationOutcome<State, SimulationEvent> = match params.integrator {
            IntegratorChoice::Rk4 => {
                let callback = |t: f64, state: &State| {
                    if t >= next_output_t - 1e-9 {
                        record_state(&mut rec, t, step, state);
                        step += 1;
                        last_output_t = t;
                        next_output_t += params.output_interval;
                    }
                };
                Rk4.integrate_with_events(
                    &system,
                    initial,
                    0.0,
                    sat.period,
                    params.dt,
                    callback,
                    &event_checker,
                )
            }
            IntegratorChoice::Dp45 => {
                let t_end = sat.period;
                let mut stepper = DormandPrince.stepper(
                    &system,
                    initial,
                    0.0,
                    params.dt.min(t_end),
                    params.tolerances.clone(),
                );
                stepper.dt_min = 1e-12 * t_end.abs().max(1.0);

                let mut final_outcome: IntegrationOutcome<State, SimulationEvent> =
                    IntegrationOutcome::Completed(stepper.state().clone());

                while stepper.t() < t_end {
                    let t_target = next_output_t.min(t_end);
                    match stepper.advance_to(t_target, |_, _| {}, &event_checker) {
                        Ok(AdvanceOutcome::Reached) => {
                            if stepper.t() >= next_output_t - 1e-9 {
                                record_state(&mut rec, stepper.t(), step, stepper.state());
                                step += 1;
                                last_output_t = stepper.t();
                                next_output_t += params.output_interval;
                            }
                            final_outcome =
                                IntegrationOutcome::Completed(stepper.state().clone());
                        }
                        Ok(AdvanceOutcome::Event { reason }) => {
                            let t = stepper.t();
                            final_outcome = IntegrationOutcome::Terminated {
                                state: stepper.into_state(),
                                t,
                                reason,
                            };
                            break;
                        }
                        Err(e) => {
                            final_outcome = IntegrationOutcome::Error(e);
                            break;
                        }
                    }
                }

                final_outcome
            }
        };

        match &outcome {
            IntegrationOutcome::Completed(final_state) => {
                if (sat.period - last_output_t) > 1e-9 {
                    record_state(&mut rec, sat.period, step, final_state);
                }
            }
            IntegrationOutcome::Terminated { state, t, reason } => {
                eprintln!(
                    "Simulation terminated at t={t:.2}s for {}: {reason:?}",
                    sat.id
                );
                record_state(&mut rec, *t, step, state);
            }
            IntegrationOutcome::Error(err) => {
                eprintln!(
                    "Simulation error for {}: {err:?}",
                    sat.id
                );
            }
        }
    }

    // Use first satellite for metadata (backward compatibility)
    let first_sat = params.satellites.first();
    rec.metadata = orts_datamodel::recording::SimMetadata {
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
    use orts_datamodel::component::Component;
    use orts_datamodel::components::{Position3D, Velocity3D};
    use orts_datamodel::timeline::TimelineName;

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
            Some(orts_datamodel::timeline::TimeIndex::Seconds(s)) => *s,
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
