use std::process::Command;

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

    // Each data line should have 7 comma-separated fields
    for line in &data_lines {
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(
            fields.len(),
            7,
            "Expected 7 fields in CSV line, got {}: '{}'",
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
