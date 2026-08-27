//! A model reads the attitude out of the state and reports loads in the frame it
//! is installed for. Those are the same frame, and `HasFrame::Frame` is what says
//! so: `S::Frame` and the loads frame `F` are one type parameter, so installing
//! this model for a `Gcrs` state while asking it for `SimpleEci` loads is a
//! compile error.
//!
//! What the bound replaced: the state carried no frame, so the model body named
//! one by hand (`rotation_tagged_as::<F>()`), which produced loads in `F` for
//! *any* state. The body typechecked either way and the mismatch was invisible —
//! a `Gcrs` attitude tagged as simple-ECI, ~484 arcsec of silent error at 2024.
use arika::frame::{Eci, Gcrs, SimpleEci};
use nalgebra::Vector3;
use orts::SpacecraftState;
use orts::model::{ExternalLoads, HasAttitude, HasFrame, Model};

struct BodyThrust;

impl<F: Eci, S: HasFrame<Frame = F> + HasAttitude> Model<S, F> for BodyThrust {
    fn name(&self) -> &str {
        "body_thrust"
    }
    fn eval(
        &self,
        _t: f64,
        state: &S,
        _epoch: Option<&arika::epoch::Epoch>,
    ) -> ExternalLoads<F> {
        let a_body = arika::frame::Vec3::from_raw(Vector3::new(0.0, 0.0, 1e-3));
        ExternalLoads {
            acceleration_inertial: state.attitude_to_inertial().transform(&a_body),
            torque_body: arika::frame::Vec3::zeros(),
            mass_rate: 0.0,
        }
    }
}

/// Install `BodyThrust` for a `Gcrs` state while asking it for `SimpleEci` loads.
fn install_mismatched<M: Model<SpacecraftState<Gcrs>, SimpleEci>>(_m: M) {}

fn main() {
    // This must fail: `SpacecraftState<Gcrs>` has `HasFrame::Frame = Gcrs`,
    // which cannot unify with the `SimpleEci` loads frame.
    install_mismatched(BodyThrust);
}
