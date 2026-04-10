use std::process::{Command, Stdio};

fn run_cli_csv() -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts");
    Command::new(binary)
        .args(["run", "--output", "stdout", "--format", "csv"])
        .output()
        .expect("failed to execute orts")
}

fn run_cli_csv_with_body(body: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts");
    Command::new(binary)
        .args([
            "run", "--body", body, "--output", "stdout", "--format", "csv",
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
    assert!(stdout.contains("# Orts 2-body orbit propagation"));
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
    let tle_text = "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993\n\
                    2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000\n";

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
    // Should contain TLE orbit info
    assert!(
        stdout.contains("from TLE"),
        "Missing TLE header in: {}",
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

// ──────────────────────────────────────────────────
// Plugin-controlled simulation via config file
// ──────────────────────────────────────────────────

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
        .join("plugins/pd-rw-control/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm");
    if !guest_wasm.exists() {
        eprintln!(
            "WASM not found: {}\n\
             Build: cd plugins/pd-rw-control && cargo +1.91.0 component build --release\n\
             Skipping this test.",
            guest_wasm.display()
        );
        return;
    }

    let output = run_cli_config_csv("plugins/pd-rw-control/orts.toml");

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
