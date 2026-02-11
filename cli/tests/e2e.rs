use std::process::{Command, Stdio};

fn run_cli_csv() -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_orts-cli");
    Command::new(binary)
        .args(["run", "--output", "stdout", "--format", "csv"])
        .output()
        .expect("failed to execute orts-cli")
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
fn test_cli_orbit_closes() {
    let output = run_cli_csv();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let data_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();

    let first = parse_csv_line(data_lines[0]);
    let last = parse_csv_line(data_lines[data_lines.len() - 1]);

    // Position should return close to initial after one period
    let dx = first.1 - last.1;
    let dy = first.2 - last.2;
    let dz = first.3 - last.3;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();

    assert!(
        distance < 1.0, // less than 1 km after one full orbit
        "Orbit did not close: distance between first and last position = {:.6} km",
        distance
    );
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
    let binary = env!("CARGO_BIN_EXE_orts-cli");
    let tle_text = "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993\n\
                    2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000\n";

    use std::io::Write;
    let mut child = Command::new(binary)
        .args(["run", "--tle", "-", "--output", "stdout", "--format", "csv"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn orts-cli");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(tle_text.as_bytes())
        .expect("failed to write TLE to stdin");

    let output = child.wait_with_output().expect("failed to wait for child");
    assert!(output.status.success(), "CLI exited with non-zero status: stderr={}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain TLE orbit info
    assert!(stdout.contains("from TLE"), "Missing TLE header in: {}", stdout.lines().take(10).collect::<Vec<_>>().join("\n"));
    // Should produce CSV data with 13 fields
    let data_lines: Vec<&str> = stdout.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(data_lines.len() > 10, "Expected many data lines, got {}", data_lines.len());
    assert_eq!(data_lines[0].split(',').count(), 13, "Expected 13 CSV fields");
}
