use std::sync::Arc;

use kaname::body::KnownBody;
use kaname::epoch::Epoch;
use orts::tle::Tle;
use utsuroi::Tolerances;

use crate::cli::{AtmosphereChoice, IntegratorChoice, SimArgs};
use crate::config::SimConfig;
use crate::satellite::{OrbitSpec, SatelliteSpec, parse_body, parse_sat_spec};
use crate::tle::{fetch_tle_by_norad_id, try_fetch_tle_by_norad_id};

/// Simulation parameters derived from CLI arguments.
pub struct SimParams {
    pub body: KnownBody,
    pub mu: f64,
    pub dt: f64,
    pub output_interval: f64,
    pub stream_interval: f64,
    pub epoch: Option<Epoch>,
    pub satellites: Vec<SatelliteSpec>,
    pub integrator: IntegratorChoice,
    pub tolerances: Tolerances,
    pub atmosphere: AtmosphereChoice,
    pub f107: f64,
    pub ap: f64,
    pub space_weather_provider: Option<Arc<tobari::CssiSpaceWeather>>,
}

impl SimParams {
    /// Build an atmosphere model from the current parameters.
    pub fn build_atmosphere_model(&self) -> Option<Box<dyn tobari::AtmosphereModel>> {
        match self.atmosphere {
            AtmosphereChoice::Exponential => None, // use default
            AtmosphereChoice::HarrisPriester => Some(Box::new(tobari::HarrisPriester::new())),
            AtmosphereChoice::Nrlmsise00 => {
                let provider: Box<dyn tobari::SpaceWeatherProvider> =
                    match &self.space_weather_provider {
                        Some(cssi) => Box::new((**cssi).clone()),
                        None => Box::new(tobari::ConstantWeather::new(self.f107, self.ap)),
                    };
                Some(Box::new(tobari::Nrlmsise00::new(provider)))
            }
        }
    }
}

impl SimParams {
    /// Build SimParams from CLI arguments.
    /// `is_serve`: when true and no orbit args are given, defaults to SSO+ISS.
    pub fn from_sim_args(args: &SimArgs, is_serve: bool) -> Self {
        let body = parse_body(&args.body);
        let mu = body.properties().mu;

        let epoch = match &args.epoch {
            Some(s) => Some(Epoch::from_iso8601(s).unwrap_or_else(|| {
                panic!("Invalid epoch format: {s}. Expected ISO 8601 (e.g. 2024-03-20T12:00:00Z)")
            })),
            None => Some(Epoch::now()),
        };

        let satellites = if !args.sats.is_empty() {
            // --sat flags provided: parse each spec
            if args.tle.is_some() || args.tle_line1.is_some() || args.norad_id.is_some() {
                panic!("Cannot specify both --sat and --tle/--tle-line1/--tle-line2/--norad-id");
            }
            args.sats
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut spec = parse_sat_spec(s, body);
                    if spec.id.is_empty() || spec.id == "auto" {
                        spec.id = format!("sat-{i}");
                    }
                    spec
                })
                .collect()
        } else {
            // No --sat flags: use legacy single-satellite args
            let tle_opt = Self::parse_tle_from_args(args);

            if let Some(tle) = tle_opt {
                let elements = tle.to_keplerian_elements(mu);
                let period = elements.period(mu);
                let sat_name = tle.name.clone();
                vec![SatelliteSpec {
                    id: "default".to_string(),
                    name: sat_name,
                    orbit: OrbitSpec::Tle {
                        tle_data: tle,
                        elements,
                    },
                    period,
                    ballistic_coeff: None,
                    srp_area_to_mass: None,
                    srp_cr: None,
                    attitude_config: None,
                }]
            } else if is_serve
                && args.altitude == 400.0
                && args.tle.is_none()
                && args.tle_line1.is_none()
                && args.norad_id.is_none()
            {
                // serve with no explicit orbit → SSO + ISS default
                Self::default_serve_satellites(body, mu)
            } else {
                // Single circular orbit
                let r0 = body.properties().radius + args.altitude;
                let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
                vec![SatelliteSpec {
                    id: "default".to_string(),
                    name: None,
                    orbit: OrbitSpec::Circular {
                        altitude: args.altitude,
                        r0,
                        inclination: 0.0,
                        raan: 0.0,
                    },
                    period,
                    ballistic_coeff: None,
                    srp_area_to_mass: None,
                    srp_cr: None,
                    attitude_config: None,
                }]
            }
        };

        let output_interval = args.output_interval.unwrap_or(args.dt);
        let stream_interval = args
            .stream_interval
            .unwrap_or(output_interval)
            .clamp(args.dt, output_interval);

        // Apply --duration override: replace each satellite's period with the user-specified duration
        let satellites = if let Some(dur) = args.duration {
            satellites
                .into_iter()
                .map(|mut s| {
                    s.period = dur;
                    s
                })
                .collect()
        } else {
            satellites
        };

        Self {
            body,
            mu,
            dt: args.dt,
            output_interval,
            stream_interval,
            epoch,
            satellites,
            integrator: args.integrator,
            tolerances: Tolerances {
                atol: args.atol,
                rtol: args.rtol,
            },
            atmosphere: args.atmosphere,
            f107: args.f107,
            ap: args.ap,
            space_weather_provider: Self::load_space_weather(args.space_weather.as_deref()),
        }
    }

    /// Build SimParams from a config file.
    pub fn from_config(config: &SimConfig) -> Self {
        let body = config.known_body();
        let mu = body.properties().mu;

        let epoch = match &config.epoch {
            Some(s) => Some(Epoch::from_iso8601(s).unwrap_or_else(|| {
                panic!("Invalid epoch format: {s}. Expected ISO 8601 (e.g. 2024-03-20T12:00:00Z)")
            })),
            None => Some(Epoch::now()),
        };

        let satellites: Vec<SatelliteSpec> = config
            .satellites
            .iter()
            .enumerate()
            .map(|(i, sc)| sc.to_satellite_spec(i, body, mu))
            .collect();

        let output_interval = config.output_interval.unwrap_or(config.dt);
        let stream_interval = config
            .stream_interval
            .unwrap_or(output_interval)
            .clamp(config.dt, output_interval);

        let satellites = if let Some(dur) = config.duration {
            satellites
                .into_iter()
                .map(|mut s| {
                    s.period = dur;
                    s
                })
                .collect()
        } else {
            satellites
        };

        Self {
            body,
            mu,
            dt: config.dt,
            output_interval,
            stream_interval,
            epoch,
            satellites,
            integrator: config.integrator_choice(),
            tolerances: Tolerances {
                atol: config.integrator.atol,
                rtol: config.integrator.rtol,
            },
            atmosphere: config.atmosphere_choice(),
            f107: config.f107,
            ap: config.ap,
            space_weather_provider: Self::load_space_weather(config.space_weather.as_deref()),
        }
    }

    /// Load space weather provider from a source string.
    fn load_space_weather(source: Option<&str>) -> Option<Arc<tobari::CssiSpaceWeather>> {
        match source {
            Some("auto") => {
                let cssi = tobari::CssiSpaceWeather::fetch_default()
                    .expect("Failed to fetch space weather data from CelesTrak");
                Some(Arc::new(cssi))
            }
            Some(path) => {
                let cssi = tobari::CssiSpaceWeather::from_file(std::path::Path::new(path))
                    .unwrap_or_else(|e| panic!("Failed to load space weather file {path}: {e}"));
                Some(Arc::new(cssi))
            }
            None => None,
        }
    }

    /// Default satellites for `serve` with no orbit args: SSO 800km + ISS.
    pub fn default_serve_satellites(body: KnownBody, mu: f64) -> Vec<SatelliteSpec> {
        let mut sats = Vec::new();

        // SSO at 800 km (always available, no network needed)
        let r0 = body.properties().radius + 800.0;
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        sats.push(SatelliteSpec {
            id: "sso".to_string(),
            name: Some("SSO 800km".to_string()),
            orbit: OrbitSpec::Circular {
                altitude: 800.0,
                r0,
                inclination: 98.6_f64.to_radians(),
                raan: 0.0,
            },
            period,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            attitude_config: Some(crate::config::AttitudeConfig {
                inertia_diag: [100.0, 200.0, 50.0],
                inertia_off_diag: [0.0, 0.0, 0.0],
                mass: 500.0,
                initial_quaternion: [1.0, 0.0, 0.0, 0.0],
                initial_angular_velocity: [0.0, 0.0, 0.0],
            }),
        });

        // ISS: try online sources, fall back to embedded TLE
        let iss_tle = try_fetch_tle_by_norad_id(25544).unwrap_or_else(|| {
            eprintln!("Online TLE sources unavailable. Using embedded ISS TLE.");
            // Embedded ISS TLE (updated 2026-02-13)
            Tle::parse(
                "0 ISS (ZARYA)\n\
                 1 25544U 98067A   26044.11739808  .00007930  00000-0  15398-3 0  9991\n\
                 2 25544  51.6313 193.8240 0011114  93.1734 267.0526 15.48574923552528",
            )
            .expect("embedded ISS TLE must be valid")
        });
        let elements = iss_tle.to_keplerian_elements(mu);
        let period = elements.period(mu);
        let sat_name = iss_tle.name.clone();
        sats.push(SatelliteSpec {
            id: "iss".to_string(),
            name: sat_name,
            orbit: OrbitSpec::Tle {
                tle_data: iss_tle,
                elements,
            },
            period,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            attitude_config: Some(crate::config::AttitudeConfig {
                // Approximate ISS inertia tensor [kg·m²]
                inertia_diag: [128_913_000.0, 107_321_000.0, 201_433_000.0],
                inertia_off_diag: [0.0, 0.0, 0.0],
                mass: 420_000.0,
                initial_quaternion: [1.0, 0.0, 0.0, 0.0],
                initial_angular_velocity: [0.0, 0.0, 0.0],
            }),
        });

        sats
    }

    pub fn parse_tle_from_args(args: &SimArgs) -> Option<Tle> {
        // --norad-id: fetch from CelesTrak
        if let Some(norad_id) = args.norad_id {
            if args.tle.is_some() || args.tle_line1.is_some() {
                panic!("Cannot specify both --norad-id and --tle/--tle-line1/--tle-line2");
            }
            return Some(fetch_tle_by_norad_id(norad_id));
        }

        if let Some(path) = &args.tle {
            let text = if path == "-" {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .unwrap_or_else(|e| panic!("Failed to read TLE from stdin: {e}"));
                buf
            } else {
                std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("Failed to read TLE file '{path}': {e}"))
            };
            Some(Tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE: {e}")))
        } else if let (Some(line1), Some(line2)) = (&args.tle_line1, &args.tle_line2) {
            let text = format!("{line1}\n{line2}");
            Some(Tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE: {e}")))
        } else if args.tle_line1.is_some() || args.tle_line2.is_some() {
            panic!("Both --tle-line1 and --tle-line2 must be specified together");
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satellite::OrbitSpec;

    #[test]
    fn sim_params_stream_interval_defaults_to_output_interval() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!((params.output_interval - 10.0).abs() < 1e-9);
        assert!((params.stream_interval - 10.0).abs() < 1e-9);
        // Defaults to Epoch::now() for known bodies
        assert!(params.epoch.is_some());
    }

    #[test]
    fn sim_params_explicit_stream_interval() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(2.0),
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!((params.dt - 1.0).abs() < 1e-9);
        assert!((params.output_interval - 10.0).abs() < 1e-9);
        assert!((params.stream_interval - 2.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_stream_interval_clamped() {
        // stream_interval < dt → clamped to dt
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 5.0,
            output_interval: Some(10.0),
            stream_interval: Some(1.0),
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!((params.stream_interval - 5.0).abs() < 1e-9);

        // stream_interval > output_interval → clamped to output_interval
        let args2 = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(20.0),
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params2 = SimParams::from_sim_args(&args2, false);
        assert!((params2.stream_interval - 10.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_with_epoch() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: Some("2024-03-20T12:00:00Z".to_string()),
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!(params.epoch.is_some());
        let epoch = params.epoch.unwrap();
        // 2024-03-20 12:00:00 UTC
        assert!((epoch.jd() - 2460390.0).abs() < 0.01);
    }

    #[test]
    #[should_panic(expected = "Cannot specify both")]
    fn sim_params_norad_id_conflicts_with_tle() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string(),
            ),
            norad_id: Some(25544),
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        SimParams::from_sim_args(&args, false);
    }

    #[test]
    fn sim_params_from_tle_lines() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string(),
            ),
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);

        // Should have one satellite in TLE mode
        assert_eq!(params.satellites.len(), 1);
        let sat = &params.satellites[0];
        assert!(matches!(sat.orbit, OrbitSpec::Tle { .. }));

        // Altitude should be ~400 km
        let alt = sat.altitude(&params.body);
        assert!((alt - 400.0).abs() < 30.0, "ISS altitude: {:.1} km", alt);

        // Period should be ~92 minutes
        assert!(
            (sat.period / 60.0 - 92.0).abs() < 2.0,
            "ISS period: {:.1} min",
            sat.period / 60.0
        );
    }

    #[test]
    fn sim_params_tle_initial_state_plausible() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string(),
            ),
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        let state = params.satellites[0].initial_state(params.mu);

        let r = state.position().magnitude();
        let v = state.velocity().magnitude();
        let altitude = r - 6378.137;

        // ISS altitude ~400 km
        assert!(
            (altitude - 400.0).abs() < 30.0,
            "ISS altitude from state: {altitude:.1} km"
        );
        // ISS velocity ~7.66 km/s
        assert!((v - 7.66).abs() < 0.2, "ISS velocity: {v:.3} km/s");
    }

    #[test]
    fn sim_params_circular_mode_still_works() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);

        assert_eq!(params.satellites.len(), 1);
        assert!(matches!(
            params.satellites[0].orbit,
            OrbitSpec::Circular { .. }
        ));
        assert!((params.satellites[0].altitude(&params.body) - 400.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_tle_epoch_overridable() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: Some("2025-01-01T00:00:00Z".to_string()),
            tle: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string(),
            ),
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);

        // Epoch should be overridden to 2025-01-01
        let epoch = params.epoch.unwrap();
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2025);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
    }

    #[test]
    fn sim_params_with_sat_flags() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![
                "altitude=800,id=sso".to_string(),
                "altitude=600,id=leo".to_string(),
            ],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert_eq!(params.satellites.len(), 2);
        assert_eq!(params.satellites[0].id, "sso");
        assert_eq!(params.satellites[1].id, "leo");
    }

    #[test]
    fn sim_params_single_sat_shorthand() {
        // When no --sat flag but --altitude is used, create single satellite
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert_eq!(params.satellites.len(), 1);
        assert_eq!(params.satellites[0].id, "default");
    }

    #[test]
    fn sim_params_serve_default_sso() {
        // serve with no orbit args → at least SSO (ISS requires network)
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            config: None,
        };
        let params = SimParams::from_sim_args(&args, true);
        // Should have at least SSO satellite
        assert!(!params.satellites.is_empty());
        assert!(params.satellites.iter().any(|s| s.id == "sso"));
    }
}
