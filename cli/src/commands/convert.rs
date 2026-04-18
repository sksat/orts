use crate::cli::OutputFormat;

/// Convert an .rrd file to another format (currently CSV only).
///
/// Uses [`load_as_recording`] to reconstruct a full [`Recording`] from the
/// .rrd file, then outputs CSV using [`write_recording_as_csv`] — the same
/// code path as `orts run --format csv`.
pub fn run_convert(input: &str, format: OutputFormat, output: Option<&str>) {
    match format {
        OutputFormat::Csv => {
            let rec = orts::record::rerun_export::load_as_recording(input).unwrap_or_else(|e| {
                eprintln!("Error reading {input}: {e}");
                std::process::exit(1);
            });

            let write_csv = |w: &mut dyn std::io::Write| -> std::io::Result<()> {
                writeln!(w, "# Converted from {input}")?;
                super::run::write_recording_as_csv(w, &rec, None)
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
