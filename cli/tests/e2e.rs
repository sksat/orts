use std::process::{Command, Stdio};

fn run_cli_csv() -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts");
    Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts")
}

fn run_cli_csv_with_body(body: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts");
    Command::new(binary)
        .args([
            "run",
            "--body",
            body,
            "--sat",
            "altitude=400",
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts")
}

#[test]
fn test_cli_runs_successfully() {
    let output = run_cli_csv();
    assert!(
        output.status.success(),
        "CLI exited with non-zero status: {:?}",
        output.status
    );
}

#[test]
fn test_cli_output_has_header() {
    let output = run_cli_csv();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# orts simulation"));
    assert!(stdout.contains("# mu ="));
    assert!(stdout.contains("# t[s],x[km]"));
}

#[test]
fn test_cli_output_is_csv() {
    let output = run_cli_csv();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();

    assert!(
        data_lines.len() > 10,
        "Expected many data lines, got {}",
        data_lines.len()
    );

    // Each data line should have 13 comma-separated fields
    // (t, x, y, z, vx, vy, vz, a, e, i, raan, omega, nu)
    for line in &data_lines {
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(
            fields.len(),
            13,
            "Expected 13 fields in CSV line, got {}: '{}'",
            fields.len(),
            line
        );
        // Each field should be a valid f64
        for field in &fields {
            field.trim().parse::<f64>().unwrap_or_else(|_| {
                panic!("Field '{}' in line '{}' is not a valid f64", field, line)
            });
        }
    }
}

#[test]
fn test_cli_point_mass_orbit_closes() {
    // Use Sun as central body (no J2) → pure point-mass → orbit should close
    let output = run_cli_csv_with_body("sun");
    let stdout = String::from_utf8_lossy(&output.stdout);

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();

    let first = parse_csv_line(data_lines[0]);
    let last = parse_csv_line(data_lines[data_lines.len() - 1]);

    let dx = first.1 - last.1;
    let dy = first.2 - last.2;
    let dz = first.3 - last.3;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();

    // For point-mass, orbit should close within numerical precision
    let r_first = (first.1 * first.1 + first.2 * first.2 + first.3 * first.3).sqrt();
    let rel_distance = distance / r_first;
    assert!(
        rel_distance < 1e-3,
        "Point-mass orbit did not close: distance = {distance:.6} km (rel = {rel_distance:.2e})"
    );
}

#[test]
fn test_cli_j2_orbit_drifts() {
    // Earth has J2 enabled by default → orbit should not close exactly
    let output = run_cli_csv();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();

    let first = parse_csv_line(data_lines[0]);
    let last = parse_csv_line(data_lines[data_lines.len() - 1]);

    let dx = first.1 - last.1;
    let dy = first.2 - last.2;
    let dz = first.3 - last.3;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();

    // J2 causes measurable drift (~100-200 km per orbit at ISS altitude)
    assert!(
        distance > 1.0,
        "J2 should cause measurable orbit drift, but distance = {distance:.6} km"
    );
    assert!(
        distance < 300.0,
        "Orbit drifted too far: distance = {distance:.6} km"
    );

    // Altitude should be roughly preserved (J2 is conservative)
    let r_first = (first.1 * first.1 + first.2 * first.2 + first.3 * first.3).sqrt();
    let r_last = (last.1 * last.1 + last.2 * last.2 + last.3 * last.3).sqrt();
    let r_diff = (r_first - r_last).abs();
    assert!(
        r_diff < 10.0,
        "Orbital radius changed too much: |r_first - r_last| = {r_diff:.6} km"
    );
}

#[test]
fn test_cli_config_file() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("test.json");
    std::fs::write(
        &config_path,
        r#"{
            "body": "earth",
            "dt": 10.0,
            "satellites": [
                { "id": "test", "orbit": { "type": "circular", "altitude": 400.0 } }
            ]
        }"#,
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());

    assert!(
        output.status.success(),
        "CLI exited with non-zero status: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("# central_body = earth"),
        "Header should specify central body"
    );
    assert!(
        stdout.contains("circular at 400 km"),
        "Header should describe orbit"
    );

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert!(data_lines.len() > 10, "Expected many data lines");
    assert_eq!(
        data_lines[0].split(',').count(),
        13,
        "Expected 13 CSV fields"
    );

    // Invariant: orbital radius ≈ 6778 km (Earth radius 6378 + 400 km altitude)
    let mut prev_t = f64::NEG_INFINITY;
    for line in &data_lines {
        let (t, x, y, z, _, _, _) = parse_csv_line(line);
        let r = (x * x + y * y + z * z).sqrt();
        assert!(r.is_finite(), "Non-finite radius at t={t}");
        assert!(t > prev_t, "Time not monotonically increasing at t={t}");
        prev_t = t;
        // J2 causes oscillations but radius stays within ~50 km of nominal (R_earth=6378.137)
        assert!(
            (r - 6778.0).abs() < 50.0,
            "Orbital radius {r:.1} km out of range at t={t}"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

fn run_cli_with_config(config_path: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts");
    Command::new(binary)
        .args([
            "run",
            "--config",
            config_path,
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts")
}

#[test]
fn test_cli_no_config_no_orbit_args_errors() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let dir = std::env::temp_dir().join(format!("orts-e2e-noconfig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let output = Command::new(binary)
        .current_dir(&dir)
        .args(["run", "--output", "stdout", "--format", "csv"])
        .output()
        .expect("failed to execute orts");

    assert!(
        !output.status.success(),
        "Expected non-zero exit when no config or orbit args are provided"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no simulation configuration found"),
        "Expected error message about missing configuration, got: {stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Parse a CSV data line into (t, x, y, z, vx, vy, vz)
fn parse_csv_line(line: &str) -> (f64, f64, f64, f64, f64, f64, f64) {
    let fields: Vec<f64> = line.split(',').map(|f| f.trim().parse().unwrap()).collect();
    (
        fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6],
    )
}

#[test]
fn test_cli_tle_from_stdin() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let tle_text = "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996\n\
                    2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008\n";

    use std::io::Write;
    let mut child = Command::new(binary)
        .args(["run", "--tle", "-", "--output", "stdout", "--format", "csv"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn orts");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(tle_text.as_bytes())
        .expect("failed to write TLE to stdin");

    let output = child.wait_with_output().expect("failed to wait for child");
    assert!(
        output.status.success(),
        "CLI exited with non-zero status: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain element-set orbit info
    assert!(
        stdout.contains("from TLE/OMM"),
        "Missing TLE/OMM header in: {}",
        stdout.lines().take(10).collect::<Vec<_>>().join("\n")
    );
    // Should produce CSV data with 13 fields
    let data_lines: Vec<&str> = stdout.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(
        data_lines.len() > 10,
        "Expected many data lines, got {}",
        data_lines.len()
    );
    assert_eq!(
        data_lines[0].split(',').count(),
        13,
        "Expected 13 CSV fields"
    );
}

#[test]
fn test_cli_config_file_multi_satellite() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-config-multi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("multi.json");
    std::fs::write(
        &config_path,
        r#"{
            "body": "earth",
            "dt": 10.0,
            "satellites": [
                { "id": "iss", "orbit": { "type": "circular", "altitude": 400.0, "inclination": 51.6 } },
                { "id": "sso", "orbit": { "type": "circular", "altitude": 800.0, "inclination": 98.6 } }
            ]
        }"#,
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    assert!(
        output.status.success(),
        "CLI exited with non-zero status: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("# satellites = iss, sso"),
        "Header should list both satellites"
    );

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert!(data_lines.len() > 10, "Expected many data lines");
    // Multi-satellite format: satellite_id + 13 = 14 columns
    assert_eq!(
        data_lines[0].split(',').count(),
        14,
        "Expected 14 CSV fields (satellite_id + 13)"
    );

    // Check both satellites appear and have plausible radii
    let mut iss_count = 0;
    let mut sso_count = 0;
    for line in &data_lines {
        let fields: Vec<&str> = line.split(',').collect();
        let sat_id = fields[0].trim();
        // Fields 1..=7 are t, x, y, z, vx, vy, vz
        let x: f64 = fields[2].trim().parse().unwrap();
        let y: f64 = fields[3].trim().parse().unwrap();
        let z: f64 = fields[4].trim().parse().unwrap();
        let r = (x * x + y * y + z * z).sqrt();
        match sat_id {
            "iss" => {
                iss_count += 1;
                assert!(
                    (r - 6778.0).abs() < 50.0,
                    "ISS radius {r:.1} km out of range"
                );
            }
            "sso" => {
                sso_count += 1;
                assert!(
                    (r - 7178.0).abs() < 50.0,
                    "SSO radius {r:.1} km out of range"
                );
            }
            _ => panic!("Unexpected satellite_id: {sat_id}"),
        }
    }
    assert!(iss_count > 5, "Expected ISS data rows, got {iss_count}");
    assert!(sso_count > 5, "Expected SSO data rows, got {sso_count}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_cli_config_file_toml() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-config-toml-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("test.toml");
    std::fs::write(
        &config_path,
        r#"
body = "earth"
dt = 10.0

[[satellites]]
id = "test"

[satellites.orbit]
type = "circular"
altitude = 400.0
inclination = 51.6
"#,
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    assert!(
        output.status.success(),
        "CLI exited with non-zero status: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert!(data_lines.len() > 10, "Expected many data lines");

    // Check altitude from first data line
    let (_, x, y, z, _, _, _) = parse_csv_line(data_lines[0]);
    let r = (x * x + y * y + z * z).sqrt();
    assert!(
        (r - 6778.0).abs() < 50.0,
        "TOML config: orbital radius {r:.1} km out of range"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_cli_config_file_mars() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-config-mars-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("mars.json");
    std::fs::write(
        &config_path,
        r#"{
            "body": "mars",
            "dt": 10.0,
            "satellites": [
                { "id": "mro", "orbit": { "type": "circular", "altitude": 300.0 } }
            ]
        }"#,
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    assert!(
        output.status.success(),
        "CLI exited with non-zero status: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("# central_body = mars"),
        "Header should specify Mars as central body"
    );

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert!(data_lines.len() > 10, "Expected many data lines");

    // Mars radius = 3396.2 km, altitude 300 km → r ≈ 3696.2 km
    for line in &data_lines {
        let (t, x, y, z, _, _, _) = parse_csv_line(line);
        let r = (x * x + y * y + z * z).sqrt();
        assert!(r.is_finite(), "Non-finite radius at t={t}");
        assert!(
            (r - 3696.0).abs() < 50.0,
            "Mars orbital radius {r:.1} km out of range at t={t}"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

// Plugin-controlled simulation via config file

/// Run `orts run --config <path> --format csv` and return stdout.
#[cfg(feature = "plugin-wasm")]
fn run_cli_config_csv(config_path: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts");
    let plugin_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(
            std::path::Path::new(config_path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        );
    let config_name = std::path::Path::new(config_path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    Command::new(binary)
        .current_dir(plugin_dir)
        .args([
            "run",
            "--config",
            config_name,
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts")
}

/// E2E: `orts run --config mission.yaml` runs PD+RW controlled simulation.
///
/// Soft-skips when the guest WASM is not built — CI's
/// `cli-plugin-backend-e2e` and `rust-test-plugin-wasm` jobs build
/// the guest explicitly, while the plain `rust-test` job does not.
#[test]
#[cfg(feature = "plugin-wasm")]
fn test_controlled_simulation_via_config() {
    let guest_wasm = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm");
    if !guest_wasm.exists() {
        eprintln!(
            "WASM not found: {}\n\
             Build: cd plugin-sdk/examples && cargo +1.91.0 component build -p orts-example-plugin-pd-rw-control --release\n\
             Skipping this test.",
            guest_wasm.display()
        );
        return;
    }

    let output = run_cli_config_csv("plugin-sdk/examples/pd-rw-control/orts.toml");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "CLI failed: {stderr}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();

    // 120s / 1.0s output_interval ≈ 121 lines (including t=0).
    assert!(
        data_lines.len() > 100,
        "Expected >100 data lines, got {}",
        data_lines.len()
    );

    // Each line should have CSV fields (at least 7: t, x, y, z, vx, vy, vz).
    for line in &data_lines[..3] {
        let fields: Vec<&str> = line.split(',').collect();
        assert!(
            fields.len() >= 7,
            "Expected >=7 CSV fields, got {}: {line}",
            fields.len()
        );
    }

    // Verify the orbit stays reasonable (LEO, ~6778 km radius).
    let last_line = data_lines.last().unwrap();
    let fields: Vec<f64> = last_line
        .split(',')
        .take(4)
        .map(|s| s.parse().unwrap())
        .collect();
    let r = (fields[1] * fields[1] + fields[2] * fields[2] + fields[3] * fields[3]).sqrt();
    assert!(
        r > 6700.0 && r < 6900.0,
        "Final orbital radius {r:.1} km out of LEO range"
    );
}

/// Verify `orts run → rrd → orts convert --format csv` produces the same
/// metadata headers as `orts run --format csv` (except the source comment).
#[test]
fn test_csv_convert_roundtrip_headers_match() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let rrd_path = std::env::temp_dir().join("test_e2e_roundtrip.rrd");

    let epoch = "2026-01-01T00:00:00Z";

    // 1. run → csv (direct)
    let run_output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--epoch",
            epoch,
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to run orts run --format csv");
    assert!(run_output.status.success());
    let run_csv = String::from_utf8_lossy(&run_output.stdout);

    // 2. run → rrd
    let rrd_output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--epoch",
            epoch,
            "--output",
            rrd_path.to_str().unwrap(),
        ])
        .stderr(Stdio::null())
        .output()
        .expect("failed to run orts run --output rrd");
    assert!(rrd_output.status.success());

    // 3. convert rrd → csv
    let convert_output = Command::new(binary)
        .args(["convert", rrd_path.to_str().unwrap(), "--format", "csv"])
        .output()
        .expect("failed to run orts convert");
    assert!(convert_output.status.success());
    let convert_csv = String::from_utf8_lossy(&convert_output.stdout);

    // Compare metadata headers (skip source-specific lines)
    let run_headers: Vec<&str> = run_csv
        .lines()
        .filter(|l| l.starts_with('#') && !l.starts_with("# Converted"))
        .collect();
    let convert_headers: Vec<&str> = convert_csv
        .lines()
        .filter(|l| l.starts_with('#') && !l.starts_with("# Converted"))
        .collect();
    assert_eq!(
        run_headers, convert_headers,
        "CSV metadata headers differ between run and convert:\nrun: {run_headers:#?}\nconvert: {convert_headers:#?}"
    );

    // Compare data row count
    let run_data: Vec<&str> = run_csv.lines().filter(|l| !l.starts_with('#')).collect();
    let convert_data: Vec<&str> = convert_csv
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect();
    assert_eq!(
        run_data.len(),
        convert_data.len(),
        "Data row count mismatch: run={} convert={}",
        run_data.len(),
        convert_data.len()
    );

    let _ = std::fs::remove_file(&rrd_path);
}

// --- Agent-friendly output contract ---------------------------------------
// stdout carries exactly one of: simulation data XOR a JSON run summary.
// Logs/diagnostics go to stderr. `--output -` (and the legacy "stdout"
// alias) select stdout; any other value is a file path.

/// Bug fix: `orts run --format csv --output <path>` must write the CSV to the
/// file, not stdout. Previously every `--format csv` invocation went to stdout
/// regardless of `--output`, silently ignoring the path.
#[test]
fn test_cli_csv_output_to_file() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let dir = std::env::temp_dir().join(format!("orts-e2e-csv-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv_path = dir.join("out.csv");

    let output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--format",
            "csv",
            "--output",
            csv_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute orts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Data went to the file, so stdout must be empty.
    assert!(
        output.stdout.is_empty(),
        "stdout should be empty when writing CSV to a file, got {} bytes",
        output.stdout.len()
    );
    let contents = std::fs::read_to_string(&csv_path).expect("CSV file was not created");
    assert!(
        contents.contains("# orts simulation"),
        "CSV file missing metadata header"
    );
    let data_lines: Vec<&str> = contents.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(
        data_lines.len() > 10,
        "expected many data lines in file, got {}",
        data_lines.len()
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// `--output -` is the canonical stdout sentinel.
#[test]
fn test_cli_csv_output_dash_is_stdout() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--format",
            "csv",
            "--output",
            "-",
        ])
        .output()
        .expect("failed to execute orts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("# orts simulation"),
        "expected CSV on stdout for `--output -`"
    );
}

/// `orts run --json` emits a machine-readable run summary on stdout while the
/// recording data is written to the `--output` file.
#[test]
fn test_cli_run_json_summary() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let dir = std::env::temp_dir().join(format!("orts-e2e-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv_path = dir.join("data.csv");

    let output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--epoch",
            "2026-01-01T00:00:00Z",
            "--json",
            "--format",
            "csv",
            "--output",
            csv_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute orts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout={stdout}"));

    assert_eq!(v["status"], "ok");
    assert_eq!(v["command"], "run");
    assert_eq!(v["simulation"]["body"], "earth");
    assert_eq!(v["simulation"]["epoch"], "2026-01-01T00:00:00Z");

    let sat0 = &v["satellites"][0];
    assert!(
        sat0["samples"].as_u64().unwrap() > 10,
        "expected many samples"
    );
    let pos = sat0["final"]["position_km"]
        .as_array()
        .expect("final.position_km should be an array");
    assert_eq!(pos.len(), 3);
    let r = (pos[0].as_f64().unwrap().powi(2)
        + pos[1].as_f64().unwrap().powi(2)
        + pos[2].as_f64().unwrap().powi(2))
    .sqrt();
    assert!(
        (r - 6778.0).abs() < 50.0,
        "final radius {r:.1} km out of range"
    );

    let artifact = &v["artifacts"][0];
    assert_eq!(artifact["format"], "csv");
    assert!(
        artifact["path"].as_str().unwrap().ends_with("data.csv"),
        "artifact path should point to the CSV file"
    );

    assert!(csv_path.exists(), "CSV data file should exist");

    std::fs::remove_dir_all(&dir).ok();
}

/// `--json` plus simulation data destined for stdout is a usage error: both
/// would collide on stdout. (`--format csv --output -` sends CSV to stdout.)
#[test]
fn test_cli_json_and_data_stdout_conflict() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400",
            "--json",
            "--format",
            "csv",
            "--output",
            "-",
        ])
        .output()
        .expect("failed to execute orts");

    assert!(
        !output.status.success(),
        "expected non-zero exit for --json with data on stdout"
    );
    assert!(
        output.stdout.is_empty(),
        "stdout should be empty on usage error"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("stdout"),
        "error should mention the stdout conflict, got: {stderr}"
    );
}

/// The circular orbit 400 km up: `2π√(r³/μ)` at r = 6778.137 km.
const PERIOD_400KM_S: f64 = 5553.624;
/// The circular orbit 800 km up.
const PERIOD_800KM_S: f64 = 6052.414;

/// Return the last `t` reached in each satellite's CSV section.
///
/// Reads the section markers rather than the rows, because the single- and
/// multi-satellite CSVs differ in shape: only the multi-satellite one prefixes
/// each row with the id.
fn final_t_per_section(csv: &str) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    let mut multi = false;
    for line in csv.lines() {
        if let Some(rest) = line.strip_prefix("# --- ") {
            let id = rest.trim_end_matches(" ---").to_string();
            out.push((id, f64::NAN));
            continue;
        }
        if line.starts_with("# satellite_id,") {
            multi = true;
            continue;
        }
        if line.starts_with('#') || line.trim().is_empty() || line.starts_with("t[") {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        let t: f64 = cols[usize::from(multi)]
            .parse()
            .expect("a numeric t column");
        match out.last_mut() {
            Some(entry) => entry.1 = t,
            None => out.push((String::from("(single)"), t)),
        }
    }
    out
}

fn run_csv(args: &[&str]) -> String {
    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args(["run", "--output", "stdout", "--format", "csv"])
        .args(args)
        .output()
        .expect("failed to execute orts");
    assert!(
        output.status.success(),
        "orts run {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `--duration` sets how long the run covers; the reported period stays the
/// orbit's own.
///
/// The two used to share one field, so `--duration 120` made the CSV header
/// (and the RRD `meta/sim/period`, and the WebSocket `SatelliteInfo`) call
/// 120 s the orbital period of an orbit that takes 5553.6 s.
#[test]
fn duration_sets_the_end_time_and_leaves_the_period_alone() {
    let csv = run_csv(&["--sat", "altitude=400", "--duration", "120"]);

    assert!(
        csv.contains(&format!("# Period = {PERIOD_400KM_S:.1} s")),
        "header should report the orbital period, got:\n{}",
        csv.lines().take(10).collect::<Vec<_>>().join("\n")
    );

    let finals = final_t_per_section(&csv);
    assert_eq!(finals.len(), 1, "one satellite, one section: {finals:?}");
    assert!(
        (finals[0].1 - 120.0).abs() < 1e-9,
        "the run should still end at the requested duration, got {finals:?}"
    );
}

/// A satellite with an attitude config ends at `duration` too.
///
/// The spacecraft path is a separate `add_satellite_until` call from the
/// orbit-only one, so it took its end time from a separate expression.
/// Measured: putting `sat.period` back there left every other test passing,
/// because the E2E cases above are orbit-only and the `SimParams` tests stop
/// before propagation.
#[test]
fn duration_sets_the_end_time_for_a_spacecraft_run() {
    let dir = std::env::temp_dir().join(format!("orts_e2e_att_dur_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let config_path = dir.join("attitude_duration.toml");
    std::fs::write(
        &config_path,
        r#"
dt = 1.0
duration = 120.0
output_interval = 30.0

[[satellites]]
id = "att"
[satellites.orbit]
type = "circular"
altitude = 400.0

[satellites.attitude]
inertia_diag = [10.0, 10.0, 10.0]
mass = 100.0
initial_angular_velocity = [0.0, 0.0, 0.01]
"#,
    )
    .expect("write config");

    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--output",
            "stdout",
            "--format",
            "csv",
            "--config",
            config_path.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("failed to execute orts");
    assert!(
        output.status.success(),
        "orts run --config failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let csv = String::from_utf8_lossy(&output.stdout).into_owned();

    // The attitude columns are what make this the spacecraft path.
    assert!(
        csv.contains("qw") || csv.contains("quaternion"),
        "the run should carry attitude columns, got header:\n{}",
        csv.lines().take(6).collect::<Vec<_>>().join("\n")
    );
    assert!(
        csv.contains(&format!("# Period = {PERIOD_400KM_S:.1} s")),
        "header should report the orbital period, got:\n{}",
        csv.lines().take(10).collect::<Vec<_>>().join("\n")
    );

    let finals = final_t_per_section(&csv);
    assert_eq!(finals.len(), 1, "one satellite, one section: {finals:?}");
    assert!(
        (finals[0].1 - 120.0).abs() < 1e-9,
        "the spacecraft run should end at the requested duration, got {finals:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Without `--duration`, each satellite is propagated for one of its own
/// orbits — which is what made the shared field look consistent.
#[test]
fn a_fleet_without_duration_ends_each_satellite_at_its_own_period() {
    let csv = run_csv(&[
        "--sat",
        "altitude=400,id=low",
        "--sat",
        "altitude=800,id=high",
    ]);

    let finals = final_t_per_section(&csv);
    assert_eq!(finals.len(), 2, "two sections expected: {finals:?}");
    let low = finals.iter().find(|(id, _)| id == "low").expect("low");
    let high = finals.iter().find(|(id, _)| id == "high").expect("high");
    assert!(
        (low.1 - PERIOD_400KM_S).abs() < 1.0,
        "low should end at its own period, got {}",
        low.1
    );
    assert!(
        (high.1 - PERIOD_800KM_S).abs() < 1.0,
        "high should end at its own period, got {}",
        high.1
    );
}

/// `--duration` is a property of the run, so it applies to the whole fleet
/// while each satellite keeps its own period.
#[test]
fn duration_applies_to_every_satellite_in_a_fleet() {
    let csv = run_csv(&[
        "--sat",
        "altitude=400,id=low",
        "--sat",
        "altitude=800,id=high",
        "--duration",
        "120",
    ]);

    let finals = final_t_per_section(&csv);
    assert_eq!(finals.len(), 2, "two sections expected: {finals:?}");
    for (id, t) in &finals {
        assert!(
            (t - 120.0).abs() < 1e-9,
            "satellite '{id}' should end at the run's duration, got {t}"
        );
    }
}
/// Two satellites resolving to the same id used to run: both wrote rows under
/// one recording entity path / CSV section, so the fleet silently became one
/// mislabeled satellite. The second entry here has no `id`, so it defaults to
/// `sat-1` and collides with the first entry's explicit id — the collision is
/// invisible in the file.
#[test]
fn test_cli_config_rejects_duplicate_satellite_ids() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-dup-id-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("dup.toml");
    std::fs::write(
        &config_path,
        "dt = 10.0\nduration = 60.0\n\n\
         [[satellites]]\nid = \"sat-1\"\n[satellites.orbit]\ntype = \"circular\"\naltitude = 400\n\n\
         [[satellites]]\n[satellites.orbit]\ntype = \"circular\"\naltitude = 800\n",
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    assert!(
        !output.status.success(),
        "duplicate satellite ids must fail the run, stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("duplicate satellite id 'sat-1'"),
        "error should name the colliding id, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// An inertia tensor that cannot be inverted reached
/// `SpacecraftDynamics::new`, which aborts the process (`expect`) once the
/// attitude config is honored. It is an input error, so it must be reported as
/// one, with the satellite named.
#[test]
fn test_cli_config_rejects_singular_inertia() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-inertia-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("singular.toml");
    std::fs::write(
        &config_path,
        "dt = 10.0\nduration = 60.0\n\n\
         [[satellites]]\nid = \"sat-a\"\n[satellites.orbit]\ntype = \"circular\"\naltitude = 400\n\n\
         [satellites.attitude]\ninertia_diag = [0.0, 0.0, 0.0]\nmass = 100.0\n",
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    assert!(
        !output.status.success(),
        "a singular inertia tensor must fail the run"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "bad input is a reported failure (`CmdError::failure`), not a panic (101): stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("satellites[0]")
            && stderr.contains("attitude:")
            && stderr.contains("inertia tensor"),
        "error should name the entry, the block and the constraint, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `--atmosphere nrlmsise0` is rejected by clap; the same typo in a config
/// file used to run the exponential model instead, with nothing in the output
/// naming the model that was actually integrated.
#[test]
fn test_cli_config_rejects_unknown_atmosphere() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-atmo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("atmo.toml");
    std::fs::write(
        &config_path,
        "dt = 10.0\nduration = 60.0\natmosphere = \"nrlmsise0\"\n\n\
         [[satellites]]\nid = \"a\"\nballistic_coeff = 0.01\n\
         [satellites.orbit]\ntype = \"circular\"\naltitude = 300\n",
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    assert!(
        !output.status.success(),
        "an unknown atmosphere model must fail the run"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nrlmsise0") && stderr.contains("nrlmsise00"),
        "error should name the typo and the legal spelling, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A misspelled key was dropped without a word, which is indistinguishable from
/// a key that was never written: `duraton = 60` ran for one full orbital period
/// and reported success. The run still goes ahead — that is what lets a config
/// written for a newer `orts` work here — but the key is named.
#[test]
fn test_cli_config_names_an_unread_key() {
    let dir = std::env::temp_dir().join(format!("orts-e2e-unknown-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("typo.toml");
    std::fs::write(
        &config_path,
        "dt = 10.0\nduraton = 60.0\n\n\
         [[satellites]]\nid = \"a\"\n[satellites.orbit]\ntype = \"circular\"\naltitude = 400\n",
    )
    .unwrap();

    let output = run_cli_with_config(config_path.to_str().unwrap());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the run goes ahead, ignoring the key: {stderr}"
    );
    assert!(
        stderr.contains("duraton"),
        "the warning should name the unread key, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The same collision from the CLI side: `--sat id=a` twice built a two-entry
/// fleet whose entries shared one entity path and one command target.
#[test]
fn test_cli_repeated_sat_id_is_rejected() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--sat",
            "altitude=400,id=a",
            "--sat",
            "altitude=800,id=a",
            "--duration",
            "60",
            "--output",
            "-",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts");
    assert!(
        !output.status.success(),
        "a repeated --sat id must fail the run, stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("duplicate satellite id 'a'"),
        "error should name the repeated id, got: {stderr}"
    );
}

/// A satellite that starts inside the atmosphere reports entry at t = 0.
///
/// Earth's boundary here is the 100 km Karman line, so `altitude=50` is
/// already past it when the run starts. The event check ran only after a
/// step, so the run reported "atmospheric entry at 50.0 km" at t = 10 s and
/// wrote a sample there — 78 km of arc after the entry it was reporting.
#[test]
fn a_run_starting_inside_the_atmosphere_terminates_at_t0() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--output",
            "stdout",
            "--format",
            "csv",
            "--sat",
            "altitude=50",
            "--duration",
            "100",
        ])
        .output()
        .expect("failed to execute orts");
    assert!(
        output.status.success(),
        "orts run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("terminated at t=0.00s"),
        "the entry holds at t0, so that is when it is reported: {stderr}"
    );
    assert!(
        stderr.contains("atmospheric entry"),
        "the reason is the atmosphere boundary: {stderr}"
    );

    // One row, the state handed in. The terminated state is the initial one,
    // so recording it again would double the sample.
    let csv = String::from_utf8_lossy(&output.stdout);
    let rows: Vec<&str> = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(rows.len(), 1, "one sample at t0: {rows:?}");
    assert!(
        rows[0].starts_with("0.000,"),
        "the sample sits at t0: {}",
        rows[0]
    );
}

/// The same, on the RK4 path.
///
/// `IndependentGroup` runs its own fixed-step loop rather than
/// `Integrator::integrate_with_events`, so `--integrator rk4` needed the
/// initial check of its own. Measured before that: `t=10.00s` and two rows.
#[test]
fn an_rk4_run_starting_inside_the_atmosphere_terminates_at_t0() {
    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--output",
            "stdout",
            "--format",
            "csv",
            "--sat",
            "altitude=50",
            "--duration",
            "100",
            "--integrator",
            "rk4",
        ])
        .output()
        .expect("failed to execute orts");
    assert!(
        output.status.success(),
        "orts run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("terminated at t=0.00s") && stderr.contains("atmospheric entry"),
        "the entry holds at t0, so that is when it is reported: {stderr}"
    );

    let csv = String::from_utf8_lossy(&output.stdout);
    let rows: Vec<&str> = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(rows.len(), 1, "one sample at t0: {rows:?}");
}
