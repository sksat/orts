use std::sync::Arc;

use arika::body::KnownBody;
use arika::elements::ParsedElementSet;
use arika::epoch::Epoch;
use utsuroi::Tolerances;

use crate::cli::FrameChoice;
use crate::cli::{
    AtmosphereChoice, IntegratorChoice, PluginAsyncModeChoice, PluginBackendChoice, SimArgs,
};
use crate::config::SimConfig;
use crate::satellite::{OrbitSpec, SatelliteSpec, parse_body, parse_sat_spec};
use crate::tle::{fetch_tle_by_norad_id, try_fetch_tle_by_norad_id};

/// Resolved WASM plugin backend selection.
///
/// Produced by [`SimParams::resolve_plugin_backend`] from the CLI
/// choice plus the satellite count. Callers pass this to the
/// WasmPluginCache to pick between `build_sync_controller` and
/// `build_async_controller`.
#[cfg(feature = "plugin-wasm")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedPluginBackend {
    Sync,
    #[cfg(feature = "plugin-wasm-async")]
    Async,
}

/// Default multiplier applied to `available_parallelism()` when the
/// user does not provide `--plugin-backend-threshold`. At 8 cores
/// this yields a threshold of 256 satellites, which comfortably sits
/// inside the "sync is still fine" band identified in the perf
/// review (≤ a few hundred OS threads).
///
/// TODO: revisit this default. The constellation-phasing bench
/// (N = 1, 8, 64 on a 16-thread CPU, sample_period = 0.1) found that
/// async is already ~5× faster than sync from N = 8 because async
/// throughput mode runs each satellite's control step in parallel
/// via `rayon` (see cli/src/commands/run.rs parallel_step path),
/// whereas sync's sim loop iterates satellites sequentially. The
/// crossover is likely near N ≈ cores, not cores × 32. Before
/// changing the default, measure with sample_period = 1.0 and with
/// heavier plugins so we don't regress plugins where per-call cost
/// dominates. Note the `.max(32)` floor in `default_auto_threshold()`
/// also pins the effective threshold to ≥ 32 regardless of this
/// constant.
#[cfg(feature = "plugin-wasm")]
const DEFAULT_THRESHOLD_PER_CORE: usize = 32;

#[cfg(feature = "plugin-wasm")]
fn resolve_plugin_backend_inner(
    choice: PluginBackendChoice,
    threshold_override: Option<usize>,
    n_sats: usize,
) -> ResolvedPluginBackend {
    let threshold = threshold_override.unwrap_or_else(default_auto_threshold);

    match choice {
        PluginBackendChoice::Sync => {
            log::info!("WASM backend = sync (forced by --plugin-backend=sync)");
            ResolvedPluginBackend::Sync
        }
        PluginBackendChoice::Async => {
            #[cfg(feature = "plugin-wasm-async")]
            {
                log::info!("WASM backend = async (forced by --plugin-backend=async)");
                ResolvedPluginBackend::Async
            }
            #[cfg(not(feature = "plugin-wasm-async"))]
            {
                log::warn!(
                    "--plugin-backend=async requested but this binary was built \
                     without the plugin-wasm-async feature; falling back to sync"
                );
                ResolvedPluginBackend::Sync
            }
        }
        PluginBackendChoice::Auto => {
            #[cfg(feature = "plugin-wasm-async")]
            {
                if n_sats > threshold {
                    log::info!(
                        "WASM backend = async (auto: n_sats={n_sats} > threshold={threshold})"
                    );
                    return ResolvedPluginBackend::Async;
                }
            }
            log::info!("WASM backend = sync (auto: n_sats={n_sats} <= threshold={threshold})");
            ResolvedPluginBackend::Sync
        }
    }
}

#[cfg(feature = "plugin-wasm")]
fn default_auto_threshold() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores * DEFAULT_THRESHOLD_PER_CORE).max(32)
}

/// The element-set epoch of the first TLE/OMM satellite, if any.
///
/// Used to default the simulation epoch when no `--epoch` is given. With a
/// single TLE/OMM this makes tsince = 0 (no SGP4 extrapolation). With several
/// element sets only the first one is at tsince = 0; the others are propagated
/// from the shared simulation epoch, so their elements *are* extrapolated by
/// the epoch difference. An explicit `--epoch` likewise extrapolates all of
/// them.
fn element_set_epoch(satellites: &[SatelliteSpec]) -> Option<Epoch> {
    satellites.iter().find_map(|s| match &s.orbit {
        OrbitSpec::ElementSet { elements, .. } => Some(elements.fields().epoch),
        OrbitSpec::Circular { .. } => None,
    })
}

/// SGP4/TEME is Earth-centered (WGS72 geopotential, TEME output frame), so a
/// TLE/OMM initial state is only physically meaningful about Earth. Reject the
/// combination of a non-Earth central body with any TLE/OMM orbit up front, so
/// we never integrate an Earth-relative SGP4 state under a foreign body's μ,
/// radius and perturbation set (which would silently produce nonsense).
pub(crate) fn validate_element_set_body(
    body: KnownBody,
    satellites: &[SatelliteSpec],
) -> Result<(), String> {
    if satellites
        .iter()
        .any(|s| matches!(s.orbit, OrbitSpec::ElementSet { .. }))
    {
        return ensure_body_carries_an_element_set(body);
    }
    Ok(())
}

/// The half of [`validate_element_set_body`] that a config settles on its own.
///
/// Shared with [`crate::config::SimConfig::validate`], which knows a satellite
/// declares `tle` or `norad` without building the spec — building it would fetch
/// a `norad_id` over the network. Reaching this only through `SimParams` meant
/// `orts config validate` called a non-Earth TLE config valid and `orts run
/// --config` then panicked on it.
pub(crate) fn ensure_body_carries_an_element_set(body: KnownBody) -> Result<(), String> {
    if body == KnownBody::Earth {
        return Ok(());
    }
    Err(format!(
        "TLE/OMM orbits are Earth-centered (SGP4/TEME, WGS72) and cannot be \
         propagated about {body:?}; use the Earth body or specify Keplerian elements"
    ))
}

/// Simulation parameters derived from CLI arguments.
pub struct SimParams {
    pub body: KnownBody,
    pub mu: f64,
    pub dt: f64,
    pub output_interval: f64,
    pub stream_interval: f64,
    pub epoch: Option<Epoch>,
    pub duration: Option<f64>,
    pub satellites: Vec<SatelliteSpec>,
    /// 時刻指定コマンドシーケンス（config transport）。CLI 引数経由では空。
    pub commands: Vec<crate::config::CommandConfig>,
    /// 地上局（contact window 検出用）。config ファイル経由でのみ設定。
    pub ground_stations: Vec<orts::visibility::GroundStation>,
    pub integrator: IntegratorChoice,
    pub tolerances: Tolerances,
    pub atmosphere: AtmosphereChoice,
    pub f107: f64,
    pub ap: f64,
    pub space_weather_provider: Option<Arc<tobari::CssiSpaceWeather>>,
    /// Inertial frame the orbit is propagated in.
    pub frame: FrameChoice,
    /// Observed Earth Orientation Parameters for `frame = gcrs`. `None` means
    /// the frame needs none (`simple-eci`) or `eop = "zero"` was asked for.
    /// Shared by every model that orients the Earth.
    pub eop: Option<Arc<arika::earth::eop::EopTable>>,
    /// Spherical-harmonic gravity field (`--gravity-field` / `[gravity_field]`),
    /// already truncated. `Some` replaces `ZonalGravity` and is where `mu`
    /// came from. Shared by every satellite's system (`Arc`).
    pub gravity_field: Option<Arc<tobari::gravity::SphericalHarmonicField>>,
    /// User-selected plugin backend from the CLI (or `Auto`).
    /// Only consulted when `plugin-wasm` is enabled.
    #[cfg_attr(not(feature = "plugin-wasm"), allow(dead_code))]
    pub plugin_backend_choice: PluginBackendChoice,
    /// Optional threshold override from the CLI.
    #[cfg_attr(not(feature = "plugin-wasm"), allow(dead_code))]
    pub plugin_backend_threshold: Option<usize>,
    /// Async backend execution mode (deterministic vs throughput).
    /// Only consulted when `plugin-wasm-async` is enabled and the
    /// resolved backend is async.
    #[cfg_attr(not(feature = "plugin-wasm-async"), allow(dead_code))]
    pub plugin_backend_async_mode: PluginAsyncModeChoice,
}

impl SimParams {
    /// Resolve the WASM plugin backend for this parameter set based
    /// on the user's CLI choice and the current satellite count.
    #[cfg(feature = "plugin-wasm")]
    pub fn resolve_plugin_backend(&self) -> ResolvedPluginBackend {
        resolve_plugin_backend_inner(
            self.plugin_backend_choice,
            self.plugin_backend_threshold,
            self.satellites.len(),
        )
    }

    /// Resolve the async-backend execution mode requested by the CLI.
    /// Only meaningful when the resolved backend is async.
    #[cfg(feature = "plugin-wasm-async")]
    pub fn resolve_async_mode(&self) -> orts::plugin::wasm::AsyncMode {
        match self.plugin_backend_async_mode {
            PluginAsyncModeChoice::Deterministic => orts::plugin::wasm::AsyncMode::Deterministic,
            PluginAsyncModeChoice::Throughput => orts::plugin::wasm::AsyncMode::Throughput,
        }
    }
}

impl SimParams {
    /// The gravity field to hand to `orts::setup`, if one is configured
    /// (a cheap `Arc` clone: the coefficients are shared, not copied).
    pub fn gravity_field(&self) -> Option<Arc<tobari::gravity::SphericalHarmonicField>> {
        self.gravity_field.clone()
    }

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
    ///
    /// `gravity_field` is the loaded `--gravity-field` (see
    /// [`load_gravity_field`](Self::load_gravity_field)); the entry points
    /// load it first so a bad file is a normal error rather than a panic
    /// here. `None` keeps the zonal model.
    pub fn from_sim_args_with_gravity_field(
        args: &SimArgs,
        is_serve: bool,
        gravity_field: Option<Arc<tobari::gravity::SphericalHarmonicField>>,
        eop: Option<Arc<arika::earth::eop::EopTable>>,
    ) -> Self {
        let body = parse_body(&args.body);
        // `mu` is the field's GM when one is configured, and it sizes every
        // satellite's period and initial state below — so it is resolved
        // before the satellites.
        let mu = Self::resolve_mu(body, gravity_field.as_deref());

        // An explicit `--epoch` wins; `None` defers the default — a TLE/OMM
        // orbit without `--epoch` starts at its element-set epoch (resolved
        // after the satellites are built, below).
        let epoch = args.epoch.as_ref().map(|s| {
            Epoch::from_iso8601(s).unwrap_or_else(|| {
                panic!("Invalid epoch format: {s}. Expected ISO 8601 (e.g. 2024-03-20T12:00:00Z)")
            })
        });

        let satellites = if !args.sats.is_empty() {
            // --sat flags provided: parse each spec
            if args.tle.is_some()
                || args.omm.is_some()
                || args.tle_line1.is_some()
                || args.tle_line2.is_some()
                || args.norad_id.is_some()
            {
                panic!(
                    "Cannot specify both --sat and --tle / --omm / --tle-line1 / --tle-line2 / --norad-id"
                );
            }
            args.sats
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut spec = parse_sat_spec(s, body, mu);
                    if spec.id.is_empty() || spec.id == "auto" {
                        spec.id = format!("sat-{i}");
                    }
                    spec
                })
                .collect()
        } else {
            // No --sat flags: use legacy single-satellite args
            let element_set_opt = Self::parse_orbit_from_args(args);

            if let Some(parsed) = element_set_opt {
                let elements = parsed.elements;
                let period = elements.period();
                let sat_name = parsed.object_name.clone();
                vec![SatelliteSpec {
                    id: "default".to_string(),
                    name: sat_name,
                    orbit: OrbitSpec::ElementSet { elements },
                    period,
                    ballistic_coeff: None,
                    srp_area_to_mass: None,
                    srp_cr: None,
                    disturbances: Default::default(),
                    panels: None,
                    attitude_config: None,
                    shape: None,
                    controller_config: None,
                    sensor_choices: None,
                    rw_config: None,
                    mtq_config: None,
                    thruster_config: None,
                    streams: Vec::new(),
                }]
            } else if is_serve {
                // serve with no explicit orbit → SSO + ISS default
                Self::default_serve_satellites(body, mu)
            } else {
                // No orbit specification — caller should have
                // checked before reaching here (orts run errors out).
                vec![]
            }
        };

        let output_interval = args.output_interval.unwrap_or(args.dt);
        // `min` keeps the clamp bounds ordered so an unvalidated arg set
        // cannot panic here; see the matching note in `from_config`.
        let stream_interval = args
            .stream_interval
            .unwrap_or(output_interval)
            .clamp(args.dt.min(output_interval), output_interval);

        // Resolve the deferred epoch: an explicit `--epoch` wins; otherwise it
        // defaults to the first TLE/OMM's element-set epoch (tsince = 0 for
        // that satellite only — see `element_set_epoch`); otherwise "now".
        let epoch = epoch
            .or_else(|| element_set_epoch(&satellites))
            .or_else(|| Some(Epoch::now()));

        validate_element_set_body(body, &satellites).unwrap_or_else(|e| panic!("{e}"));

        Self {
            body,
            mu,
            dt: args.dt,
            output_interval,
            stream_interval,
            epoch,
            duration: args.duration,
            satellites,
            // CLI-arg path has no command timeline (config-file only).
            commands: Vec::new(),
            ground_stations: Vec::new(),
            integrator: args.integrator,
            tolerances: Tolerances {
                atol: args.atol,
                rtol: args.rtol,
            },
            atmosphere: args.atmosphere,
            f107: args.f107,
            ap: args.ap,
            space_weather_provider: Self::load_space_weather(args.space_weather.as_deref()),
            frame: args.frame(),
            eop,
            gravity_field,
            plugin_backend_choice: args.plugin_backend,
            plugin_backend_threshold: args.plugin_backend_threshold,
            plugin_backend_async_mode: args.plugin_backend_async_mode,
        }
    }

    /// Build SimParams from a config file.
    ///
    /// Loads `[gravity_field]` itself and panics on a bad file, like a bad
    /// `--space-weather` file; `orts run` / `orts serve` load it first and call
    /// [`from_config_with_gravity_field`](Self::from_config_with_gravity_field).
    /// A WebSocket `start_simulation` cannot carry `[gravity_field]`
    /// (`serve::manager::validate_sim_config` rejects it), so it never
    /// reaches the panic.
    pub fn from_config(config: &SimConfig) -> Self {
        let gravity_field =
            Self::load_config_gravity_field(config).unwrap_or_else(|e| panic!("{e}"));
        let eop = Self::load_eop(config.eop.as_deref()).unwrap_or_else(|e| panic!("{e}"));
        Self::from_config_with_gravity_field(config, gravity_field, eop)
    }

    /// [`from_config`](Self::from_config) with `[gravity_field]` already
    /// loaded (`None` = the config has no table, zonal model).
    pub fn from_config_with_gravity_field(
        config: &SimConfig,
        gravity_field: Option<Arc<tobari::gravity::SphericalHarmonicField>>,
        eop: Option<Arc<arika::earth::eop::EopTable>>,
    ) -> Self {
        let body = config.known_body();
        // Field before `mu`: see `from_sim_args_with_gravity_field`.
        let mu = Self::resolve_mu(body, gravity_field.as_deref());

        // `None` defers the default; resolved from the element-set epoch after
        // the satellites are built (see `from_sim_args_with_gravity_field`).
        let epoch = config.epoch.as_ref().map(|s| {
            Epoch::from_iso8601(s).unwrap_or_else(|| {
                panic!("Invalid epoch format: {s}. Expected ISO 8601 (e.g. 2024-03-20T12:00:00Z)")
            })
        });

        let satellites: Vec<SatelliteSpec> = config
            .satellites
            .iter()
            .enumerate()
            .map(|(i, sc)| sc.to_satellite_spec(i, body, mu))
            .collect();

        let output_interval = config.output_interval.unwrap_or(config.dt);
        // `SimConfig::validate` rejects `output_interval < dt`, but this
        // constructor is also reachable without it; `min` keeps the clamp
        // bounds ordered so a bad config cannot panic here. For validated
        // configs `dt <= output_interval`, so this is the plain `dt`.
        let stream_interval = config
            .stream_interval
            .unwrap_or(output_interval)
            .clamp(config.dt.min(output_interval), output_interval);

        let epoch = epoch
            .or_else(|| element_set_epoch(&satellites))
            .or_else(|| Some(Epoch::now()));

        validate_element_set_body(body, &satellites).unwrap_or_else(|e| panic!("{e}"));

        Self {
            body,
            mu,
            dt: config.dt,
            output_interval,
            stream_interval,
            epoch,
            duration: config.duration,
            satellites,
            commands: config.commands.clone(),
            ground_stations: config
                .ground_stations
                .iter()
                .map(|g| g.to_ground_station())
                .collect(),
            integrator: config.integrator_choice(),
            tolerances: Tolerances {
                atol: config.integrator.atol,
                rtol: config.integrator.rtol,
            },
            atmosphere: config.atmosphere_choice(),
            f107: config.f107,
            ap: config.ap,
            space_weather_provider: Self::load_space_weather(config.space_weather.as_deref()),
            frame: config.frame_choice(),
            eop,
            gravity_field,
            // Config-file path: no CLI override, use defaults. The
            // auto selection logic falls back to its derived threshold.
            plugin_backend_choice: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        }
    }

    /// The simulation's μ: the gravity field's GM when one is configured
    /// (the point mass and the harmonics are one model), else the body's.
    fn resolve_mu(body: KnownBody, field: Option<&tobari::gravity::SphericalHarmonicField>) -> f64 {
        field.map_or_else(|| body.properties().mu, |f| f.gm())
    }

    /// Load and truncate a spherical-harmonic gravity field from an ICGEM
    /// `.gfc` path; `Ok(None)` when no path is given.
    ///
    /// The entry points call this once, on the main task, and hand the result
    /// to `*_with_gravity_field`: a missing or malformed file is then a normal
    /// error, the file is parsed once, and there is no second open that could
    /// fail differently. (`orts serve` builds its `SimParams` inside a spawned
    /// task, where a panic would only kill that task and leave the server up
    /// without a simulation manager.)
    pub fn load_gravity_field(
        path: Option<&str>,
        degree: Option<usize>,
        order: Option<usize>,
    ) -> Result<Option<Arc<tobari::gravity::SphericalHarmonicField>>, String> {
        let Some(path) = path else {
            return Ok(None);
        };
        let field =
            tobari::gravity::SphericalHarmonicField::from_icgem_file(std::path::Path::new(path))
                .map_err(|e| format!("Failed to load gravity field {path}: {e}"))?;
        let degree = degree.unwrap_or(field.max_degree());
        let order = order.unwrap_or(degree);
        if degree < 2 || order > degree {
            return Err(format!(
                "gravity field truncation {degree}x{order}: need degree >= 2 and order <= degree"
            ));
        }
        Ok(Some(Arc::new(field.truncated(degree, order))))
    }

    /// Load Earth Orientation Parameters from `--eop` / `eop`.
    ///
    /// `"auto"` downloads the IERS `finals2000A.all` series (24 h cache),
    /// `"zero"` asks for no observed EOP at all (`Ok(None)`, the IAU 2006
    /// model CIP), anything else is a finals2000A file path. Like the gravity
    /// field, the entry points call this on the main task so a bad file is a
    /// normal error.
    pub fn load_eop(
        source: Option<&str>,
    ) -> Result<Option<Arc<arika::earth::eop::EopTable>>, String> {
        use arika::earth::eop::EopTable;
        match source {
            None | Some("zero") => Ok(None),
            Some("auto") => {
                let table = EopTable::fetch_default()
                    .map_err(|e| format!("Failed to fetch EOP data from IERS: {e}"))?;
                Ok(Some(Arc::new(table)))
            }
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read EOP file {path}: {e}"))?;
                let table = EopTable::from_finals2000a(&text)
                    .map_err(|e| format!("Failed to parse EOP file {path}: {e}"))?;
                Ok(Some(Arc::new(table)))
            }
        }
    }

    /// The EOP storage for frame `F`, built from the loaded table.
    pub fn eop_storage<F: crate::sim::frame::RunFrame>(&self) -> F::EopStorage {
        F::eop_storage(self.eop.as_ref())
    }

    /// [`load_gravity_field`](Self::load_gravity_field) for a config's
    /// `[gravity_field]` table.
    pub fn load_config_gravity_field(
        config: &SimConfig,
    ) -> Result<Option<Arc<tobari::gravity::SphericalHarmonicField>>, String> {
        match &config.gravity_field {
            Some(gf) => Self::load_gravity_field(Some(&gf.path), gf.degree, gf.order),
            None => Ok(None),
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
    ///
    /// No `serve` invocation reaches this any more. Idle mode took the branch:
    /// `has_explicit_sim_args` gates the only call to
    /// `from_sim_args(_, is_serve = true)`, and it is true only when a config or
    /// an orbit argument is present — in which case the arms above build that
    /// instead. `orts serve` with nothing else waits for a `start_simulation`.
    /// See #393 for whether the fleet or the code should go.
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
            disturbances: Default::default(),
            panels: None,
            attitude_config: Some(crate::config::AttitudeConfig {
                // 500 kg, 2 x 1 x 1 m box: I = m/12 * (b^2 + c^2) per axis.
                // The previous [100, 200, 50] broke the triangle inequality
                // (100 + 50 < 200), i.e. no mass distribution has it — and
                // `AttitudeConfig::validate` now rejects the same numbers in a
                // config file, so the shipped default has to be realizable.
                inertia_diag: [83.3, 208.3, 208.3],
                inertia_off_diag: [0.0, 0.0, 0.0],
                mass: 500.0,
                initial_quaternion: [1.0, 0.0, 0.0, 0.0],
                initial_angular_velocity: [0.0, 0.0, 0.0],
            }),
            shape: None,
            controller_config: None,
            sensor_choices: None,
            rw_config: None,
            mtq_config: None,
            thruster_config: None,
            streams: Vec::new(),
        });

        // ISS: try online sources, fall back to embedded TLE
        let parsed_iss = try_fetch_tle_by_norad_id(25544).unwrap_or_else(|| {
            eprintln!("Online TLE sources unavailable. Using embedded ISS TLE.");
            // Embedded ISS TLE (updated 2026-02-13)
            arika::tle::parse(
                "0 ISS (ZARYA)\n\
                 1 25544U 98067A   26044.11739808  .00007930  00000-0  15398-3 0  9991\n\
                 2 25544  51.6313 193.8240 0011114  93.1734 267.0526 15.48574923552528",
            )
            .expect("embedded ISS TLE must be valid")
        });
        let iss_tle = parsed_iss.elements;
        let period = iss_tle.period();
        let sat_name = parsed_iss.object_name.clone();
        sats.push(SatelliteSpec {
            id: "iss".to_string(),
            name: sat_name,
            orbit: OrbitSpec::ElementSet { elements: iss_tle },
            period,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: Default::default(),
            panels: None,
            attitude_config: Some(crate::config::AttitudeConfig {
                // Approximate ISS inertia tensor [kg·m²]
                inertia_diag: [128_913_000.0, 107_321_000.0, 201_433_000.0],
                inertia_off_diag: [0.0, 0.0, 0.0],
                mass: 420_000.0,
                initial_quaternion: [1.0, 0.0, 0.0, 0.0],
                initial_angular_velocity: [0.0, 0.0, 0.0],
            }),
            shape: None,
            controller_config: None,
            sensor_choices: None,
            rw_config: None,
            mtq_config: None,
            thruster_config: None,
            streams: Vec::new(),
        });

        sats
    }

    /// Parse the orbit-source CLI args (`--norad-id` / `--tle` / `--omm` /
    /// `--tle-line1/2`) into a [`ParsedElementSet`], if any was given.
    pub fn parse_orbit_from_args(args: &SimArgs) -> Option<ParsedElementSet> {
        // --norad-id: fetch from CelesTrak / SatNOGS.
        if let Some(norad_id) = args.norad_id {
            if args.tle.is_some()
                || args.omm.is_some()
                || args.tle_line1.is_some()
                || args.tle_line2.is_some()
            {
                panic!("Cannot combine --norad-id with --tle / --omm / --tle-line1 / --tle-line2");
            }
            return Some(fetch_tle_by_norad_id(norad_id));
        }
        if args.tle.is_some() && args.omm.is_some() {
            panic!("Cannot specify both --tle and --omm");
        }
        // A file-based source would otherwise win silently over inline lines.
        if (args.tle.is_some() || args.omm.is_some())
            && (args.tle_line1.is_some() || args.tle_line2.is_some())
        {
            panic!("Cannot combine --tle / --omm with --tle-line1 / --tle-line2");
        }

        if let Some(path) = &args.tle {
            let text = Self::read_orbit_source(path, "TLE");
            Some(arika::tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE: {e}")))
        } else if let Some(path) = &args.omm {
            let text = Self::read_orbit_source(path, "OMM");
            // --omm is for OMM serializations (JSON/KVN/XML); route TLE to --tle.
            if arika::elements::detect(&text) == Some(arika::elements::Format::Tle) {
                panic!("--omm expects an OMM file (JSON/KVN/XML); use --tle for TLE");
            }
            Some(
                arika::elements::parse(&text)
                    .unwrap_or_else(|e| panic!("Failed to parse OMM: {e}")),
            )
        } else if let (Some(line1), Some(line2)) = (&args.tle_line1, &args.tle_line2) {
            let text = format!("{line1}\n{line2}");
            Some(arika::tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE: {e}")))
        } else if args.tle_line1.is_some() || args.tle_line2.is_some() {
            panic!("Both --tle-line1 and --tle-line2 must be specified together");
        } else {
            None
        }
    }

    /// Read an orbit-source argument from a file path, or stdin if `path == "-"`.
    fn read_orbit_source(path: &str, what: &str) -> String {
        if path == "-" {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .unwrap_or_else(|e| panic!("Failed to read {what} from stdin: {e}"));
            buf
        } else {
            std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("Failed to read {what} file '{path}': {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satellite::OrbitSpec;

    /// `duration` is the run's end time, not the satellite's orbital period.
    ///
    /// The two used to share `SatelliteSpec::period`, so `--duration 120` left
    /// every consumer of the period — the CSV header, the RRD `meta/sim/period`
    /// and the WebSocket `SatelliteInfo` — reporting 120 s for an orbit that
    /// takes 5553.6 s. The period is a property of the orbit and nothing on the
    /// command line moves it.
    #[test]
    fn duration_does_not_overwrite_the_orbital_period() {
        // 2π√(r³/μ) at r = 6778.137 km, the circular orbit 400 km up.
        const PERIOD_400KM: f64 = 5553.6;

        let mut args = sim_args_for_period_tests();
        args.sats = vec!["altitude=400".to_string()];

        let without = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!(
            (without.satellites[0].period - PERIOD_400KM).abs() < 1.0,
            "period without --duration: {}",
            without.satellites[0].period
        );

        args.duration = Some(120.0);
        let with = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!(
            (with.satellites[0].period - PERIOD_400KM).abs() < 1.0,
            "--duration 120 changed the orbital period to {}",
            with.satellites[0].period
        );
        assert_eq!(
            with.duration,
            Some(120.0),
            "the end time still has to reach SimParams"
        );
    }

    /// Same for the config path, which resolves `duration` separately.
    #[test]
    fn config_duration_does_not_overwrite_the_orbital_period() {
        const PERIOD_400KM: f64 = 5553.6;

        let config: crate::config::SimConfig = toml::from_str(
            r#"
body = "earth"
dt = 10.0
duration = 120.0

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
"#,
        )
        .expect("the fixture config parses");
        let params = SimParams::from_config(&config);

        assert!(
            (params.satellites[0].period - PERIOD_400KM).abs() < 1.0,
            "config duration changed the orbital period to {}",
            params.satellites[0].period
        );
        assert_eq!(params.duration, Some(120.0));
    }

    /// A fleet keeps each satellite's own period under `--duration`.
    ///
    /// Overwriting the period flattened the fleet onto one value, which is what
    /// made the collision invisible: both satellites reported the same period
    /// because both had been assigned the same duration.
    #[test]
    fn duration_leaves_each_satellite_its_own_period() {
        let mut args = sim_args_for_period_tests();
        args.sats = vec![
            "altitude=400,id=low".to_string(),
            "altitude=800,id=high".to_string(),
        ];
        args.duration = Some(120.0);
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);

        let (low, high) = (&params.satellites[0], &params.satellites[1]);
        assert!(
            high.period > low.period + 400.0,
            "800 km must orbit slower than 400 km: {} vs {}",
            high.period,
            low.period
        );
    }

    /// A `SimArgs` with every field at its default, for the tests above to
    /// vary one field at a time.
    fn sim_args_for_period_tests() -> SimArgs {
        SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        }
    }

    #[test]
    fn sim_params_stream_interval_defaults_to_output_interval() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!((params.output_interval - 10.0).abs() < 1e-9);
        assert!((params.stream_interval - 10.0).abs() < 1e-9);
        // Defaults to Epoch::now() for known bodies
        assert!(params.epoch.is_some());
    }

    #[test]
    fn sim_params_explicit_stream_interval() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(2.0),
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!((params.dt - 1.0).abs() < 1e-9);
        assert!((params.output_interval - 10.0).abs() < 1e-9);
        assert!((params.stream_interval - 2.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_stream_interval_clamped() {
        // stream_interval < dt → clamped to dt
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 5.0,
            output_interval: Some(10.0),
            stream_interval: Some(1.0),
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!((params.stream_interval - 5.0).abs() < 1e-9);

        // stream_interval > output_interval → clamped to output_interval
        let args2 = SimArgs {
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(20.0),
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params2 = SimParams::from_sim_args_with_gravity_field(&args2, false, None, None);
        assert!((params2.stream_interval - 10.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_with_epoch() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: Some("2024-03-20T12:00:00Z".to_string()),
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!(params.epoch.is_some());
        let epoch = params.epoch.unwrap();
        // 2024-03-20 12:00:00 UTC
        assert!((epoch.jd() - 2460390.0).abs() < 0.01);
    }

    #[test]
    #[should_panic(expected = "Cannot combine --norad-id")]
    fn sim_params_norad_id_conflicts_with_tle() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008".to_string(),
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
    }

    #[test]
    fn sim_params_from_tle_lines() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008".to_string(),
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);

        // Should have one satellite in TLE mode
        assert_eq!(params.satellites.len(), 1);
        let sat = &params.satellites[0];
        assert!(matches!(sat.orbit, OrbitSpec::ElementSet { .. }));

        // Altitude should be ~400 km
        let alt = sat.altitude(&params.body, params.mu);
        assert!((alt - 400.0).abs() < 30.0, "ISS altitude: {:.1} km", alt);

        // Period should be ~92 minutes
        assert!(
            (sat.period / 60.0 - 92.0).abs() < 2.0,
            "ISS period: {:.1} min",
            sat.period / 60.0
        );
    }

    #[test]
    fn sim_params_from_omm_json_file() {
        use std::io::Write;
        // Same ISS element set as an OMM JSON document, loaded via `--omm`.
        let json = r#"{"OBJECT_NAME":"ISS (ZARYA)","NORAD_CAT_ID":25544,
            "EPOCH":"2024-03-19T12:00:00","MEAN_MOTION":15.49561654,
            "ECCENTRICITY":0.0007417,"INCLINATION":51.64,"RA_OF_ASC_NODE":208.652,
            "ARG_OF_PERICENTER":35.391,"MEAN_ANOMALY":324.758,"BSTAR":0.00003}"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(json.as_bytes()).unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: Some(path),
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);

        assert_eq!(params.satellites.len(), 1);
        let sat = &params.satellites[0];
        assert!(matches!(sat.orbit, OrbitSpec::ElementSet { .. }));
        let alt = sat.altitude(&params.body, params.mu);
        assert!(
            (alt - 400.0).abs() < 30.0,
            "ISS altitude from OMM JSON: {alt:.1} km"
        );
    }

    #[test]
    fn sim_params_tle_initial_state_plausible() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008".to_string(),
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        let state = params.satellites[0]
            .initial_state(params.mu, params.epoch)
            .unwrap();

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
    fn validate_element_set_body_rejects_non_earth() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008".to_string(),
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert!(matches!(
            params.satellites[0].orbit,
            OrbitSpec::ElementSet { .. }
        ));

        // The same TLE-backed satellite is valid on Earth (SGP4/TEME is
        // Earth-centered) but must be rejected about any other body.
        assert!(validate_element_set_body(KnownBody::Earth, &params.satellites).is_ok());
        let err = validate_element_set_body(KnownBody::Mars, &params.satellites)
            .expect_err("a TLE/OMM orbit about Mars must be rejected");
        assert!(err.contains("Earth-centered"), "unexpected error: {err}");
    }

    #[test]
    fn sim_params_tle_epoch_overridable() {
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: Some("2025-01-01T00:00:00Z".to_string()),
            tle: None,
            omm: None,
            tle_line1: Some(
                "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996".to_string(),
            ),
            tle_line2: Some(
                "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008".to_string(),
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);

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
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, false, None, None);
        assert_eq!(params.satellites.len(), 2);
        assert_eq!(params.satellites[0].id, "sso");
        assert_eq!(params.satellites[1].id, "leo");
    }

    #[test]
    fn sim_params_serve_default_sso() {
        // serve with no orbit args → at least SSO (ISS requires network)
        let args = SimArgs {
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            omm: None,
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
            gravity_field: None,
            gravity_degree: None,
            gravity_order: None,
            frame_arg: None,
            eop: None,
            duration: None,
            config: None,
            plugin_backend: PluginBackendChoice::Auto,
            plugin_backend_threshold: None,
            plugin_backend_async_mode: PluginAsyncModeChoice::Deterministic,
        };
        let params = SimParams::from_sim_args_with_gravity_field(&args, true, None, None);
        // Should have at least SSO satellite
        assert!(!params.satellites.is_empty());
        assert!(params.satellites.iter().any(|s| s.id == "sso"));
    }

    // --- gravity field ---------------------------------------------------

    const GFC_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tobari/tests/fixtures/orekit_geopotential_70x70.gfc"
    );

    fn config_with_field(degree: &str) -> SimConfig {
        toml::from_str(&format!(
            r#"
body = "earth"
dt = 10.0
epoch = "2024-03-20T12:00:00Z"

[gravity_field]
path = "{GFC_FIXTURE}"
{degree}

[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 570 }}
"#
        ))
        .expect("valid toml")
    }

    /// With a field, `mu` is the field's GM (EGM-class 398600.4415, not
    /// WGS-84's 398600.4418) — and it is resolved *before* the satellites, so
    /// the circular orbit's period is sized with it.
    #[test]
    fn gravity_field_sets_mu_to_the_fields_gm_before_satellites() {
        let params = SimParams::from_config(&config_with_field("degree = 20\norder = 20"));
        let field = params.gravity_field.as_ref().expect("field loaded");
        assert_eq!(params.mu, field.gm());
        assert_eq!(params.mu, 398600.4415);
        assert_ne!(params.mu, KnownBody::Earth.properties().mu);
        assert_eq!((field.max_degree(), field.max_order()), (20, 20));
        // Period sized with the field's GM, not WGS-84's.
        let r = KnownBody::Earth.properties().radius + 570.0;
        let expected = 2.0 * std::f64::consts::PI * (r * r * r / params.mu).sqrt();
        assert!((params.satellites[0].period - expected).abs() < 1e-9);
    }

    #[test]
    fn gravity_field_truncation_defaults_to_the_files_degree_and_order_to_degree() {
        let params = SimParams::from_config(&config_with_field(""));
        let field = params.gravity_field.as_ref().unwrap();
        assert_eq!((field.max_degree(), field.max_order()), (70, 70));
        let params = SimParams::from_config(&config_with_field("degree = 8"));
        let field = params.gravity_field.as_ref().unwrap();
        assert_eq!((field.max_degree(), field.max_order()), (8, 8));
    }

    #[test]
    fn no_gravity_field_keeps_the_bodys_mu() {
        let cfg: SimConfig = toml::from_str(
            r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 570 }
"#,
        )
        .unwrap();
        let params = SimParams::from_config(&cfg);
        assert!(params.gravity_field.is_none());
        assert_eq!(params.mu, KnownBody::Earth.properties().mu);
    }

    #[test]
    #[should_panic(expected = "Failed to load gravity field")]
    fn missing_gravity_field_file_is_a_fatal_configuration_error() {
        let cfg: SimConfig = toml::from_str(
            r#"
[gravity_field]
path = "/nonexistent/EGM.gfc"

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 570 }
"#,
        )
        .unwrap();
        let _ = SimParams::from_config(&cfg);
    }

    #[test]
    fn load_gravity_field_reports_instead_of_panicking() {
        assert!(
            SimParams::load_gravity_field(None, None, None)
                .unwrap()
                .is_none()
        );
        let field = SimParams::load_gravity_field(Some(GFC_FIXTURE), Some(8), Some(8))
            .unwrap()
            .expect("a field");
        assert_eq!((field.max_degree(), field.max_order()), (8, 8));
        let missing =
            SimParams::load_gravity_field(Some("/nonexistent/EGM.gfc"), None, None).unwrap_err();
        assert!(missing.contains("/nonexistent/EGM.gfc"), "{missing}");
        let bad = SimParams::load_gravity_field(Some(GFC_FIXTURE), Some(8), Some(9)).unwrap_err();
        assert!(bad.contains("order <= degree"), "{bad}");
    }

    /// The loaded field is the one the parameters carry — no second open.
    #[test]
    fn from_config_with_gravity_field_uses_the_given_field() {
        let cfg = config_with_field("degree = 8");
        let field = SimParams::load_config_gravity_field(&cfg).unwrap();
        let params = SimParams::from_config_with_gravity_field(&cfg, field.clone(), None);
        assert!(Arc::ptr_eq(
            params.gravity_field.as_ref().unwrap(),
            field.as_ref().unwrap()
        ));
        assert_eq!(params.mu, 398600.4415);
    }

    // --- frame / eop -----------------------------------------------------

    const EOP_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../orts/tests/fixtures/finals2000A.sample"
    );

    #[test]
    fn load_eop_reads_a_finals2000a_file_and_reports_failures() {
        assert!(SimParams::load_eop(None).unwrap().is_none());
        // "zero" is "no observed EOP", not "no frame": the table is absent by
        // request, and the Gcrs storage falls back to the model CIP.
        assert!(SimParams::load_eop(Some("zero")).unwrap().is_none());

        let table = SimParams::load_eop(Some(EOP_FIXTURE))
            .unwrap()
            .expect("a table");
        let (start, end) = table.mjd_range();
        assert!(
            start < end && table.len() > 10,
            "{start}..{end}, {} rows",
            table.len()
        );

        // `EopTable` is not `Debug`, so the error is matched rather than
        // unwrapped through it.
        let missing = match SimParams::load_eop(Some("/nonexistent/finals2000A.all")) {
            Err(e) => e,
            Ok(_) => panic!("a missing EOP file must be an error"),
        };
        assert!(
            missing.contains("/nonexistent/finals2000A.all"),
            "{missing}"
        );
    }

    /// The loaded table reaches the frame's storage, and `simple-eci` asks for
    /// none.
    #[test]
    fn eop_storage_follows_the_frame() {
        let cfg: SimConfig = toml::from_str(&format!(
            "frame = \"gcrs\"\neop = \"{EOP_FIXTURE}\"\nepoch = \"2024-03-20T12:00:00Z\"\n\
             \n[[satellites]]\nid = \"a\"\norbit = {{ type = \"circular\", altitude = 570 }}\n"
        ))
        .expect("valid toml");
        let params = SimParams::from_config(&cfg);
        assert_eq!(params.frame, FrameChoice::Gcrs);
        assert!(params.eop.is_some());
        let _: arika::earth::GcrsEopStorage = params.eop_storage::<arika::frame::Gcrs>();
        let _: () = params.eop_storage::<arika::frame::SimpleEci>();
    }
}
