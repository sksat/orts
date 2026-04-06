//! Snapshot data handed to a plugin controller at each sample tick.
//!
//! `Observation` bundles everything a guest needs to compute a
//! [`super::Command`]. It wraps the full [`SpacecraftState`] (orbit +
//! attitude + mass), an optional absolute epoch, and an `EnvSnapshot`
//! of environment values pre-evaluated at the tick instant.
//!
//! Using a single struct instead of multiple positional arguments makes
//! it cheap to add new env fields later without breaking every backend.
//! See DESIGN.md Phase P, D4: "環境情報は tick 開始時の immutable
//! snapshot を一括渡し".
//!
//! The `SpacecraftState` unification also keeps the host-side
//! `Observation` shape aligned with the Phase P1 WIT `record
//! spacecraft-state` that WASM guests will see.

use kaname::epoch::Epoch;

use crate::SpacecraftState;

/// Environment snapshot evaluated at the current tick instant.
///
/// Phase P0.5 keeps this empty so the plugin layer can compile without
/// pulling `tobari::magnetic` or other env models. Phase P1 will
/// populate it with fields needed by the first WASM guest:
///
/// - `magnetic_field_eci_t: Vector3<f64>`
/// - `sun_direction_eci: Vector3<f64>`
/// - `atmospheric_density_kgm3: f64`
/// - ...
///
/// The guarantee is: whatever ends up here, it is evaluated **once per
/// tick** and handed to the guest as an immutable snapshot. Guests never
/// call back into the host for environment values mid-tick.
#[derive(Debug, Clone, Default)]
pub struct EnvSnapshot {
    /// Placeholder field to match the WIT `env-snapshot` record which
    /// requires at least one field (Component Model / wit-bindgen 0.41+
    /// rule). Phase P3 will replace this with meaningful pre-computed
    /// environment values (magnetic field, sun direction, etc.).
    pub(crate) reserved: bool,
}

impl EnvSnapshot {
    /// Construct an empty snapshot (all placeholder fields at default).
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Per-tick input handed to a plugin controller's `update` call.
///
/// Borrowed references keep this zero-copy in the Phase P0.5 native
/// path. The Phase P1 WASM backend will serialize a matching
/// `SpacecraftState` / `EnvSnapshot` shape via `postcard + serde`
/// across the guest boundary (DESIGN.md Phase P, D4).
#[derive(Debug, Clone, Copy)]
pub struct Observation<'a> {
    /// Current simulation time \[s\] (seconds from the controller's
    /// reference t=0, not wall clock).
    pub t: f64,
    /// Complete spacecraft state: orbit + attitude + mass.
    pub spacecraft: &'a SpacecraftState,
    /// Absolute epoch, if the simulation is bound to a wall-clock time
    /// base (e.g. for ephemeris / magnetic-field models).
    pub epoch: Option<&'a Epoch>,
    /// Environment snapshot evaluated at this tick.
    pub env: &'a EnvSnapshot,
}
