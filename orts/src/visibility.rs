//! Ground-station visibility: contact window (AOS/LOS) detection.
//!
//! [`PassTracker`] is a pure state machine over `(t, elevation)` samples —
//! it knows nothing about frames or epochs, which keeps edge cases (grazing
//! passes, threshold-exact samples, windows clipped by the simulation span)
//! directly testable with synthetic sequences. Frame conversion lives in the
//! monitor layer that feeds it (see `VisibilityMonitor`).

use arika::earth::{Geodetic, TopocentricSite};
use arika::epoch::{Epoch, Utc};
use arika::frame::Vec3;

use arika::earth::EarthFixedTransform;

/// A ground station with an elevation mask.
#[derive(Debug, Clone)]
pub struct GroundStation {
    /// Station name (for reporting).
    pub name: String,
    /// Station location (WGS-84).
    pub geodetic: Geodetic,
    /// Minimum elevation for visibility [rad].
    pub min_elevation: f64,
}

/// One contact window between a satellite and a ground station.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactWindow {
    /// Acquisition of signal, simulation time [s].
    ///
    /// Linearly interpolated between samples unless `open_start`.
    pub aos: f64,
    /// Loss of signal, simulation time [s].
    ///
    /// Linearly interpolated between samples unless `open_end`.
    pub los: f64,
    /// Maximum sampled elevation within the window [rad].
    pub max_elevation: f64,
    /// Simulation time of the maximum sampled elevation [s].
    pub max_elevation_time: f64,
    /// Already visible at the first sample (true AOS not observed).
    pub open_start: bool,
    /// Still visible at the last sample (true LOS not observed).
    pub open_end: bool,
}

/// Streaming AOS/LOS detector for a single station.
///
/// Feed `(t, elevation)` samples in increasing `t` order via [`update`],
/// then call [`finish`] to retrieve the detected windows. A sample with
/// `elevation == min_elevation` counts as visible. Passes shorter than the
/// sample interval (no visible sample) cannot be detected.
///
/// [`update`]: PassTracker::update
/// [`finish`]: PassTracker::finish
#[derive(Debug)]
pub struct PassTracker {
    min_elevation: f64,
    /// Previous sample as `(t, g)` where `g = elevation - min_elevation`.
    prev: Option<(f64, f64)>,
    /// Pass currently in progress, if any.
    current: Option<ContactWindow>,
    windows: Vec<ContactWindow>,
}

impl PassTracker {
    /// Create a tracker with the given elevation mask [rad].
    pub fn new(min_elevation: f64) -> Self {
        Self {
            min_elevation,
            prev: None,
            current: None,
            windows: Vec::new(),
        }
    }

    /// Feed one `(t, elevation)` sample [s, rad]. `t` must be increasing.
    pub fn update(&mut self, t: f64, elevation: f64) {
        let g = elevation - self.min_elevation;
        let visible = g >= 0.0;

        match self.prev {
            None => {
                // First sample: a pass already in progress has no observable AOS.
                if visible {
                    self.open_pass(t, true);
                }
            }
            Some((t_prev, g_prev)) => {
                let was_visible = g_prev >= 0.0;
                if !was_visible && visible {
                    // AOS: g crossed zero upward between the two samples.
                    self.open_pass(crossing_time(t_prev, g_prev, t, g), false);
                } else if was_visible && !visible {
                    // LOS: g crossed zero downward.
                    self.close_pass(crossing_time(t_prev, g_prev, t, g));
                }
            }
        }

        if visible {
            self.track_max(t, elevation);
        }
        self.prev = Some((t, g));
    }

    /// Consume the tracker and return all detected windows.
    ///
    /// A pass still in progress is emitted with `los` = last sample time and
    /// `open_end = true`.
    pub fn finish(mut self) -> Vec<ContactWindow> {
        if let (Some(mut window), Some((t_last, _))) = (self.current.take(), self.prev) {
            window.los = t_last;
            window.open_end = true;
            self.windows.push(window);
        }
        self.windows
    }

    /// Start a new pass at time `aos`.
    fn open_pass(&mut self, aos: f64, open_start: bool) {
        debug_assert!(self.current.is_none());
        self.current = Some(ContactWindow {
            aos,
            los: f64::NAN,
            max_elevation: f64::NEG_INFINITY,
            max_elevation_time: f64::NAN,
            open_start,
            open_end: false,
        });
    }

    /// Close the pass in progress at time `los`.
    fn close_pass(&mut self, los: f64) {
        let mut window = self
            .current
            .take()
            .expect("close_pass called without a pass in progress");
        window.los = los;
        self.windows.push(window);
    }

    /// Track the running maximum sampled elevation of the pass in progress.
    fn track_max(&mut self, t: f64, elevation: f64) {
        if let Some(window) = &mut self.current
            && elevation > window.max_elevation
        {
            window.max_elevation = elevation;
            window.max_elevation_time = t;
        }
    }
}

/// Linearly interpolated zero-crossing time of `g` between two samples.
fn crossing_time(t0: f64, g0: f64, t1: f64, g1: f64) -> f64 {
    t0 + (t1 - t0) * g0 / (g0 - g1)
}

/// One contact window attributed to a station.
#[derive(Debug, Clone)]
pub struct StationContact {
    /// Station name.
    pub station: String,
    /// The detected window.
    pub window: ContactWindow,
}

/// Streaming visibility monitor: ECI samples in, contact windows out.
///
/// Wraps one [`PassTracker`] per station and performs the ECI → ECEF
/// conversion at each sample via the [`EarthFixedTransform`] for `F`
/// (`SimpleEci` = ERA-only rotation, `Gcrs` = full IAU 2006 chain).
pub struct VisibilityMonitor<F: EarthFixedTransform> {
    /// Simulation start epoch (`t = 0`).
    epoch: Epoch<Utc>,
    eop: F::EopStorage,
    stations: Vec<StationState<F>>,
}

struct StationState<F: EarthFixedTransform> {
    station: GroundStation,
    site: TopocentricSite<F::Fixed>,
    tracker: PassTracker,
}

impl<F: EarthFixedTransform> VisibilityMonitor<F> {
    /// Create a monitor. `epoch` anchors simulation time `t = 0` (contact
    /// windows depend on Earth rotation, so it is required).
    pub fn new(epoch: Epoch<Utc>, eop: F::EopStorage, stations: Vec<GroundStation>) -> Self {
        let stations = stations
            .into_iter()
            .map(|station| StationState {
                site: TopocentricSite::new(station.geodetic),
                tracker: PassTracker::new(station.min_elevation),
                station,
            })
            .collect();
        Self {
            epoch,
            eop,
            stations,
        }
    }

    /// Feed one sample: simulation time `t` [s since epoch] and the
    /// satellite's ECI position [km]. `t` must be increasing.
    pub fn update(&mut self, t: f64, position: &Vec3<F>) {
        let utc = self.epoch.add_si_seconds(t);
        let ecef = F::fixed_to_inertial(&utc, &self.eop)
            .inverse()
            .transform(position);
        for s in &mut self.stations {
            s.tracker.update(t, s.site.look_angles(&ecef).elevation);
        }
    }

    /// Consume the monitor and return all detected contacts, in station
    /// order (chronological sorting across stations is left to the caller).
    pub fn finish(self) -> Vec<StationContact> {
        self.stations
            .into_iter()
            .flat_map(|s| {
                let name = s.station.name;
                s.tracker
                    .finish()
                    .into_iter()
                    .map(move |window| StationContact {
                        station: name.clone(),
                        window,
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_tracker(mask: f64, samples: &[(f64, f64)]) -> Vec<ContactWindow> {
        let mut tracker = PassTracker::new(mask);
        for &(t, elevation) in samples {
            tracker.update(t, elevation);
        }
        tracker.finish()
    }

    #[test]
    fn single_pass_interpolates_aos_and_los() {
        // g crosses zero between t=10..20 (−0.1→0.1 ⇒ t=15)
        // and between t=40..50 (0.1→−0.1 ⇒ t=45).
        let windows = run_tracker(
            0.0,
            &[
                (0.0, -0.2),
                (10.0, -0.1),
                (20.0, 0.1),
                (30.0, 0.3),
                (40.0, 0.1),
                (50.0, -0.1),
            ],
        );
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert!((w.aos - 15.0).abs() < 1e-12, "aos: {}", w.aos);
        assert!((w.los - 45.0).abs() < 1e-12, "los: {}", w.los);
        assert_eq!(w.max_elevation, 0.3);
        assert_eq!(w.max_elevation_time, 30.0);
        assert!(!w.open_start);
        assert!(!w.open_end);
    }

    #[test]
    fn never_visible_yields_no_windows() {
        let windows = run_tracker(0.0, &[(0.0, -0.3), (10.0, -0.1), (20.0, -0.2)]);
        assert!(windows.is_empty());
    }

    #[test]
    fn visible_at_start_sets_open_start() {
        let windows = run_tracker(0.0, &[(0.0, 0.5), (10.0, 0.3), (20.0, -0.1)]);
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert_eq!(w.aos, 0.0);
        assert!(w.open_start);
        // LOS interpolated between t=10..20: 0.3→−0.1 ⇒ t=17.5.
        assert!((w.los - 17.5).abs() < 1e-12, "los: {}", w.los);
        assert!(!w.open_end);
    }

    #[test]
    fn visible_at_end_sets_open_end() {
        let windows = run_tracker(0.0, &[(0.0, -0.1), (10.0, 0.1), (20.0, 0.2)]);
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert!((w.aos - 5.0).abs() < 1e-12, "aos: {}", w.aos);
        assert_eq!(w.los, 20.0);
        assert!(w.open_end);
        assert_eq!(w.max_elevation, 0.2);
    }

    #[test]
    fn multiple_passes_are_separated() {
        let windows = run_tracker(
            0.0,
            &[
                (0.0, -0.1),
                (10.0, 0.1),
                (20.0, -0.1),
                (30.0, -0.1),
                (40.0, 0.1),
                (50.0, -0.1),
            ],
        );
        assert_eq!(windows.len(), 2);
        assert!((windows[0].aos - 5.0).abs() < 1e-12);
        assert!((windows[0].los - 15.0).abs() < 1e-12);
        assert!((windows[1].aos - 35.0).abs() < 1e-12);
        assert!((windows[1].los - 45.0).abs() < 1e-12);
    }

    #[test]
    fn elevation_mask_is_applied() {
        // Elevations above the horizon but below a 0.2 rad mask: no contact.
        let windows = run_tracker(0.2, &[(0.0, 0.05), (10.0, 0.15), (20.0, 0.1)]);
        assert!(windows.is_empty());
    }

    #[test]
    fn sample_exactly_at_mask_counts_as_visible() {
        // g: −0.1 → 0.0 → −0.1: a degenerate touching pass at t=10.
        let windows = run_tracker(0.1, &[(0.0, 0.0), (10.0, 0.1), (20.0, 0.0)]);
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert!((w.aos - 10.0).abs() < 1e-12, "aos: {}", w.aos);
        assert!((w.los - 10.0).abs() < 1e-12, "los: {}", w.los);
        assert_eq!(w.max_elevation, 0.1);
    }

    #[test]
    fn whole_span_visible_is_one_open_window() {
        let windows = run_tracker(0.0, &[(0.0, 0.1), (10.0, 0.3), (20.0, 0.2)]);
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert_eq!(w.aos, 0.0);
        assert_eq!(w.los, 20.0);
        assert!(w.open_start);
        assert!(w.open_end);
        assert_eq!(w.max_elevation, 0.3);
        assert_eq!(w.max_elevation_time, 10.0);
    }

    #[test]
    fn crossing_time_is_linear() {
        assert_eq!(crossing_time(10.0, -0.1, 20.0, 0.1), 15.0);
        assert_eq!(crossing_time(0.0, -3.0, 10.0, 1.0), 7.5);
        // Crossing exactly at a sample.
        assert_eq!(crossing_time(10.0, 0.0, 20.0, -0.1), 10.0);
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    use arika::frame;

    fn epoch() -> Epoch<Utc> {
        Epoch::from_iso8601("2026-06-10T00:00:00Z").unwrap()
    }

    fn equator_station(name: &str, longitude: f64) -> GroundStation {
        GroundStation {
            name: name.into(),
            geodetic: Geodetic {
                latitude: 0.0,
                longitude,
                altitude: 0.0,
            },
            min_elevation: 5.0_f64.to_radians(),
        }
    }

    /// ECI position at `altitude_km` directly above (geodetic zenith of) a
    /// site, at the given UTC instant.
    fn eci_at_zenith(geo: Geodetic, altitude_km: f64, utc: &Epoch<Utc>) -> Vec3<frame::SimpleEci> {
        let above = Geodetic {
            altitude: geo.altitude + altitude_km,
            ..geo
        };
        let ecef: Vec3<frame::SimpleEcef> = above.to_ecef();
        frame::SimpleEci::fixed_to_inertial(utc, &()).transform(&ecef)
    }

    #[test]
    fn satellite_at_zenith_is_visible() {
        let station = equator_station("gs0", 0.0);
        let geo = station.geodetic;
        let mut monitor = VisibilityMonitor::<frame::SimpleEci>::new(epoch(), (), vec![station]);
        monitor.update(0.0, &eci_at_zenith(geo, 400.0, &epoch()));
        let contacts = monitor.finish();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].station, "gs0");
        // Single visible sample: both ends are clipped.
        assert!(contacts[0].window.open_start);
        assert!(contacts[0].window.open_end);
        assert!(
            (contacts[0].window.max_elevation - core::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "max elevation: {}",
            contacts[0].window.max_elevation
        );
    }

    #[test]
    fn monitor_uses_its_epoch_for_earth_rotation() {
        // Same ECI position, but the monitor's epoch is 12 h later — Earth
        // has rotated ~180°, the station faces away.
        let station = equator_station("gs0", 0.0);
        let geo = station.geodetic;
        let shifted = epoch().add_seconds(43_200.0);
        let mut monitor = VisibilityMonitor::<frame::SimpleEci>::new(shifted, (), vec![station]);
        monitor.update(0.0, &eci_at_zenith(geo, 400.0, &epoch()));
        assert!(monitor.finish().is_empty());
    }

    #[test]
    fn pass_ends_as_earth_rotates_away() {
        // Satellite fixed in inertial space at the t=0 zenith; Earth rotation
        // carries the station away. With a 5° mask at 400 km the LOS comes
        // ~15° of rotation later (≈ 3600–3700 s).
        let station = equator_station("gs0", 0.0);
        let geo = station.geodetic;
        let mut monitor = VisibilityMonitor::<frame::SimpleEci>::new(epoch(), (), vec![station]);
        let pos = eci_at_zenith(geo, 400.0, &epoch());
        let mut t = 0.0;
        while t <= 21_600.0 {
            monitor.update(t, &pos);
            t += 60.0;
        }
        let contacts = monitor.finish();
        assert_eq!(contacts.len(), 1);
        let w = &contacts[0].window;
        assert!(w.open_start);
        assert!(!w.open_end);
        assert!(
            w.los > 3000.0 && w.los < 4500.0,
            "LOS should be ~3600 s, got {}",
            w.los
        );
        assert_eq!(w.max_elevation_time, 0.0);
    }

    // Characterization snapshots
    //
    // The tests above bound the pass geometry loosely (`los > 3000 && < 4500`),
    // which a changed ECI→ECEF rotation could still satisfy. These pin the
    // actual numbers of both frames' `EarthFixedTransform` paths — the `Gcrs`
    // monitor path had no coverage at all.
    //
    // The 1e-12 relative tolerance is far tighter than any plausible change to
    // the rotation chain (a mis-threaded epoch moves the LOS by seconds) while
    // staying above last-ULP differences between platform libm implementations.

    /// Samples the `pass_ends_as_earth_rotates_away` geometry and returns
    /// `(los, max_elevation)` for the single detected window.
    fn sampled_pass<F: EarthFixedTransform>(eop: F::EopStorage, pos: Vec3<F>) -> (f64, f64) {
        let station = equator_station("gs0", 0.0);
        let mut monitor = VisibilityMonitor::<F>::new(epoch(), eop, vec![station]);
        let mut t = 0.0;
        while t <= 21_600.0 {
            monitor.update(t, &pos);
            t += 60.0;
        }
        let contacts = monitor.finish();
        assert_eq!(contacts.len(), 1);
        let w = &contacts[0].window;
        (w.los, w.max_elevation)
    }

    #[track_caller]
    fn assert_close(got: f64, want: f64, what: &str) {
        assert!(
            (got - want).abs() <= 1e-12 * want.abs(),
            "{what} changed: got {got}, want {want}"
        );
    }

    #[test]
    fn simple_eci_pass_snapshot() {
        let geo = equator_station("gs0", 0.0).geodetic;
        let (los, max_elevation) =
            sampled_pass::<frame::SimpleEci>((), eci_at_zenith(geo, 400.0, &epoch()));
        assert_close(los, 3681.1574873563354, "SimpleEci LOS");
        assert_close(
            max_elevation,
            core::f64::consts::FRAC_PI_2,
            "SimpleEci peak elevation",
        );
    }

    #[test]
    fn gcrs_pass_snapshot() {
        // Same inertial position, propagated through the full IAU 2006 chain
        // instead of the ERA-only rotation: the pass differs by the
        // frame-bias/precession offset.
        let geo = equator_station("gs0", 0.0).geodetic;
        let raw = eci_at_zenith(geo, 400.0, &epoch()).into_inner();
        let (los, max_elevation) = sampled_pass::<frame::Gcrs>(
            crate::test_support::zero_eop(),
            Vec3::<frame::Gcrs>::from_raw(raw),
        );
        assert_close(los, 3681.1588250119635, "Gcrs LOS");
        assert_close(max_elevation, 1.5612210123114512, "Gcrs peak elevation");
    }

    #[test]
    fn only_the_station_under_the_satellite_sees_it() {
        let near = equator_station("near", 0.0);
        let far = equator_station("far", core::f64::consts::PI);
        let geo = near.geodetic;
        let mut monitor = VisibilityMonitor::<frame::SimpleEci>::new(epoch(), (), vec![near, far]);
        monitor.update(0.0, &eci_at_zenith(geo, 400.0, &epoch()));
        let contacts = monitor.finish();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].station, "near");
    }
}
