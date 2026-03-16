/// A named timeline axis that data can be indexed by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimelineName {
    /// Simulation time in seconds.
    SimTime,
    /// Integration step counter.
    Step,
    /// Wall-clock time (for real-time streaming).
    WallClock,
    /// User-defined named timeline.
    Custom(String),
}

/// A single time index on a specific timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeIndex {
    /// Continuous time value (seconds).
    Seconds(f64),
    /// Discrete sequence number.
    Sequence(u64),
}

/// A point in time across all active timelines.
#[derive(Debug, Clone)]
pub struct TimePoint {
    indices: Vec<(TimelineName, TimeIndex)>,
}

impl TimePoint {
    pub fn new() -> Self {
        TimePoint {
            indices: Vec::new(),
        }
    }

    pub fn with_sim_time(mut self, t: f64) -> Self {
        self.indices
            .push((TimelineName::SimTime, TimeIndex::Seconds(t)));
        self
    }

    pub fn with_step(mut self, step: u64) -> Self {
        self.indices
            .push((TimelineName::Step, TimeIndex::Sequence(step)));
        self
    }

    pub fn with_wall_clock(mut self, t: f64) -> Self {
        self.indices
            .push((TimelineName::WallClock, TimeIndex::Seconds(t)));
        self
    }

    pub fn get(&self, name: &TimelineName) -> Option<TimeIndex> {
        self.indices
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, idx)| *idx)
    }

    pub fn indices(&self) -> &[(TimelineName, TimeIndex)] {
        &self.indices
    }
}

impl Default for TimePoint {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_time_point() {
        let tp = TimePoint::new();
        assert!(tp.indices().is_empty());
        assert_eq!(tp.get(&TimelineName::SimTime), None);
    }

    #[test]
    fn sim_time_only() {
        let tp = TimePoint::new().with_sim_time(100.5);
        assert_eq!(
            tp.get(&TimelineName::SimTime),
            Some(TimeIndex::Seconds(100.5))
        );
        assert_eq!(tp.get(&TimelineName::Step), None);
        assert_eq!(tp.indices().len(), 1);
    }

    #[test]
    fn multi_timeline() {
        let tp = TimePoint::new()
            .with_sim_time(100.0)
            .with_step(42)
            .with_wall_clock(1700000000.0);

        assert_eq!(
            tp.get(&TimelineName::SimTime),
            Some(TimeIndex::Seconds(100.0))
        );
        assert_eq!(tp.get(&TimelineName::Step), Some(TimeIndex::Sequence(42)));
        assert_eq!(
            tp.get(&TimelineName::WallClock),
            Some(TimeIndex::Seconds(1700000000.0))
        );
        assert_eq!(tp.indices().len(), 3);
    }

    #[test]
    fn custom_timeline() {
        let orbit_num = TimelineName::Custom("orbit_number".to_string());
        let tp = TimePoint {
            indices: vec![(orbit_num.clone(), TimeIndex::Sequence(5))],
        };
        assert_eq!(tp.get(&orbit_num), Some(TimeIndex::Sequence(5)));
    }

    #[test]
    fn default_is_empty() {
        let tp = TimePoint::default();
        assert!(tp.indices().is_empty());
    }
}
