//! `NullEop` does not implement `PolarMotion`, so passing it to
//! `Rotation::<Tirs, Itrs>::polar_motion` must be a compile error.

use kaname::earth::eop::NullEop;
use kaname::epoch::Epoch;
use kaname::frame::{Itrs, Rotation, Tirs};

fn main() {
    let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let tt = utc.to_tt();
    // This must fail: NullEop : !PolarMotion.
    let _w = Rotation::<Tirs, Itrs>::polar_motion(&tt, &utc, &NullEop);
}
