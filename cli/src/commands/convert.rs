use crate::cli::OutputFormat;

/// Convert an .rrd file to another format (currently CSV only).
///
/// Uses [`load_as_recording`] to reconstruct a full [`Recording`] from the
/// .rrd file, then outputs CSV using the same code path as `orts run --format csv`.
/// This guarantees identical CSV format between `orts run` and `orts convert`.
pub fn run_convert(input: &str, format: OutputFormat, output: Option<&str>) {
    match format {
        OutputFormat::Csv => {
            let rec = orts::record::rerun_export::load_as_recording(input).unwrap_or_else(|e| {
                eprintln!("Error reading {input}: {e}");
                std::process::exit(1);
            });

            let write_csv = |w: &mut dyn std::io::Write| -> std::io::Result<()> {
                writeln!(w, "# Converted from {input}")?;

                // Write metadata from Recording
                let meta = &rec.metadata;
                if let Some(mu) = meta.mu {
                    writeln!(w, "# mu = {} km^3/s^2", mu)?;
                }
                if let Some(epoch_jd) = meta.epoch_jd {
                    writeln!(w, "# epoch_jd = {}", epoch_jd)?;
                }
                if let Some(ref name) = meta.body_name {
                    writeln!(w, "# central_body = {}", name.to_lowercase())?;
                }
                if let Some(radius) = meta.body_radius {
                    writeln!(w, "# central_body_radius = {} km", radius)?;
                }

                // Find satellite entities (under /world/sat/)
                use orts::record::entity_path::EntityPath;
                let sat_prefix = EntityPath::parse("/world/sat");
                let mut sat_paths = rec.entities_under(&sat_prefix);
                sat_paths.sort_by_key(|p| p.to_string());

                let mu = meta.mu.unwrap_or(398600.4418);
                let multi_sat = sat_paths.len() > 1;

                if multi_sat {
                    let id_strings: Vec<String> = sat_paths
                        .iter()
                        .map(|p| {
                            p.to_string()
                                .rsplit('/')
                                .next()
                                .unwrap_or("default")
                                .to_string()
                        })
                        .collect();
                    writeln!(w, "# satellites = {}", id_strings.join(", "))?;
                }

                // Use the shared CSV output functions
                if let Some(first) = sat_paths.first() {
                    let header = super::run::build_csv_header(&rec, first, multi_sat);
                    writeln!(w, "{header}")?;
                }

                for sat_path in &sat_paths {
                    if multi_sat {
                        let id = sat_path
                            .to_string()
                            .rsplit('/')
                            .next()
                            .unwrap_or("default")
                            .to_string();
                        writeln!(w, "# --- {id} ---")?;
                    }
                    // Capture output to writer instead of stdout
                    print_satellite_csv_to(w, &rec, sat_path, mu, multi_sat)?;
                }

                Ok(())
            };

            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(path).unwrap_or_else(|e| {
                        eprintln!("Error creating {path}: {e}");
                        std::process::exit(1);
                    });
                    write_csv(&mut file).unwrap();
                    eprintln!("Converted {input} -> {path}");
                }
                None => {
                    let mut stdout = std::io::stdout().lock();
                    write_csv(&mut stdout).unwrap();
                }
            }
        }
        OutputFormat::Rrd => {
            eprintln!("Error: cannot convert to .rrd format (input is already .rrd)");
            std::process::exit(1);
        }
    }
}

/// Write satellite CSV data to any writer (shared logic with print_satellite_csv).
fn print_satellite_csv_to(
    w: &mut dyn std::io::Write,
    rec: &orts::record::recording::Recording,
    sat_path: &orts::record::entity_path::EntityPath,
    mu: f64,
    with_id: bool,
) -> std::io::Result<()> {
    use orts::orbital::kepler::KeplerianElements;
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
