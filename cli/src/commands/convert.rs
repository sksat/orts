use crate::cli::OutputFormat;

use super::CmdError;

/// Convert an .rrd file to another format (currently CSV only).
///
/// Uses [`load_as_recording`] to reconstruct a full [`Recording`] from the
/// .rrd file, then outputs CSV using [`write_recording_as_csv`] — the same
/// code path as `orts run --format csv`.
pub fn run_convert(
    input: &str,
    format: OutputFormat,
    output: Option<&str>,
) -> Result<(), CmdError> {
    match format {
        OutputFormat::Csv => {
            let rec = orts::record::rerun_export::load_as_recording(input)
                .map_err(|e| CmdError::failure(format!("reading {input}: {e}")))?;

            let write_csv = |w: &mut dyn std::io::Write| -> std::io::Result<()> {
                writeln!(w, "# Converted from {input}")?;
                super::run::write_recording_as_csv(w, &rec, None)
            };

            // A write that fails — a reader that closes the pipe early
            // (`orts convert … | head`), a full disk — is an error to report,
            // as `run` reports it, not a reason to panic out of the writer.
            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(path)
                        .map_err(|e| CmdError::failure(format!("creating {path}: {e}")))?;
                    write_csv(&mut file)
                        .map_err(|e| CmdError::failure(format!("writing {path}: {e}")))?;
                    eprintln!("Converted {input} -> {path}");
                }
                None => {
                    let mut stdout = std::io::stdout().lock();
                    write_csv(&mut stdout)
                        .map_err(|e| CmdError::failure(format!("writing CSV to stdout: {e}")))?;
                }
            }
            Ok(())
        }
        OutputFormat::Rrd => Err(CmdError::failure(
            "cannot convert to .rrd format (input is already .rrd)",
        )),
    }
}
