use crate::component::Component;
use crate::components::{BodyRadius, GravitationalParameter, Position3D, Velocity3D};
use crate::entity_path::EntityPath;
use crate::recording::Recording;
use crate::timeline::{TimeIndex, TimelineName};

/// Save a Recording to a .rrd file using the Rerun SDK.
pub fn save_as_rrd(
    recording: &Recording,
    app_id: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let rec = rerun::RecordingStreamBuilder::new(app_id).save(path)?;

    for entity_path in recording.entity_paths() {
        let store = recording.entity(entity_path).unwrap();
        let rr_path = to_rerun_path(entity_path);

        // Log static data
        for (name, scalars) in &store.static_data {
            if *name == GravitationalParameter::component_name() {
                rec.log_static(
                    format!("{rr_path}/mu"),
                    &rerun::Scalars::new([scalars[0]]),
                )?;
            } else if *name == BodyRadius::component_name() {
                rec.log_static(
                    format!("{rr_path}/radius"),
                    &rerun::Scalars::new([scalars[0]]),
                )?;
            }
        }

        // Log temporal data (position + velocity)
        let pos_col = store.columns.get(&Position3D::component_name());
        let vel_col = store.columns.get(&Velocity3D::component_name());

        if let (Some(pos_col), Some(vel_col)) = (pos_col, vel_col) {
            let n = pos_col.num_rows();
            debug_assert_eq!(n, vel_col.num_rows());

            let sim_times = store.timelines.get(&TimelineName::SimTime);
            let steps = store.timelines.get(&TimelineName::Step);

            // Each log_orbital_state logs 2 components (position, velocity),
            // so the timeline has 2*n entries. The stride between logical rows is:
            let stride = sim_times
                .or(steps)
                .map(|tl| if n > 0 { tl.len() / n } else { 1 })
                .unwrap_or(1);

            for i in 0..n {
                let tl_idx = i * stride;

                if let Some(sim_times) = sim_times
                    && let Some(TimeIndex::Seconds(t)) = sim_times.get(tl_idx)
                {
                    rec.set_duration_secs("sim_time", *t);
                }
                if let Some(steps) = steps
                    && let Some(TimeIndex::Sequence(s)) = steps.get(tl_idx)
                {
                    rec.set_time_sequence("step", *s as i64);
                }

                // Log position as 3D point
                let pos = pos_col.get_row(i).unwrap();
                rec.log(
                    rr_path.clone(),
                    &rerun::Points3D::new([[pos[0], pos[1], pos[2]]]),
                )?;

                // Log velocity as individual scalars
                let vel = vel_col.get_row(i).unwrap();
                rec.log(format!("{rr_path}/vx"), &rerun::Scalars::new([vel[0]]))?;
                rec.log(format!("{rr_path}/vy"), &rerun::Scalars::new([vel[1]]))?;
                rec.log(format!("{rr_path}/vz"), &rerun::Scalars::new([vel[2]]))?;
            }
        }
    }

    rec.flush_blocking()?;
    Ok(())
}

fn to_rerun_path(path: &EntityPath) -> String {
    let s = path.to_string();
    s.strip_prefix('/').unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetypes::OrbitalState;
    use crate::timeline::TimePoint;
    use nalgebra::Vector3;

    #[test]
    fn save_recording_to_rrd() {
        let mut rec = Recording::new();
        let body = EntityPath::parse("/world/earth");
        let sat = EntityPath::parse("/world/sat/default");

        rec.log_static(&body, &GravitationalParameter(398600.4418));
        rec.log_static(&body, &BodyRadius(6378.137));

        let r0 = 6778.137;
        let v0 = (398600.4418_f64 / r0).sqrt();

        for i in 0..10u64 {
            let tp = TimePoint::new()
                .with_sim_time(i as f64 * 10.0)
                .with_step(i);
            let os = OrbitalState::new(
                Vector3::new(r0, 0.0, 0.0),
                Vector3::new(0.0, v0, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        assert!(path.exists(), ".rrd file should exist");
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0, ".rrd file should not be empty");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn to_rerun_path_strips_leading_slash() {
        let path = EntityPath::parse("/world/earth");
        assert_eq!(to_rerun_path(&path), "world/earth");
    }

    #[test]
    fn save_static_only_entity() {
        let mut rec = Recording::new();
        let body = EntityPath::parse("/world/earth");
        rec.log_static(&body, &GravitationalParameter(398600.4418));
        rec.log_static(&body, &BodyRadius(6378.137));

        let path = std::env::temp_dir().join("test_orts_static.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);

        let _ = std::fs::remove_file(&path);
    }
}
