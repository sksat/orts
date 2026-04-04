use std::collections::BTreeSet;

use crate::cli::OutputFormat;

/// Convert an .rrd file to another format (currently CSV only).
///
/// ## CSV format
///
/// Follows the same convention as `orts run`:
///
/// - **Single entity**: `t,x,y,z,vx,vy,vz` (no entity column)
/// - **Multiple entities**: `satellite_id,t,x,y,z,vx,vy,vz` with a
///   `# satellites = id1, id2, ...` metadata header
///
/// The viewer's CSV import (`viewer/src/sources/`) uses the `# satellites`
/// header to switch between single-sat and multi-sat parsing.  If you
/// change the format here, update the viewer parser as well.
pub fn run_convert(input: &str, format: OutputFormat, output: Option<&str>) {
    match format {
        OutputFormat::Csv => {
            let data = orts::record::rerun_export::load_rrd_data(input).unwrap_or_else(|e| {
                eprintln!("Error reading {input}: {e}");
                std::process::exit(1);
            });

            // Collect unique entity paths to decide single-/multi-sat mode.
            let entities: BTreeSet<&str> = data
                .rows
                .iter()
                .filter_map(|r| r.entity_path.as_deref())
                .collect();
            let multi_sat = entities.len() > 1;

            let write_csv = |w: &mut dyn std::io::Write| -> std::io::Result<()> {
                writeln!(w, "# Converted from {input}")?;
                let meta = &data.metadata;
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

                if multi_sat {
                    writeln!(
                        w,
                        "# satellites = {}",
                        entities.iter().copied().collect::<Vec<_>>().join(", ")
                    )?;
                    writeln!(
                        w,
                        "# satellite_id,t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]"
                    )?;
                    for row in &data.rows {
                        let entity = row.entity_path.as_deref().unwrap_or("");
                        writeln!(
                            w,
                            "{},{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
                            entity, row.t, row.x, row.y, row.z, row.vx, row.vy, row.vz,
                        )?;
                    }
                } else {
                    writeln!(w, "# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]")?;
                    for row in &data.rows {
                        writeln!(
                            w,
                            "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
                            row.t, row.x, row.y, row.z, row.vx, row.vy, row.vz,
                        )?;
                    }
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
