//! どのダイナミクスで fleet を回すかの決定。
//!
//! `orts run` と `orts serve` は同じ fleet を伝播するので、モード選択は
//! 各エントリポイントではなくここで一度だけ行う。`serve` で姿勢が伝播される
//! config は `run` でも姿勢が伝播されなければならない。

use crate::satellite::SatelliteSpec;

/// Fleet 全体に対して選択されたダイナミクス。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimMode {
    /// 並進運動のみ。
    OrbitOnly,
    /// 軌道 + 姿勢 (`[satellites.attitude]`)。制御ループなし。
    Spacecraft,
    /// 軌道 + 姿勢 + プラグインコントローラによる離散制御ループ。
    Controlled,
}

impl SimMode {
    /// 診断メッセージ用の短い名前。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OrbitOnly => "orbit-only",
            Self::Spacecraft => "spacecraft",
            Self::Controlled => "controlled",
        }
    }
}

/// `satellites` に対するモードを選ぶ。
///
/// 姿勢設定・コントローラ設定は fleet 単位で全か無かでなければならない:
/// どちらも「一部の衛星だけ」では、その設定を持つ衛星の分が黙って捨てられる
/// （制御ループは fleet 全体を回すか全く回さないかのどちらか）。
///
/// 空の fleet は orbit-only。`serve` が衛星なしで起動して
/// `add_satellite` で増やせるようにするため。
pub fn select_sim_mode(satellites: &[SatelliteSpec]) -> Result<SimMode, String> {
    if satellites.is_empty() {
        return Ok(SimMode::OrbitOnly);
    }
    let fleet_size = satellites.len();
    let with_attitude = satellites
        .iter()
        .filter(|s| s.attitude_config.is_some())
        .count();
    let with_controller = satellites
        .iter()
        .filter(|s| s.controller_config.is_some())
        .count();
    ensure_fleet_declares_uniformly(fleet_size, with_attitude, with_controller)?;

    if with_controller == fleet_size {
        return Ok(SimMode::Controlled);
    }
    if with_attitude == fleet_size {
        return Ok(SimMode::Spacecraft);
    }
    Ok(SimMode::OrbitOnly)
}

/// The part of [`select_sim_mode`]'s rule that a config settles on its own:
/// attitude and controller are declared fleet-wide or not at all.
///
/// Shared with [`crate::config::SimConfig::validate`] so a config file meets it
/// before anything runs. It used to be reached only from the engine, which
/// `orts serve` builds inside the spawned manager: a mixed fleet validated
/// clean, `orts serve --config` printed its banner, and the config the caller
/// passed never ran — the server sat idle waiting for a `start_simulation`.
pub fn ensure_fleet_declares_uniformly(
    fleet_size: usize,
    with_attitude: usize,
    with_controller: usize,
) -> Result<(), String> {
    if with_attitude != 0 && with_attitude != fleet_size {
        return Err(
            "Mixed attitude config: some satellites have attitude, some don't. \
             Specify attitude for all satellites or remove it from all."
                .to_string(),
        );
    }
    if with_controller != 0 && with_controller != fleet_size {
        return Err(format!(
            "Mixed controller config: {with_controller} of {fleet_size} satellites have \
             `[satellites.controller]`. The control loop steps the whole fleet or none of it, \
             so those controllers would never run. Specify a controller for all satellites \
             or remove it from all."
        ));
    }
    // A controlled satellite is built on its attitude state, so
    // `build_controlled_satellite` refuses one without it. Reaching that refusal
    // takes until the engine is built, which under `orts serve --config` is
    // after the startup banner: the fleet is uniform, the mode comes out
    // `Controlled`, and the server then sits idle.
    if with_controller == fleet_size && with_attitude == 0 {
        return Err(
            "`[satellites.controller]` without `[satellites.attitude]`: a controller \
             commands the satellite's attitude, so a controlled fleet propagates attitude \
             dynamics and needs the inertia and initial state to do it. Add \
             `[satellites.attitude]` to every satellite or remove the controllers."
                .to_string(),
        );
    }
    Ok(())
}

/// 選択されたモードでは効かない設定キーについての警告文。
///
/// センサ・アクチュエータは制御ループが読み書きして初めて効く。コントローラ
/// なしで宣言してもシミュレーションに影響しないので、黙って捨てずに知らせる。
pub fn unhonored_config_warnings(satellites: &[SatelliteSpec], mode: SimMode) -> Vec<String> {
    if mode == SimMode::Controlled {
        return Vec::new();
    }
    let mut warnings = Vec::new();
    for spec in satellites {
        let mut keys: Vec<&str> = Vec::new();
        if spec.sensor_choices.is_some() {
            keys.push("sensors");
        }
        if spec.rw_config.is_some() {
            keys.push("reaction_wheels");
        }
        if spec.mtq_config.is_some() {
            keys.push("magnetorquers");
        }
        if spec.thruster_config.is_some() {
            keys.push("thruster");
        }
        if keys.is_empty() {
            continue;
        }
        warnings.push(format!(
            "satellite '{}': {} {} declared without `[satellites.controller]`; \
             nothing reads the sensors or commands the actuators, so {} no effect in \
             {} mode",
            spec.id,
            keys.iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", "),
            if keys.len() == 1 { "is" } else { "are" },
            if keys.len() == 1 {
                "it has"
            } else {
                "they have"
            },
            mode.as_str(),
        ));
    }
    warnings
}

/// `[[command]]` は届け先のコントローラが必要。
///
/// コマンドは制御ループが tick ごとに配送するので、コントローラのない run では
/// 一件も届かない。黙って捨てるのではなく拒否する。
pub fn ensure_commands_deliverable(mode: SimMode, command_count: usize) -> Result<(), String> {
    if command_count == 0 || mode == SimMode::Controlled {
        return Ok(());
    }
    Err(format!(
        "config has {command_count} `[[command]]` timeline entr{}, but no satellite has \
         `[satellites.controller]`: commands are delivered to a plugin controller on each \
         control tick, so in {} mode they would never be delivered. Add a controller or \
         remove the command timeline.",
        if command_count == 1 { "y" } else { "ies" },
        mode.as_str(),
    ))
}

fn declares_streams(satellites: &[SatelliteSpec]) -> bool {
    satellites.iter().any(|s| !s.streams.is_empty())
}

/// serve: 宣言された stream-io ストリームは配送先のコントローラが必要。
///
/// ストリームはコントローラを通して pump されるので、orbit-only / spacecraft
/// では受け取り手がなく、エンドポイントが black hole になる。
pub fn ensure_streams_supported(mode: SimMode, satellites: &[SatelliteSpec]) -> Result<(), String> {
    if mode == SimMode::Controlled || !declares_streams(satellites) {
        return Ok(());
    }
    Err(
        "stream-io streams are declared but no satellite has a controller; \
         streams require a plugin-controlled simulation"
            .to_string(),
    )
}

/// run: stream-io ストリームは `orts run` では扱えない。
///
/// ストリームを実際に pump するのは serve の realtime ループ（WS / stdio
/// bridge）だけで、`orts run` の制御ループには対向する transport がない。
/// controlled モードでも inbound は永遠に届かず、guest が書いた outbound は
/// 誰も取り出さないまま溜まって overrun する。開けない口を黙って開けるより
/// 拒否する。
pub fn ensure_streams_unused(satellites: &[SatelliteSpec]) -> Result<(), String> {
    if !declares_streams(satellites) {
        return Ok(());
    }
    Err(
        "stream-io streams are declared but `orts run` has no transport to pump them: \
         inbound bytes would never arrive and the guest's outbound writes would pile up \
         until the stream faults. Use `orts serve` (WebSocket or --stream-stdio) for \
         streams, or remove the `streams` declaration."
            .to_string(),
    )
}

/// 単一衛星の姿勢設定を検証し、`build_spacecraft_dynamics` が伝播できない姿勢で
/// panic しないようにする。
///
/// 検査そのものは [`crate::config::AttitudeConfig::validate`] にあり、config を
/// 読む経路（`orts config validate` を含む）と、spec しか持たない serve の実行時
/// `add_satellite` 経路が同じ規則を通るようにしている。
pub fn validate_satellite_spec(spec: &SatelliteSpec) -> Result<(), String> {
    let Some(att) = &spec.attitude_config else {
        return Ok(());
    };
    att.validate()
        .map_err(|e| format!("Satellite '{}' has {e}", spec.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SimConfig;
    use crate::satellite::parse_body;

    fn specs(toml: &str) -> Vec<SatelliteSpec> {
        let config: SimConfig = toml::from_str(toml).expect("config parses");
        let body = parse_body(&config.body);
        let mu = body.properties().mu;
        config
            .satellites
            .iter()
            .enumerate()
            .map(|(i, s)| s.to_satellite_spec(i, body, mu))
            .collect()
    }

    const ORBIT: &str = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
"#;

    #[test]
    fn empty_fleet_is_orbit_only() {
        assert_eq!(select_sim_mode(&[]), Ok(SimMode::OrbitOnly));
    }

    /// One attitude config with `attitude` filled in from `fields`.
    fn attitude_spec(fields: &str) -> SatelliteSpec {
        let toml = format!(
            r#"
body = "earth"
[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 400 }}
[satellites.attitude]
{fields}
"#
        );
        specs(&toml).pop().expect("one satellite")
    }

    /// Every comparison against `NaN` is false, so a config carrying one slips
    /// past a validator written only as range checks and fails partway through
    /// the run instead.
    #[test]
    fn non_finite_attitude_fields_are_rejected() {
        for (field, fields) in [
            ("inertia_diag", "inertia_diag = [nan, 10, 10]\nmass = 500"),
            (
                "inertia_off_diag",
                "inertia_diag = [10, 10, 10]\nmass = 500\ninertia_off_diag = [inf, 0, 0]",
            ),
            ("mass", "inertia_diag = [10, 10, 10]\nmass = nan"),
            (
                "initial_quaternion",
                "inertia_diag = [10, 10, 10]\nmass = 500\ninitial_quaternion = [1, 0, -inf, 0]",
            ),
            (
                "initial_angular_velocity",
                "inertia_diag = [10, 10, 10]\nmass = 500\ninitial_angular_velocity = [0, nan, 0]",
            ),
        ] {
            let err = validate_satellite_spec(&attitude_spec(fields))
                .expect_err(&format!("{field} should be rejected"));
            assert!(
                err.contains(field) && err.contains("non-finite"),
                "{field}: got {err}"
            );
        }
    }

    /// Downstream normalization divides by the sum of squares, so a quaternion
    /// is unusable whenever that sum is not positive and finite — which
    /// includes components that are themselves finite and non-zero.
    #[test]
    fn quaternions_that_cannot_be_normalized_are_rejected() {
        for quat in [
            "[0, 0, 0, 0]",
            // Squares to zero.
            "[1e-200, 0, 0, 0]",
            "[5e-324, 0, 0, 0]",
            // Squares to infinity.
            "[1e200, 0, 0, 0]",
        ] {
            let fields =
                format!("inertia_diag = [10, 10, 10]\nmass = 500\ninitial_quaternion = {quat}");
            let err = validate_satellite_spec(&attitude_spec(&fields))
                .expect_err(&format!("{quat} should be rejected"));
            assert!(err.contains("initial_quaternion"), "{quat}: got {err}");
        }
    }

    /// An unnormalized quaternion is still the attitude it points at: it is
    /// normalized on use and renormalized after every step.
    #[test]
    fn an_unnormalized_quaternion_is_accepted() {
        for quat in [
            "[1e-16, 0, 0, 0]",
            "[0.5, 0.5, 0.5, 0.5]",
            "[3, 0, 0, 4]",
            "[1e150, 0, 0, 0]",
        ] {
            let fields =
                format!("inertia_diag = [10, 10, 10]\nmass = 500\ninitial_quaternion = {quat}");
            validate_satellite_spec(&attitude_spec(&fields))
                .unwrap_or_else(|e| panic!("{quat} should be accepted: {e}"));
        }
    }

    /// The test is whether the inverse the dynamics take exists and is finite,
    /// which a threshold on the determinant cannot decide: the determinant
    /// carries the cube of the units, so a small one does not mean the tensor is
    /// ill-conditioned and a large one does not mean the inverse is usable.
    #[test]
    fn inertia_is_judged_by_the_inverse_the_dynamics_take() {
        for (label, fields) in [
            ("all zero", "inertia_diag = [0, 0, 0]\nmass = 500"),
            (
                "products overflow to inf - inf",
                "inertia_diag = [1e200, 1e200, 1e200]\nmass = 500\n\
                 inertia_off_diag = [1e200, 1e200, 1e200]",
            ),
            (
                "inverse has an infinite component",
                "inertia_diag = [5e-324, 1e154, 1e154]\nmass = 500",
            ),
            // Condition number 1, but cofactors and determinant both overflow,
            // so the inverse comes back as a finite matrix of zeros.
            (
                "inverse is silently all zeros",
                "inertia_diag = [1e154, 1e154, 1e154]\nmass = 500",
            ),
        ] {
            let err = validate_satellite_spec(&attitude_spec(fields))
                .expect_err(&format!("{label} should be rejected"));
            assert!(err.contains("inertia tensor"), "{label}: got {err}");
        }

        // Perfectly conditioned, determinant 1e-33: a magnitude threshold on the
        // determinant would have rejected this.
        validate_satellite_spec(&attitude_spec(
            "inertia_diag = [1e-11, 1e-11, 1e-11]\nmass = 500",
        ))
        .expect("a well-conditioned tensor is accepted whatever its determinant");
    }

    /// An invertible tensor is not yet an integrable state: the torque-free
    /// Euler term can overflow from the inertia and the rate alone.
    #[test]
    fn an_initial_state_that_starts_at_infinite_acceleration_is_rejected() {
        // The rate, not the inertia, is what carries it out of range here. A
        // tensor lopsided enough to overflow `I⁻¹` on its own — `[1e-308, 1, 2]`
        // — is refused earlier for violating the triangle inequality, and the
        // inequality is also what keeps the lopsided case from reaching this
        // term: a tiny `I1` forces `I2 ≈ I3`, which shrinks the first component
        // of `ω × Iω` by as much as `I⁻¹` magnifies it.
        let err = validate_satellite_spec(&attitude_spec(
            "inertia_diag = [1, 1, 2]\nmass = 500\n\
             initial_angular_velocity = [1e200, 1e200, 1e200]",
        ))
        .expect_err("an infinite initial angular acceleration should be rejected");
        assert!(err.contains("angular acceleration"), "got {err}");

        // The same inertia at rest has nothing to diverge.
        validate_satellite_spec(&attitude_spec("inertia_diag = [1, 1, 2]\nmass = 500"))
            .expect("a resting spacecraft integrates whatever its inertia");
    }

    /// The simulation starts from the unit quaternion the config denotes, not
    /// from its scale.
    ///
    /// Integrating the raw one lets a large quaternion grow until its sum of
    /// squares overflows; the post-step projection then divides by infinity and
    /// yields all zeros, which passes `is_finite` — so nothing stops the run —
    /// and normalizes to `NaN` the moment a sensor reads it.
    #[test]
    fn the_initial_quaternion_is_normalized_before_it_is_integrated() {
        let spec = attitude_spec(
            "inertia_diag = [1, 1, 1]\nmass = 500\n\
             initial_quaternion = [1e150, 0, 0, 0]\ninitial_angular_velocity = [1e4, 0, 0]",
        );
        let att = spec.attitude_config.as_ref().expect("attitude config");
        let q = att.normalized_initial_quaternion();
        assert!(
            (q.norm() - 1.0).abs() < 1e-12,
            "expected a unit quaternion, got {q:?}"
        );
        assert!((q[0] - 1.0).abs() < 1e-12, "expected identity, got {q:?}");
    }

    #[test]
    fn a_finite_attitude_config_is_accepted() {
        validate_satellite_spec(&attitude_spec("inertia_diag = [10, 10, 10]\nmass = 500"))
            .expect("a well-formed attitude config is accepted");
    }

    #[test]
    fn orbit_config_is_orbit_only() {
        assert_eq!(select_sim_mode(&specs(ORBIT)), Ok(SimMode::OrbitOnly));
    }

    #[test]
    fn attitude_without_controller_is_spacecraft() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
"#;
        assert_eq!(select_sim_mode(&specs(toml)), Ok(SimMode::Spacecraft));
    }

    #[test]
    fn mixed_attitude_is_rejected() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 500 }
"#;
        let err = select_sim_mode(&specs(toml)).unwrap_err();
        assert!(err.contains("Mixed attitude config"), "got: {err}");
    }

    #[test]
    fn mixed_controller_is_rejected() {
        // Both satellites have attitude, so the fleet is a valid spacecraft
        // fleet — but only one has a controller, and the control loop steps
        // the whole fleet or none of it. Running spacecraft mode here would
        // silently drop 'a's controller.
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
controller = { type = "wasm", path = "ctrl.wasm" }
[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
"#;
        let err = select_sim_mode(&specs(toml)).unwrap_err();
        assert!(err.contains("Mixed controller config"), "got: {err}");
        assert!(err.contains("1 of 2"), "got: {err}");
    }

    #[test]
    fn all_controllers_is_controlled() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
controller = { type = "wasm", path = "ctrl.wasm" }
"#;
        assert_eq!(select_sim_mode(&specs(toml)), Ok(SimMode::Controlled));
    }

    #[test]
    fn actuators_without_controller_warn() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
sensors = ["gyroscope"]
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
reaction_wheels = { type = "three_axis", inertia = 0.01, max_momentum = 1.0, max_torque = 0.5 }
"#;
        let specs = specs(toml);
        let warnings = unhonored_config_warnings(&specs, SimMode::Spacecraft);
        assert_eq!(warnings.len(), 1, "got: {warnings:?}");
        assert!(warnings[0].contains("`sensors`"), "got: {}", warnings[0]);
        assert!(
            warnings[0].contains("`reaction_wheels`"),
            "got: {}",
            warnings[0]
        );
    }

    #[test]
    fn controlled_mode_honors_actuators() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
sensors = ["gyroscope"]
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
controller = { type = "wasm", path = "ctrl.wasm" }
"#;
        assert!(unhonored_config_warnings(&specs(toml), SimMode::Controlled).is_empty());
    }

    #[test]
    fn streams_need_a_controller() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
streams = ["comlink"]
"#;
        let streamed = specs(toml);
        assert!(ensure_streams_supported(SimMode::Controlled, &streamed).is_ok());
        let err = ensure_streams_supported(SimMode::OrbitOnly, &streamed).unwrap_err();
        assert!(err.contains("streams require"), "got: {err}");
        assert!(ensure_streams_supported(SimMode::OrbitOnly, &specs(ORBIT)).is_ok());
    }

    /// `orts run` has no stream transport at all, so even controlled mode has
    /// to reject declared streams — accepting them would leave inbound bytes
    /// undelivered and outbound writes piling up until the stream faults.
    #[test]
    fn run_rejects_streams_in_every_mode() {
        let toml = r#"
body = "earth"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
controller = { type = "wasm", path = "ctrl.wasm" }
streams = ["comlink"]
"#;
        let streamed = specs(toml);
        assert_eq!(select_sim_mode(&streamed), Ok(SimMode::Controlled));
        let err = ensure_streams_unused(&streamed).unwrap_err();
        assert!(err.contains("`orts run` has no transport"), "got: {err}");
        assert!(ensure_streams_unused(&specs(ORBIT)).is_ok());
    }

    #[test]
    fn commands_need_a_controller() {
        assert!(ensure_commands_deliverable(SimMode::Controlled, 3).is_ok());
        assert!(ensure_commands_deliverable(SimMode::Spacecraft, 0).is_ok());
        let err = ensure_commands_deliverable(SimMode::Spacecraft, 1).unwrap_err();
        assert!(err.contains("`[[command]]`"), "got: {err}");
        let err = ensure_commands_deliverable(SimMode::OrbitOnly, 2).unwrap_err();
        assert!(
            err.contains("2 `[[command]]` timeline entries"),
            "got: {err}"
        );
    }
}
