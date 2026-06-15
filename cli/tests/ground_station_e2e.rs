//! E2E tests for ground-station visibility / contact window reporting.

use std::process::Command;

fn run_with_config(name: &str, content: &str) -> std::process::Output {
    let dir = std::env::temp_dir().join(format!("orts-e2e-gs-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("orts.toml");
    std::fs::write(&config_path, content).unwrap();

    let binary = env!("CARGO_BIN_EXE_orts");
    let output = Command::new(binary)
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts");
    std::fs::remove_dir_all(&dir).ok();
    output
}

#[test]
fn equatorial_orbit_over_equatorial_station_reports_contacts() {
    // An equatorial LEO orbit ground-tracks the equator, so an equatorial
    // station gets one pass per synodic revolution (~5900 s at 400 km).
    // 12000 s ≥ 2 revolutions → at least one complete contact window
    // regardless of the initial phase.
    let output = run_with_config(
        "contact",
        r#"
body = "earth"
dt = 10.0
epoch = "2026-01-01T00:00:00Z"
duration = 12000.0

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400.0, inclination = 0.0 }

[[ground_station]]
name = "gs-equator"
latitude_deg = 0.0
longitude_deg = 0.0
min_elevation_deg = 5.0
"#,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Contact windows ("),
        "missing contact report: {stderr}"
    );
    assert!(
        stderr.contains("gs-equator"),
        "missing station name: {stderr}"
    );
    assert!(stderr.contains("sat-1"), "missing satellite id: {stderr}");
    assert!(stderr.contains("max el"), "missing max elevation: {stderr}");
}

/// Extract the `(t=...s)` sim times (AOS, LOS) from a contact report line.
fn extract_times(line: &str) -> Vec<f64> {
    line.split("(t=")
        .skip(1)
        .filter_map(|s| s.split('s').next()?.parse::<f64>().ok())
        .collect()
}

#[test]
fn coarse_output_interval_does_not_miss_passes() {
    // The pass is ~496 s long, shorter than output_interval = 600 s:
    // sampling at the output cadence would miss it or smear the window.
    // Visibility feeds from accepted integrator steps (rk4 dt = 10 s), so
    // the window must survive with an accurate duration.
    let output = run_with_config(
        "coarse",
        r#"
body = "earth"
dt = 10.0
output_interval = 600.0
epoch = "2026-01-01T00:00:00Z"
duration = 12000.0

[integrator]
type = "rk4"

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400.0, inclination = 0.0 }

[[ground_station]]
name = "gs-equator"
latitude_deg = 0.0
longitude_deg = 0.0
min_elevation_deg = 5.0
"#,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Contact windows ("),
        "missing contact report: {stderr}"
    );
    let mut checked = 0;
    for line in stderr.lines().filter(|l| l.contains("gs-equator")) {
        let times = extract_times(line);
        assert_eq!(times.len(), 2, "expected AOS/LOS times in: {line}");
        let duration = times[1] - times[0];
        assert!(
            (420.0..570.0).contains(&duration),
            "pass duration {duration:.1}s should be ~496 s: {line}"
        );
        checked += 1;
    }
    assert!(checked >= 1, "no contact lines found: {stderr}");
}

#[test]
fn station_out_of_reach_reports_no_contacts() {
    // An equatorial orbit at 400 km is never visible above 5° elevation
    // from a station at 80° latitude.
    let output = run_with_config(
        "nocontact",
        r#"
body = "earth"
dt = 10.0
epoch = "2026-01-01T00:00:00Z"
duration = 12000.0

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400.0, inclination = 0.0 }

[[ground_station]]
name = "gs-polar"
latitude_deg = 80.0
longitude_deg = 0.0
min_elevation_deg = 5.0
"#,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Contact windows: none detected"),
        "expected 'none detected': {stderr}"
    );
}

#[test]
fn non_earth_body_disables_ground_stations_with_warning() {
    let output = run_with_config(
        "mars",
        r#"
body = "mars"
dt = 10.0
epoch = "2026-01-01T00:00:00Z"
duration = 6000.0

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400.0 }

[[ground_station]]
name = "gs-0"
latitude_deg = 0.0
longitude_deg = 0.0
"#,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ground station") && stderr.contains("Earth"),
        "expected Earth-only warning: {stderr}"
    );
    assert!(
        !stderr.contains("Contact windows"),
        "contact detection should be disabled: {stderr}"
    );
}
