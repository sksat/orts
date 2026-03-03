use std::collections::HashMap;

use crate::record::archetypes::OrbitalState;
use crate::record::component::{Component, ComponentName};
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
    pub mu: Option<f64>,
    pub body_radius: Option<f64>,
    pub body_name: Option<String>,
    pub altitude: Option<f64>,
    pub period: Option<f64>,
}

/// The top-level simulation recording. Holds all entities and their data.
#[derive(Debug, Default)]
pub struct Recording {
    entities: HashMap<EntityPath, EntityStore>,
    pub metadata: SimMetadata,
}

impl Recording {
    pub fn new() -> Self {
        Recording {
            entities: HashMap::new(),
            metadata: SimMetadata::default(),
        }
    }

    /// Log static (timeless) component data for an entity.
    pub fn log_static<C: Component>(&mut self, entity: &EntityPath, component: &C) {
        let store = self.entities.entry(entity.clone()).or_default();
        store
            .static_data
            .insert(C::component_name(), component.to_scalars());
    }

    /// Log temporal component data at a specific time point.
    pub fn log_temporal<C: Component>(
        &mut self,
        entity: &EntityPath,
        time_point: &TimePoint,
        component: &C,
    ) {
        let store = self.entities.entry(entity.clone()).or_default();

        let column = store
            .columns
            .entry(C::component_name())
            .or_insert_with(|| ComponentColumn::new(C::num_scalars()));

        column.push(&component.to_scalars());

        for (timeline_name, time_index) in time_point.indices() {
            store
                .timelines
                .entry(timeline_name.clone())
                .or_default()
                .push(*time_index);
        }

        store.num_rows += 1;
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

    /// Get the entity store for a given path.
    pub fn entity(&self, path: &EntityPath) -> Option<&EntityStore> {
        self.entities.get(path)
    }

    /// Iterate over all entity paths.
    pub fn entity_paths(&self) -> impl Iterator<Item = &EntityPath> {
        self.entities.keys()
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
        assert_eq!(
            pos_col.get_row(0),
            Some([6778.137, 0.0, 0.0].as_slice())
        );

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
        rec.log_static(
            &EntityPath::parse("/world/satellite"),
            &BodyRadius(0.0),
        );
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
}
