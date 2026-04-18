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
                    super::run::write_satellite_csv(w, &rec, sat_path, mu, multi_sat)?;
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
