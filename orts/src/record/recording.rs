use std::collections::HashMap;

use crate::record::archetypes::OrbitalState;
use crate::record::component::{Component, ComponentName};
use crate::record::components::{AngularVelocity3D, Quaternion4D};
use crate::record::entity_path::EntityPath;
use crate::record::timeline::{TimeIndex, TimePoint, TimelineName};

/// Which logical row each stored entry belongs to.
///
/// A component logged at every step, which is the common case, needs no mapping:
/// stored entry `k` is logical row `k`. `Sparse` carries the mapping for a
/// component that was logged at only some of the entity's steps, so its values
/// keep the times they were logged at rather than lining up with the leading
/// rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RowMap {
    #[default]
    Dense,
    /// Logical row of each stored entry, ascending.
    Sparse(Vec<u32>),
}

impl RowMap {
    /// Record that the entry at stored index `stored` is logical row `logical`,
    /// switching away from `Dense` only once the two actually diverge.
    fn record(&mut self, stored: usize, logical: usize) {
        // `stored_index` binary-searches, so the rows have to stay strictly
        // ascending, and they are stored as `u32`. A repeated row would make the
        // search resolve to whichever entry it landed on.
        debug_assert!(
            u32::try_from(logical).is_ok(),
            "logical row {logical} is past the u32 the row map stores"
        );
        debug_assert!(
            stored == 0 || logical > self.logical_row(stored - 1),
            "logical row {logical} does not follow {} at stored index {stored}",
            self.logical_row(stored.saturating_sub(1)),
        );
        match self {
            Self::Dense if stored == logical => {}
            Self::Dense => {
                let mut rows: Vec<u32> = (0..stored as u32).collect();
                rows.push(logical as u32);
                *self = Self::Sparse(rows);
            }
            Self::Sparse(rows) => rows.push(logical as u32),
        }
    }

    /// Logical row of stored entry `stored`.
    pub fn logical_row(&self, stored: usize) -> usize {
        match self {
            Self::Dense => stored,
            Self::Sparse(rows) => rows.get(stored).map(|r| *r as usize).unwrap_or(stored),
        }
    }

    /// Stored index holding logical row `logical`, if it holds one at all.
    pub fn stored_index(&self, logical: usize) -> Option<usize> {
        match self {
            Self::Dense => Some(logical),
            // A row past the `u32` the map stores is held by no entry. Casting
            // would wrap and could match an unrelated row.
            Self::Sparse(rows) => rows.binary_search(&u32::try_from(logical).ok()?).ok(),
        }
    }
}

/// A column of component data (SoA layout for a single component type).
#[derive(Debug, Clone)]
pub struct ComponentColumn {
    /// Number of f64 values per row.
    pub scalars_per_row: usize,
    /// Flat storage: scalars_per_row * num_rows f64 values.
    pub data: Vec<f64>,
    /// Which logical row each stored row belongs to.
    pub rows: RowMap,
}

impl ComponentColumn {
    pub fn new(scalars_per_row: usize) -> Self {
        ComponentColumn {
            scalars_per_row,
            data: Vec::new(),
            rows: RowMap::Dense,
        }
    }

    /// Append `scalars` as the next logical row.
    ///
    /// Mixing this with [`Self::push_at`] on one column would leave the row map
    /// disagreeing with the data, so it is deliberately not public: a caller
    /// that knows about logical rows uses `push_at`.
    pub(crate) fn push(&mut self, scalars: &[f64]) {
        let stored = self.num_rows();
        self.push_at(scalars, stored);
    }

    /// Append `scalars` as the entity's logical row `logical_row`.
    ///
    /// `logical_row` has to be greater than the one given for the previous
    /// stored row, and within `u32`. The row lookup binary-searches, so a
    /// repeated or out-of-order row would make it resolve to whichever entry the
    /// search landed on. A debug build asserts both.
    pub fn push_at(&mut self, scalars: &[f64], logical_row: usize) {
        debug_assert_eq!(scalars.len(), self.scalars_per_row);
        let stored = self.num_rows();
        self.data.extend_from_slice(scalars);
        self.rows.record(stored, logical_row);
    }

    pub fn num_rows(&self) -> usize {
        // checked_div yields None when scalars_per_row == 0 (avoids div-by-zero).
        self.data
            .len()
            .checked_div(self.scalars_per_row)
            .unwrap_or(0)
    }

    /// The `index`-th *stored* row. For a column that skipped steps this is not
    /// logical row `index`; use [`Self::at_logical_row`] for that.
    pub fn get_row(&self, index: usize) -> Option<&[f64]> {
        let start = index * self.scalars_per_row;
        let end = start + self.scalars_per_row;
        if end <= self.data.len() {
            Some(&self.data[start..end])
        } else {
            None
        }
    }

    /// The value this column holds at the entity's logical row `logical_row`,
    /// or `None` when the component was not logged at that step.
    pub fn at_logical_row(&self, logical_row: usize) -> Option<&[f64]> {
        self.get_row(self.rows.stored_index(logical_row)?)
    }

    /// Logical row of the `index`-th stored row.
    pub fn logical_row_of(&self, index: usize) -> usize {
        self.rows.logical_row(index)
    }
}

/// Time indices for one axis, with the logical row each one belongs to.
///
/// A `TimePoint` need not name every axis the entity uses, so an axis can cover
/// only some rows. Keeping the mapping here is what lets two axes that cover
/// different rows still be read as the same entity's timeline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimelineColumn {
    pub data: Vec<TimeIndex>,
    pub rows: RowMap,
}

/// Collects a dense axis, one time per logical row.
impl FromIterator<TimeIndex> for TimelineColumn {
    fn from_iter<I: IntoIterator<Item = TimeIndex>>(iter: I) -> Self {
        Self {
            data: iter.into_iter().collect(),
            rows: RowMap::Dense,
        }
    }
}

impl TimelineColumn {
    pub fn push_at(&mut self, index: TimeIndex, logical_row: usize) {
        let stored = self.data.len();
        self.data.push(index);
        self.rows.record(stored, logical_row);
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The time this axis holds at logical row `logical_row`.
    pub fn at_logical_row(&self, logical_row: usize) -> Option<TimeIndex> {
        self.data.get(self.rows.stored_index(logical_row)?).copied()
    }

    /// Logical row of the `index`-th stored time.
    pub fn logical_row_of(&self, index: usize) -> usize {
        self.rows.logical_row(index)
    }
}

/// Per-entity storage for static and temporal data.
#[derive(Debug, Clone, Default)]
pub struct EntityStore {
    /// Static components (timeless).
    pub static_data: HashMap<ComponentName, Vec<f64>>,
    /// Temporal component columns.
    pub columns: HashMap<ComponentName, ComponentColumn>,
    /// Time indices for each timeline, each carrying the logical rows it covers.
    pub timelines: HashMap<TimelineName, TimelineColumn>,
    /// Number of logical rows logged.
    ///
    /// The export addresses the columns and the axes by logical row, so a store
    /// assembled by hand rather than through `log_temporal` has to set this: at
    /// zero, nothing temporal is written.
    pub num_rows: usize,
    /// The `TimePoint` of the row currently being filled. Two `log_temporal`
    /// calls carrying the same `TimePoint` belong to one logical row.
    last_time_point: Option<TimePoint>,
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
        writeln!(w, "# orts simulation")?;
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
    /// Calls carrying the same `TimePoint` fill one logical row, so logging
    /// several components at one time step (e.g. via
    /// [`log_orbital_state`](Self::log_orbital_state)) advances the row once.
    ///
    /// Skipping a component at some step is how "no value here" is expressed:
    /// its column records which logical rows it does cover, so its values keep
    /// the times they were logged at.
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

        // The row is identified by the time point itself rather than inferred
        // from row counts, which is what let a column that skipped steps line up
        // with the wrong times.
        let continues_row = store
            .last_time_point
            .as_ref()
            .is_some_and(|last| last.is_same_row(time_point));
        // A component logged twice at one time point is two samples of it, so the
        // second starts a row of its own at the same time rather than landing on
        // the row the first already occupies. That also keeps one entry per
        // logical row in the column's `RowMap`.
        let row_taken = continues_row
            && store
                .columns
                .get(&C::component_name())
                .is_some_and(|column| {
                    column.num_rows() > 0
                        && column.logical_row_of(column.num_rows() - 1) == store.num_rows - 1
                });
        if !continues_row || row_taken {
            store.last_time_point = Some(time_point.clone());
            let logical_row = store.num_rows;
            for (timeline_name, time_index) in time_point.indices() {
                store
                    .timelines
                    .entry(timeline_name.clone())
                    .or_default()
                    .push_at(*time_index, logical_row);
            }
            store.num_rows += 1;
        }
        let logical_row = store.num_rows - 1;

        store
            .columns
            .entry(C::component_name())
            .or_insert_with(|| ComponentColumn::new(C::num_scalars()))
            .push_at(&component.to_scalars(), logical_row);
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
    fn a_column_logged_every_step_needs_no_row_mapping() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/dense");

        for i in 0..4u64 {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(i as f64, 0.0, 0.0)));
        }

        let store = rec.entity(&sat).unwrap();
        let col = &store.columns[&Position3D::component_name()];
        assert_eq!(
            col.rows,
            RowMap::Dense,
            "the common case must not allocate a mapping"
        );
        assert_eq!(store.timelines[&TimelineName::SimTime].rows, RowMap::Dense);
        // Dense addressing means the two lookups agree and each row is itself.
        for i in 0..4 {
            assert_eq!(col.at_logical_row(i), Some([i as f64, 0.0, 0.0].as_slice()));
        }
    }

    /// Skipping a component at some steps is how "no value here" is said. Its
    /// column records the rows it does cover, so the values stay on their own
    /// steps instead of sliding onto the leading ones (#375).
    #[test]
    fn a_column_that_skips_steps_records_the_rows_it_covers() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/sparse");

        for i in 0..6u64 {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(i as f64, 0.0, 0.0)));
            if i >= 4 {
                rec.log_temporal(&sat, &tp, &Velocity3D(Vector3::new(0.0, i as f64, 0.0)));
            }
        }

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 6);

        let vel = &store.columns[&Velocity3D::component_name()];
        assert_eq!(vel.num_rows(), 2);
        assert_eq!(vel.rows, RowMap::Sparse(vec![4, 5]));
        assert_eq!(vel.logical_row_of(0), 4);
        assert_eq!(vel.logical_row_of(1), 5);

        assert_eq!(vel.at_logical_row(4), Some([0.0, 4.0, 0.0].as_slice()));
        assert_eq!(vel.at_logical_row(5), Some([0.0, 5.0, 0.0].as_slice()));
        for absent in [0, 1, 2, 3] {
            assert_eq!(
                vel.at_logical_row(absent),
                None,
                "row {absent} has no velocity"
            );
        }

        // The dense column alongside it is untouched.
        let pos = &store.columns[&Position3D::component_name()];
        assert_eq!(pos.rows, RowMap::Dense);
        assert_eq!(pos.num_rows(), 6);
    }

    /// A `TimePoint` need not name every axis, so an axis can cover only some
    /// rows. Keeping that mapping is what stops two axes of different lengths
    /// from being read as if index 0 of each meant the same row.
    #[test]
    fn an_axis_records_the_rows_it_covers() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/axes");

        for i in 0..6u64 {
            let tp = if i < 3 {
                TimePoint::new().with_step(i)
            } else {
                TimePoint::new().with_sim_time(i as f64).with_step(i)
            };
            rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(i as f64, 0.0, 0.0)));
        }

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 6);

        let step = &store.timelines[&TimelineName::Step];
        assert_eq!(step.len(), 6);
        assert_eq!(step.rows, RowMap::Dense);

        let sim = &store.timelines[&TimelineName::SimTime];
        assert_eq!(sim.len(), 3);
        assert_eq!(sim.rows, RowMap::Sparse(vec![3, 4, 5]));
        assert_eq!(sim.at_logical_row(3), Some(TimeIndex::Seconds(3.0)));
        assert_eq!(sim.at_logical_row(0), None, "row 0 named no sim_time");
    }

    /// Two calls at the same `TimePoint` fill one row, which is what lets an
    /// archetype log several components per step.
    #[test]
    fn calls_at_one_time_point_share_a_row() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/shared");
        let tp = TimePoint::new().with_sim_time(7.0).with_step(2);

        rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(1.0, 2.0, 3.0)));
        rec.log_temporal(&sat, &tp, &Velocity3D(Vector3::new(4.0, 5.0, 6.0)));

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 1);
        assert_eq!(store.timelines[&TimelineName::SimTime].len(), 1);
        for name in [Position3D::component_name(), Velocity3D::component_name()] {
            assert_eq!(store.columns[&name].logical_row_of(0), 0);
        }
    }

    /// The axes of a `TimePoint` name a row regardless of the order they were
    /// added in. Comparing the backing `Vec` would split one step into two rows
    /// and leave position and velocity on different rows.
    #[test]
    fn axis_order_does_not_split_a_row() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/order");

        let a = TimePoint::new().with_sim_time(7.0).with_step(2);
        let b = TimePoint::new().with_step(2).with_sim_time(7.0);

        rec.log_temporal(&sat, &a, &Position3D(Vector3::new(1.0, 2.0, 3.0)));
        rec.log_temporal(&sat, &b, &Velocity3D(Vector3::new(4.0, 5.0, 6.0)));

        let store = rec.entity(&sat).unwrap();
        assert_eq!(
            store.num_rows, 1,
            "one step, whichever order the axes came in"
        );
        assert_eq!(store.timelines[&TimelineName::SimTime].len(), 1);
        assert_eq!(
            store.columns[&Position3D::component_name()].logical_row_of(0),
            store.columns[&Velocity3D::component_name()].logical_row_of(0),
        );
    }

    /// A `NaN` time is still one row per step. Comparing `NaN` by value makes
    /// every call a new row, which would scatter one step's components.
    #[test]
    fn a_nan_time_still_groups_one_step() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/nan");
        let tp = TimePoint::new().with_sim_time(f64::NAN).with_step(0);

        rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(1.0, 2.0, 3.0)));
        rec.log_temporal(&sat, &tp, &Velocity3D(Vector3::new(4.0, 5.0, 6.0)));

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 1);
    }

    /// Revisiting a time already logged starts a new row rather than merging
    /// into the earlier one, which keeps `RowMap::Sparse` ascending — its
    /// `stored_index` lookup binary-searches.
    #[test]
    fn a_revisited_time_starts_a_new_row() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/revisit");

        for t in [0.0, 10.0, 0.0] {
            let tp = TimePoint::new().with_sim_time(t);
            rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(t, 0.0, 0.0)));
        }

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 3);
        let col = &store.columns[&Position3D::component_name()];
        assert_eq!(col.rows, RowMap::Dense);
        // Both t=0 rows are kept, in the order they were logged.
        let times: Vec<f64> = (0..3).map(|i| col.at_logical_row(i).unwrap()[0]).collect();
        assert_eq!(times, vec![0.0, 10.0, 0.0]);
    }

    /// `+0.0` and `-0.0` are the same instant, so they name one row. Comparing
    /// the bit patterns would split the step and leave the CSV writer with no
    /// velocity at the position's row.
    #[test]
    fn signed_zero_is_one_row() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/zero");

        rec.log_temporal(
            &sat,
            &TimePoint::new().with_sim_time(0.0),
            &Position3D(Vector3::new(1.0, 2.0, 3.0)),
        );
        rec.log_temporal(
            &sat,
            &TimePoint::new().with_sim_time(-0.0),
            &Velocity3D(Vector3::new(4.0, 5.0, 6.0)),
        );

        assert_eq!(rec.entity(&sat).unwrap().num_rows, 1);
    }

    /// One point holds one index per axis, so a repeated `with_*` is the later
    /// value. Two entries for one axis would be written to it twice and would
    /// make two points that name different times compare as one row.
    #[test]
    fn a_repeated_axis_is_the_later_value() {
        let tp = TimePoint::new().with_sim_time(1.0).with_sim_time(2.0);
        assert_eq!(tp.indices().len(), 1);
        assert_eq!(
            tp.get(&TimelineName::SimTime),
            Some(TimeIndex::Seconds(2.0))
        );
        assert!(!tp.is_same_row(&TimePoint::new().with_sim_time(1.0)));
    }

    /// Logging one component twice at a time point is two samples of it, so the
    /// second takes a row of its own at the same time. Landing both on one row
    /// would leave the column's row map with a repeated row, which its
    /// binary-searched lookup cannot resolve.
    #[test]
    fn one_component_logged_twice_at_a_time_takes_two_rows() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/twice");
        let tp = TimePoint::new().with_sim_time(4.0).with_step(1);

        rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(1.0, 0.0, 0.0)));
        rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(2.0, 0.0, 0.0)));

        let store = rec.entity(&sat).unwrap();
        assert_eq!(store.num_rows, 2);
        let col = &store.columns[&Position3D::component_name()];
        assert_eq!(col.rows, RowMap::Dense);
        assert_eq!(col.at_logical_row(0), Some([1.0, 0.0, 0.0].as_slice()));
        assert_eq!(col.at_logical_row(1), Some([2.0, 0.0, 0.0].as_slice()));
        // Both rows carry the time they were logged at.
        let axis = &store.timelines[&TimelineName::SimTime];
        for row in 0..2 {
            assert_eq!(axis.at_logical_row(row), Some(TimeIndex::Seconds(4.0)));
        }
    }

    /// A logical row past the `u32` the map stores is held by no entry. Casting
    /// would wrap and could match an unrelated row.
    #[test]
    fn a_row_past_the_map_domain_is_absent() {
        let mut col = ComponentColumn::new(1);
        col.push_at(&[1.0], 5);
        assert_eq!(col.rows, RowMap::Sparse(vec![5]));

        assert_eq!(col.at_logical_row(5), Some([1.0].as_slice()));
        // 2^32 + 5 truncates to 5, which must not be read as row 5.
        assert_eq!(col.at_logical_row(1usize << 32 | 5), None);
        assert_eq!(col.rows.stored_index(1usize << 32 | 5), None);
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
