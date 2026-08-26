//! Frame-generic adapter for the tobari [`MagneticFieldModel`] trait.
//!
//! The model's [`MagneticFieldModel::field_ecef`] returns the field in
//! ECEF Cartesian coordinates. Callers inside orts work in an ECI frame,
//! so this module provides [`field_inertial`] which handles the full
//! round-trip via [`EarthFixedTransform`]:
//!
//! ```text
//! ECI position → ECEF (via EarthFixedTransform) → geodetic
//!   → field_ecef → ECEF field vector → ECI (inverse rotation)
//! ```

use arika::epoch::{Epoch, Utc};
use arika::frame::Vec3;
use tobari::magnetic::{MagneticFieldInput, MagneticFieldModel};

use arika::earth::{EarthFixedTransform, EarthOrientation};

/// Evaluate a magnetic field model and return the result in the
/// propagation frame `F`.
///
/// Uses [`EarthFixedTransform`] for the ECI↔ECEF conversion, so it works
/// with both `SimpleEci` (ERA rotation) and `Gcrs` (IAU 2006 chain).
/// `orientation` carries the instant and — for the frames that need it —
/// the EOP data.
pub fn field_inertial<F: EarthFixedTransform>(
    model: &dyn MagneticFieldModel,
    position: &Vec3<F>,
    orientation: &EarthOrientation<'_, F>,
) -> Vec3<F> {
    let geodetic = F::to_geodetic(position, orientation);
    let rot_to_eci = F::fixed_to_inertial(orientation);

    let b_ecef_arr = model.field_ecef(&MagneticFieldInput {
        geodetic,
        utc: orientation.utc(),
    });
    let b_ecef = Vec3::<F::Fixed>::new(b_ecef_arr[0], b_ecef_arr[1], b_ecef_arr[2]);
    rot_to_eci.transform(&b_ecef)
}

/// SimpleEci convenience wrapper for [`field_inertial`].
///
/// Retained for callers whose interface is itself typed in the simple ECI frame
/// — the plugin WASM host's `magnetic-field-eci` import, whose WIT signature
/// takes a bare ECI position. Frame-generic code (models, controllers, sensors)
/// calls [`field_inertial`] with its own frame instead.
pub fn field_eci(
    model: &dyn MagneticFieldModel,
    position_eci: &Vec3<arika::frame::SimpleEci>,
    epoch: &Epoch<Utc>,
) -> Vec3<arika::frame::SimpleEci> {
    field_inertial::<arika::frame::SimpleEci>(
        model,
        position_eci,
        &EarthOrientation::simple(*epoch),
    )
}
