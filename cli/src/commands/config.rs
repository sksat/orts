//! `orts config` — inspect and validate simulation config files.
//!
//! `example` prints a ready-to-edit config; `validate` checks a config and
//! reports the verdict (human-readable on stderr, or machine-readable JSON on
//! stdout with `--json`). Both steer users and coding agents toward the config
//! file as the canonical, multi-satellite-capable input (vs. the `--sat`
//! shorthand).

use crate::cli::{ConfigCommands, ConfigFormat};
use crate::config::SimConfig;

/// Curated, valid example config in TOML. This is the single source of truth
/// for `config example`: the JSON and YAML forms are produced by parsing this
/// and re-serializing, so all three formats stay in sync and always validate.
const EXAMPLE_TOML: &str = r#"# Example orts simulation config.
# Validate with: orts config validate <file>
body = "earth"                    # central body: earth, moon, mars, sun, ...
dt = 10.0                         # integration step [s]
epoch = "2026-01-01T00:00:00Z"    # ISO 8601 UTC; omit to use the current time
duration = 5400.0                 # total sim time [s]; omit for one orbital period
output_interval = 10.0            # recording cadence [s]; defaults to dt
atmosphere = "exponential"        # exponential | harris-priester | nrlmsise00

[integrator]
type = "dp45"                     # rk4 | dp45 | dop853
atol = 1.0e-10                    # adaptive integrators only
rtol = 1.0e-8

# Full spherical-harmonic gravity (Earth only). Replaces the J2/J3/J4 zonal
# model and sets mu to the file's GM. Uncomment and point at an ICGEM .gfc.
# [gravity_field]
# path = "EGM2008.gfc"            # ICGEM gfc, static coefficients
# degree = 70                     # truncation; omit for the file's maximum
# order = 70                      # defaults to degree

[[satellites]]
id = "sat-0"

[satellites.orbit]
type = "circular"                 # circular | tle | norad
altitude = 500.0                  # [km]
inclination = 51.6                # [deg]
raan = 0.0                        # [deg]
"#;

pub fn run_config(cmd: ConfigCommands) {
    match cmd {
        ConfigCommands::Example { format } => run_example(format),
        ConfigCommands::Validate { path, json } => run_validate(&path, json),
    }
}

fn run_example(format: ConfigFormat) {
    match format {
        // Print the curated source verbatim so its comments are preserved.
        ConfigFormat::Toml => print!("{EXAMPLE_TOML}"),
        ConfigFormat::Json | ConfigFormat::Yaml => {
            let config: SimConfig =
                toml::from_str(EXAMPLE_TOML).expect("built-in example config must parse");
            let rendered = match format {
                ConfigFormat::Json => {
                    serde_json::to_string_pretty(&config).expect("serialize example to JSON")
                }
                ConfigFormat::Yaml => {
                    serde_yaml::to_string(&config).expect("serialize example to YAML")
                }
                ConfigFormat::Toml => unreachable!(),
            };
            // serde_json has no trailing newline; serde_yaml already ends with
            // one — print! keeps the YAML output free of a blank trailing line.
            print!("{rendered}");
            if !rendered.ends_with('\n') {
                println!();
            }
        }
    }
}

fn run_validate(path: &str, json: bool) {
    match SimConfig::load_with_warnings(std::path::Path::new(path)) {
        Ok(loaded) => {
            // A key nothing read is a warning, not a failure: a config written
            // for a newer `orts` still runs here. Naming the paths is what
            // makes a typo findable, since the value it carried is gone.
            // The JSON form carries the key as it was written — a JSON string
            // encodes a newline or an escape character itself. The line printed
            // to a terminal goes through `printable_key`, like every other
            // warning naming a key.
            let warnings: Vec<String> = loaded
                .unread_keys
                .iter()
                .map(|key| format!("nothing reads `{key}`; its value is ignored"))
                .collect();
            if json {
                emit_verdict(serde_json::json!({
                    "schema": "orts.config-validate/v1",
                    "status": "ok",
                    "path": path,
                    "warnings": warnings,
                }));
            } else {
                for key in &loaded.unread_keys {
                    log::warn!(
                        "{path}: nothing reads `{}`; its value is ignored",
                        crate::config::printable_key(key)
                    );
                }
                eprintln!("OK: {path} is a valid orts config");
            }
        }
        Err(e) => {
            if json {
                emit_verdict(serde_json::json!({
                    "schema": "orts.config-validate/v1",
                    "status": "error",
                    "path": path,
                    "error": e,
                }));
            } else {
                eprintln!("Error: {path} is not a valid orts config: {e}");
            }
            std::process::exit(2);
        }
    }
}

fn emit_verdict(value: serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("serialize validate verdict")
    );
}
