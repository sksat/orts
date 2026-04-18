use std::collections::HashMap;

use crate::record::archetypes::OrbitalState;
use crate::record::component::{Component, ComponentName};
use crate::record::components::{AngularVelocity3D, Quaternion4D};
use crate::record::entity_path::EntityPath;
use crate::record::timeline::{TimeIndex, TimePoint, TimelineName};

/// A column of component data (SoA layout for a single component type).
#[derive(Debug, Clone)]
pub struct ComponentColumn {
    /// Number of f64 values per row.
    pub scalars_per_row: usize,
    /// Flat storage: scalars_per_row * num_rows f64 values.
    pub data: Vec<f64>,
}

impl ComponentColumn {
    pub fn new(scalars_per_row: usize) -> Self {
        ComponentColumn {
            scalars_per_row,
            data: Vec::new(),
        }
    }

    pub fn push(&mut self, scalars: &[f64]) {
        debug_assert_eq!(scalars.len(), self.scalars_per_row);
        self.data.extend_from_slice(scalars);
    }

    pub fn num_rows(&self) -> usize {
        if self.scalars_per_row == 0 {
            0
        } else {
            self.data.len() / self.scalars_per_row
        }
    }

    pub fn get_row(&self, index: usize) -> Option<&[f64]> {
        let start = index * self.scalars_per_row;
        let end = start + self.scalars_per_row;
        if end <= self.data.len() {
            Some(&self.data[start..end])
        } else {
            None
        }
    }
}

/// Per-entity storage for static and temporal data.
#[derive(Debug, Clone, Default)]
pub struct EntityStore {
    /// Static components (timeless).
    pub static_data: HashMap<ComponentName, Vec<f64>>,
    /// Temporal component columns.
    pub columns: HashMap<ComponentName, ComponentColumn>,
    /// Time indices for each timeline (parallel arrays with component columns).
    pub timelines: HashMap<TimelineName, Vec<TimeIndex>>,
    /// Number of temporal rows logged.
    pub num_rows: usize,
}

/// Simulation metadata that can be embedded in a Recording.
#[derive(Debug, Clone, Default)]
pub struct SimMetadata {
    pub epoch_jd: Option<f64>,
    pub epoch_iso: Option<String>,
    pub mu: Option<f64>,
    pub body_radius: Option<f64>,
    pub body_name: Option<String>,
    pub altitude: Option<f64>,
    pub period: Option<f64>,
    /// Human-readable initial orbit description (e.g. "circular at 400 km altitude").
    pub orbit_description: Option<String>,
}

impl SimMetadata {
    /// Write CSV metadata header comments to a writer.
    ///
    /// This is the single source of truth for CSV metadata format,
    /// used by both `orts run --format csv` and `orts convert --format csv`.
    pub fn write_csv_header(&self, w: &mut dyn std::io::Write) -> std::io::Result<()> {
        writeln!(w, "# Orts simulation")?;
        if let Some(mu) = self.mu {
            writeln!(w, "# mu = {} km^3/s^2", mu)?;
        }
        if let Some(epoch_jd) = self.epoch_jd {
            writeln!(w, "# epoch_jd = {}", epoch_jd)?;
        }
        if let Some(ref iso) = self.epoch_iso {
            writeln!(w, "# epoch = {}", iso)?;
        }
        if let Some(ref name) = self.body_name {
            writeln!(w, "# central_body = {}", name.to_lowercase())?;
        }
        if let Some(radius) = self.body_radius {
            writeln!(w, "# central_body_radius = {} km", radius)?;
        }
        if let Some(ref desc) = self.orbit_description {
            writeln!(w, "# {}", desc)?;
        }
        if let Some(period) = self.period {
            writeln!(w, "# Period = {:.1} s ({:.1} min)", period, period / 60.0)?;
        }
        Ok(())
    }
}

/// Schema information for a registered component type.
#[derive(Debug, Clone)]
pub struct ComponentFieldInfo {
    /// Number of f64 values per instance.
    pub scalars_per_row: usize,
    /// Column names for each scalar (e.g. ["x", "y", "z"] for Position3D).
    pub field_names: Vec<String>,
}

/// The top-level simulation recording. Holds all entities and their data.
#[derive(Debug, Default)]
pub struct Recording {
    entities: HashMap<EntityPath, EntityStore>,
    pub metadata: SimMetadata,
    /// Registry of component schemas, populated automatically by log_temporal/log_static.
    pub component_registry: HashMap<ComponentName, ComponentFieldInfo>,
}

impl Recording {
    pub fn new() -> Self {
        Recording {
            entities: HashMap::new(),
            metadata: SimMetadata::default(),
            component_registry: HashMap::new(),
        }
    }

    /// Log static (timeless) component data for an entity.
    pub fn log_static<C: Component>(&mut self, entity: &EntityPath, component: &C) {
        let store = self.entities.entry(entity.clone()).or_default();
        store
            .static_data
            .insert(C::component_name(), component.to_scalars());

        // Register component schema
        self.component_registry
            .entry(C::component_name())
            .or_insert_with(|| ComponentFieldInfo {
                scalars_per_row: C::num_scalars(),
                field_names: C::field_names().iter().map(|s| s.to_string()).collect(),
            });
    }

    /// Look up the field names for a component by its name.
    /// Returns the component name as a single-element fallback if not registered.
    /// In practice, all components logged via `log_temporal`/`log_static` are
    /// automatically registered, so the fallback only applies to manually
    /// constructed `EntityStore` data.
    pub fn lookup_component_fields(&self, name: &ComponentName) -> Vec<String> {
        if let Some(info) = self.component_registry.get(name) {
            info.field_names.clone()
        } else {
            vec![name.to_string()]
        }
    }

    /// Log temporal component data at a specific time point.
    ///
    /// When multiple components are logged at the same time point (e.g. via
    /// [`log_orbital_state`](Self::log_orbital_state)), the timeline indices
    /// are pushed only once per logical time step, keeping
    /// `timelines[*].len() == num_rows` invariant.
    pub fn log_temporal<C: Component>(
        &mut self,
        entity: &EntityPath,
        time_point: &TimePoint,
        component: &C,
    ) {
        let store = self.entities.entry(entity.clone()).or_default();

        // Register component schema for generic export
        self.component_registry
            .entry(C::component_name())
            .or_insert_with(|| ComponentFieldInfo {
                scalars_per_row: C::num_scalars(),
                field_names: C::field_names().iter().map(|s| s.to_string()).collect(),
            });

        let column = store
            .columns
            .entry(C::component_name())
            .or_insert_with(|| ComponentColumn::new(C::num_scalars()));

        column.push(&component.to_scalars());

        // Push timeline indices only once per logical time step.
        // After pushing component data, if any column has more rows than
        // there are timeline entries, this is a new logical row.
        let max_component_rows = store
            .columns
            .values()
            .map(|c| c.num_rows())
            .max()
            .unwrap_or(0);
        let timeline_len = store
            .timelines
            .values()
            .map(|tl| tl.len())
            .max()
            .unwrap_or(0);

        if max_component_rows > timeline_len {
            for (timeline_name, time_index) in time_point.indices() {
                store
                    .timelines
                    .entry(timeline_name.clone())
                    .or_default()
                    .push(*time_index);
            }
            store.num_rows += 1;
        }
    }

    /// Convenience: log an OrbitalState archetype (position + velocity).
    pub fn log_orbital_state(
        &mut self,
        entity: &EntityPath,
        time_point: &TimePoint,
        state: &OrbitalState,
    ) {
        self.log_temporal(entity, time_point, &state.position);
        self.log_temporal(entity, time_point, &state.velocity);
    }

    /// Log orbital state with optional attitude components.
    pub fn log_orbital_state_with_attitude(
        &mut self,
        entity: &EntityPath,
        time_point: &TimePoint,
        state: &OrbitalState,
        quaternion: Option<&Quaternion4D>,
        angular_velocity: Option<&AngularVelocity3D>,
    ) {
        self.log_temporal(entity, time_point, &state.position);
        self.log_temporal(entity, time_point, &state.velocity);
        if let Some(q) = quaternion {
            self.log_temporal(entity, time_point, q);
        }
        if let Some(w) = angular_velocity {
            self.log_temporal(entity, time_point, w);
        }
    }

    /// Get the entity store for a given path.
    pub fn entity(&self, path: &EntityPath) -> Option<&EntityStore> {
        self.entities.get(path)
    }

    /// Iterate over all entity paths.
    pub fn entity_paths(&self) -> impl Iterator<Item = &EntityPath> {
        self.entities.keys()
    }

    /// Get a mutable reference to the entity store, creating it if needed.
    pub fn entity_mut(&mut self, path: &EntityPath) -> &mut EntityStore {
        self.entities.entry(path.clone()).or_default()
    }

    /// Register component field names for a given component name.
    /// Used by `load_as_recording` to populate the registry from schema metadata.
    pub fn register_component_fields(&mut self, name: ComponentName, fields: Vec<&str>) {
        self.component_registry
            .entry(name)
            .or_insert_with(|| ComponentFieldInfo {
                scalars_per_row: fields.len(),
                field_names: fields.iter().map(|s| s.to_string()).collect(),
            });
    }

    /// Get all entities matching a prefix path.
    pub fn entities_under(&self, prefix: &EntityPath) -> Vec<&EntityPath> {
        let prefix_str = prefix.to_string();
        self.entities
            .keys()
            .filter(|p| {
                let p_str = p.to_string();
                p_str.starts_with(&prefix_str)
                    && (p_str.len() == prefix_str.len()
                        || p_str.as_bytes().get(prefix_str.len()) == Some(&b'/'))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use nalgebra::Vector3;

    use super::*;
    use crate::record::components::*;
    use crate::record::timeline::TimePoint;

    #[test]
    fn component_column_push_and_get() {
        let mut col = ComponentColumn::new(3);
        col.push(&[1.0, 2.0, 3.0]);
        col.push(&[4.0, 5.0, 6.0]);

        assert_eq!(col.num_rows(), 2);
        assert_eq!(col.get_row(0), Some([1.0, 2.0, 3.0].as_slice()));
        assert_eq!(col.get_row(1), Some([4.0, 5.0, 6.0].as_slice()));
        assert_eq!(col.get_row(2), None);
    }

    #[test]
    fn component_column_scalar() {
        let mut col = ComponentColumn::new(1);
        col.push(&[42.0]);
        col.push(&[99.0]);

        assert_eq!(col.num_rows(), 2);
        assert_eq!(col.get_row(0), Some([42.0].as_slice()));
        assert_eq!(col.get_row(1), Some([99.0].as_slice()));
    }

    #[test]
    fn log_static() {
        let mut rec = Recording::new();
        let earth = EntityPath::parse("/world/earth");

        rec.log_static(&earth, &GravitationalParameter(398600.4418));
        rec.log_static(&earth, &BodyRadius(6378.137));

        let store = rec.entity(&earth).unwrap();
        assert_eq!(
            store.static_data[&GravitationalParameter::component_name()],
            vec![398600.4418]
        );
        assert_eq!(
            store.static_data[&BodyRadius::component_name()],
            vec![6378.137]
        );
    }

    #[test]
    fn log_temporal() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/default");

        let tp0 = TimePoint::new().with_sim_time(0.0).with_step(0);
        let tp1 = TimePoint::new().with_sim_time(10.0).with_step(1);

        let p0 = Position3D(Vector3::new(6778.0, 0.0, 0.0));
        let p1 = Position3D(Vector3::new(6777.0, 76.0, 0.0));

        rec.log_temporal(&sat, &tp0, &p0);
        rec.log_temporal(&sat, &tp1, &p1);

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 2);

        let col = &store.columns[&Position3D::component_name()];
        assert_eq!(col.num_rows(), 2);
        assert_eq!(col.get_row(0), Some([6778.0, 0.0, 0.0].as_slice()));
        assert_eq!(col.get_row(1), Some([6777.0, 76.0, 0.0].as_slice()));

        let sim_times = &store.timelines[&TimelineName::SimTime];
        assert_eq!(sim_times.len(), 2);
    }

    #[test]
    fn log_orbital_state() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/iss");

        let tp = TimePoint::new().with_sim_time(0.0).with_step(0);
        let os = OrbitalState::new(
            Vector3::new(6778.137, 0.0, 0.0),
            Vector3::new(0.0, 7.669, 0.0),
        );

        rec.log_orbital_state(&sat, &tp, &os);

        let store = rec.entity(&sat).unwrap();
        assert!(store.columns.contains_key(&Position3D::component_name()));
        assert!(store.columns.contains_key(&Velocity3D::component_name()));

        let pos_col = &store.columns[&Position3D::component_name()];
        assert_eq!(pos_col.get_row(0), Some([6778.137, 0.0, 0.0].as_slice()));

        let vel_col = &store.columns[&Velocity3D::component_name()];
        assert_eq!(vel_col.get_row(0), Some([0.0, 7.669, 0.0].as_slice()));
    }

    #[test]
    fn entity_paths_and_query() {
        let mut rec = Recording::new();
        let earth = EntityPath::parse("/world/earth");
        let sat1 = EntityPath::parse("/world/sat/iss");
        let sat2 = EntityPath::parse("/world/sat/hubble");
        let station = EntityPath::parse("/world/station/tanegashima");

        rec.log_static(&earth, &GravitationalParameter(398600.4418));
        rec.log_static(&sat1, &BodyRadius(0.0));
        rec.log_static(&sat2, &BodyRadius(0.0));
        rec.log_static(&station, &BodyRadius(0.0));

        assert_eq!(rec.entity_paths().count(), 4);

        let sats = rec.entities_under(&EntityPath::parse("/world/sat"));
        assert_eq!(sats.len(), 2);

        let world = rec.entities_under(&EntityPath::parse("/world"));
        assert_eq!(world.len(), 4);
    }

    #[test]
    fn entities_under_excludes_partial_matches() {
        let mut rec = Recording::new();
        rec.log_static(&EntityPath::parse("/world/satellite"), &BodyRadius(0.0));
        rec.log_static(&EntityPath::parse("/world/sat/iss"), &BodyRadius(0.0));

        // "/world/sat" should NOT match "/world/satellite"
        let sats = rec.entities_under(&EntityPath::parse("/world/sat"));
        assert_eq!(sats.len(), 1);
        assert_eq!(sats[0].to_string(), "/world/sat/iss");
    }

    #[test]
    fn empty_recording() {
        let rec = Recording::new();
        assert_eq!(rec.entity_paths().count(), 0);
        assert!(rec.entity(&EntityPath::parse("/anything")).is_none());
    }

    #[test]
    fn log_orbital_state_timelines_match_num_rows() {
        // Verify the timeline invariant: timelines.len() == num_rows
        // after log_orbital_state (which logs P+V at the same time point).
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/iss");

        for i in 0..5u64 {
            let tp = TimePoint::new().with_sim_time(i as f64 * 10.0).with_step(i);
            let os = OrbitalState::new(
                Vector3::new(6778.0, 0.0, 0.0),
                Vector3::new(0.0, 7.669, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let store = rec.entity(&sat).unwrap();
        let sim_times = &store.timelines[&TimelineName::SimTime];
        let steps = &store.timelines[&TimelineName::Step];

        // Timeline entries must equal logical row count, not 2x
        assert_eq!(
            sim_times.len(),
            5,
            "sim_times should have 5 entries, not 10"
        );
        assert_eq!(steps.len(), 5);
        assert_eq!(store.num_rows, 5);

        // Each component column also has 5 rows
        assert_eq!(store.columns[&Position3D::component_name()].num_rows(), 5);
        assert_eq!(store.columns[&Velocity3D::component_name()].num_rows(), 5);
    }

    #[test]
    fn log_orbital_state_with_attitude_timelines_match() {
        // Verify the timeline invariant holds even with 4 components per step.
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/default");

        for i in 0..3u64 {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            let os = OrbitalState::new(
                Vector3::new(6778.0, 0.0, 0.0),
                Vector3::new(0.0, 7.669, 0.0),
            );
            let q = Quaternion4D(nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0));
            let w = AngularVelocity3D(Vector3::new(0.0, 0.0, 0.01));
            rec.log_orbital_state_with_attitude(&sat, &tp, &os, Some(&q), Some(&w));
        }

        let store = rec.entity(&sat).unwrap();
        let sim_times = &store.timelines[&TimelineName::SimTime];

        // Timeline entries must be 3, not 3*4=12
        assert_eq!(sim_times.len(), 3);
        assert_eq!(store.num_rows, 3);
        assert_eq!(store.columns[&Position3D::component_name()].num_rows(), 3);
        assert_eq!(store.columns[&Velocity3D::component_name()].num_rows(), 3);
        assert_eq!(store.columns[&Quaternion4D::component_name()].num_rows(), 3);
        assert_eq!(
            store.columns[&AngularVelocity3D::component_name()].num_rows(),
            3
        );
    }

    #[test]
    fn log_position_only_entity() {
        // Verify that Position3D can be logged without Velocity3D.
        // This is the core requirement for fixing the artemis1 Moon workaround.
        let mut rec = Recording::new();
        let moon = EntityPath::parse("/world/moon");

        for i in 0..4u64 {
            let tp = TimePoint::new()
                .with_sim_time(i as f64 * 100.0)
                .with_step(i);
            let pos = Position3D(Vector3::new(-384400.0, i as f64 * 10.0, 0.0));
            rec.log_temporal(&moon, &tp, &pos);
        }

        let store = rec.entity(&moon).unwrap();
        assert_eq!(store.num_rows, 4);
        assert_eq!(store.timelines[&TimelineName::SimTime].len(), 4);
        assert_eq!(store.columns[&Position3D::component_name()].num_rows(), 4);
        assert!(!store.columns.contains_key(&Velocity3D::component_name()));
    }

    #[test]
    fn component_registry_populated() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/default");

        let tp = TimePoint::new().with_sim_time(0.0);
        let pos = Position3D(Vector3::new(6778.0, 0.0, 0.0));
        rec.log_temporal(&sat, &tp, &pos);

        // Registry should have Position3D
        let info = rec
            .component_registry
            .get(&Position3D::component_name())
            .unwrap();
        assert_eq!(info.scalars_per_row, 3);
        assert_eq!(info.field_names, vec!["x", "y", "z"]);

        // lookup_component_fields should return the same
        let fields = rec.lookup_component_fields(&Position3D::component_name());
        assert_eq!(fields, vec!["x", "y", "z"]);

        // Unknown component returns component name as fallback
        let unknown = rec.lookup_component_fields(&"orts.Unknown".into());
        assert_eq!(unknown, vec!["orts.Unknown"]);
    }
}
