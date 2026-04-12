//! Frame-generic adapter for the tobari [`MagneticFieldModel`] trait.
//!
//! The model's [`MagneticFieldModel::field_ecef`] returns the field in
//! ECEF Cartesian coordinates. Callers inside orts work in an ECI frame,
//! so this module provides [`field_inertial`] which handles the full
//! round-trip via [`EarthFrameBridge`]:
//!
//! ```text
//! ECI position → ECEF (via EarthFrameBridge) → geodetic
//!   → field_ecef → ECEF field vector → ECI (inverse rotation)
//! ```

use arika::epoch::{Epoch, Utc};
use arika::frame::Vec3;
use tobari::magnetic::{MagneticFieldInput, MagneticFieldModel};

use crate::environment::EarthFrameBridge;

/// Evaluate a magnetic field model and return the result in the
/// propagation frame `F`.
///
/// Uses [`EarthFrameBridge`] for the ECI↔ECEF conversion, so it works
/// with both `SimpleEci` (ERA rotation) and `Gcrs` (IAU 2006 chain).
pub fn field_inertial<F: EarthFrameBridge>(
    model: &dyn MagneticFieldModel,
    position: &Vec3<F>,
    epoch: &Epoch<Utc>,
    eop: &F::EopStorage,
) -> Vec3<F> {
    let geodetic = F::to_geodetic(position, epoch, eop);
    let rot_to_eci = F::fixed_to_inertial(epoch, eop);

    let b_ecef_arr = model.field_ecef(&MagneticFieldInput {
        geodetic,
        utc: epoch,
    });
    let b_ecef = Vec3::<F::Fixed>::new(b_ecef_arr[0], b_ecef_arr[1], b_ecef_arr[2]);
    rot_to_eci.transform(&b_ecef)
}

/// SimpleEci convenience wrapper for [`field_inertial`].
///
/// Retained for callers that are `Frame = SimpleEci` constrained
/// (bdot controllers, magnetometer sensor, plugin WASM host).
pub fn field_eci(
    model: &dyn MagneticFieldModel,
    position_eci: &Vec3<arika::frame::SimpleEci>,
    epoch: &Epoch<Utc>,
) -> Vec3<arika::frame::SimpleEci> {
    field_inertial::<arika::frame::SimpleEci>(model, position_eci, epoch, &())
}
