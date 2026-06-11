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
