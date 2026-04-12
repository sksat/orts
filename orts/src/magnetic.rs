//! Temporary adapter for the tobari [`MagneticFieldModel`] trait.
//!
//! TODO: Phase 4B-1 で OrbitalState<F> / AtmosphereFrame trait を導入したら、
//! この SimpleEci-hardcoded adapter は frame-generic な変換に統合して削除する。
//!
//! The model's [`MagneticFieldModel::field_ecef`] returns the field in
//! ECEF Cartesian coordinates.  Callers inside orts typically work in
//! the ECI frame, so this module provides [`field_eci`] which handles
//! the full round-trip:
//!
//! ```text
//! ECI position → ECEF (via GMST rotation) → geodetic
//!   → field_ecef → ECEF field vector → ECI (inverse rotation)
//! ```

use arika::epoch::Epoch;
use arika::frame::{self, Rotation, Vec3};
use tobari::magnetic::{MagneticFieldInput, MagneticFieldModel};

/// Evaluate a magnetic field model and return the result in the ECI frame.
///
/// Performs the following chain:
/// 1. Rotate `position_eci` to ECEF using the GMST-based ERA rotation.
/// 2. Convert the ECEF position to geodetic coordinates (Bowring).
/// 3. Call [`MagneticFieldModel::field_ecef`].
/// 4. Rotate the resulting ECEF field vector back to ECI.
pub fn field_eci(
    model: &dyn MagneticFieldModel,
    position_eci: &Vec3<frame::SimpleEci>,
    epoch: &Epoch,
) -> Vec3<frame::SimpleEci> {
    let gmst = epoch.gmst();
    let rot_to_ecef = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(gmst);
    let rot_to_eci = Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(gmst);

    let ecef = rot_to_ecef.transform(position_eci);
    let geodetic = ecef.to_geodetic();

    let b_ecef_arr = model.field_ecef(&MagneticFieldInput {
        geodetic,
        utc: epoch,
    });
    let b_ecef = Vec3::<frame::SimpleEcef>::new(b_ecef_arr[0], b_ecef_arr[1], b_ecef_arr[2]);
    rot_to_eci.transform(&b_ecef)
}
