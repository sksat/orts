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

/// Whether this crate has a magnetic field model for `body`.
///
/// [`tobari::magnetic::Igrf`] and [`tobari::magnetic::TiltedDipole`] are
/// Earth's, and they are the only models there are — which is not the same as
/// another body having no field. Callers that need a model for a body without
/// one use [`tobari::magnetic::NoField`], so a magnetometer reads zero and a
/// magnetorquer's `m × B` is zero instead of both taking Earth's field.
pub fn field_is_modelled(body: arika::body::KnownBody) -> bool {
    body == arika::body::KnownBody::Earth
}

/// The field model to serve for `body`.
///
/// [`TiltedDipole`](tobari::magnetic::TiltedDipole) on Earth — what the WASM
/// host's `magnetic-field-eci` has always used — and
/// [`NoField`](tobari::magnetic::NoField) where [`field_is_modelled`] says
/// there is none, so the answer is zero rather than Earth's field read at a
/// position that is not geocentric.
///
/// Both WASM backends call this, so the sync and async hosts cannot disagree.
pub fn field_for_body(body: arika::body::KnownBody) -> std::sync::Arc<dyn MagneticFieldModel> {
    if field_is_modelled(body) {
        std::sync::Arc::new(tobari::magnetic::TiltedDipole::earth())
    } else {
        std::sync::Arc::new(tobari::magnetic::NoField)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use arika::body::KnownBody;
    use arika::earth::geodetic::Geodetic;

    /// The model served for a body is that body's, on both WASM backends.
    ///
    /// Both host states call [`field_for_body`], so this ties the body to the
    /// model once for the sync and the async path — an async-only regression
    /// would have to route around this function.
    #[test]
    fn field_for_body_is_zero_where_no_model_exists() {
        let epoch = Epoch::<Utc>::j2000();
        let input = MagneticFieldInput {
            geodetic: Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 400.0,
            },
            utc: &epoch,
        };

        for body in [KnownBody::Mars, KnownBody::Moon, KnownBody::Sun] {
            assert_eq!(
                field_for_body(body).field_ecef(&input),
                [0.0, 0.0, 0.0],
                "{body:?} has no field model, so the host serves zero"
            );
        }

        let on_earth = field_for_body(KnownBody::Earth).field_ecef(&input);
        let magnitude =
            (on_earth[0] * on_earth[0] + on_earth[1] * on_earth[1] + on_earth[2] * on_earth[2])
                .sqrt();
        assert!(
            (1e-5..1e-4).contains(&magnitude),
            "Earth's field is modelled, so the host serves 20-60 µT, got {magnitude:.3e} T"
        );
    }
}
