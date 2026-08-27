//! The v0 plugin WIT `attitude-body-to-inertial` payload carries four bare
//! floats whose frame is *defined* by the contract to be simple-ECI. The named
//! boundary operation therefore exists only on
//! `AttitudeBodyToInertial<SimpleEci>`: handing a `Gcrs` reading to a guest —
//! whose components denote a different physical attitude, off by
//! precession/nutation — must be a compile error, not silent nonsense.

use arika::frame::{Body, Gcrs, Rotation};
use nalgebra::UnitQuaternion;
use orts::plugin::tick_input::AttitudeBodyToInertial;

fn main() {
    let reading =
        AttitudeBodyToInertial::new(Rotation::<Body, Gcrs>::from_raw(UnitQuaternion::identity()));
    // This must fail: the v0 WIT payload is simple-ECI only.
    let _q = reading.to_wit_v0_simple_eci_quat();
}
