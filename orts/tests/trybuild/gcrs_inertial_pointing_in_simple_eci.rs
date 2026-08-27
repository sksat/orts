//! An inertial-hold target is a quaternion, and its components mean different
//! physical attitudes in different inertial frames. `InertialPointing<Gcrs>`
//! therefore implements `AttitudeReference<Gcrs>` only — asking it for a target
//! in `SimpleEci` must be rejected at compile time.

use arika::frame::{Body, Gcrs, Rotation, SimpleEci};
use nalgebra::UnitQuaternion;
use orts::attitude::control::{AttitudeReference, InertialPointing};
use orts::orbital::OrbitalState;
use nalgebra::Vector3;

fn main() {
    let reference = InertialPointing {
        target_q: Rotation::<Body, Gcrs>::from_raw(UnitQuaternion::identity()),
    };
    let orbit = OrbitalState::<SimpleEci>::new_in_frame(
        Vector3::new(7000.0, 0.0, 0.0),
        Vector3::new(0.0, 7.5, 0.0),
    );
    // This must fail: `InertialPointing<Gcrs>: AttitudeReference<SimpleEci>`
    // does not hold.
    let _ = AttitudeReference::<SimpleEci>::target(&reference, 0.0, &orbit, None);
}
