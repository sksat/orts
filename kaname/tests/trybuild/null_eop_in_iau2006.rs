//! `NullEop` does not implement `NutationCorrections`, so passing it
//! to `Rotation::<Gcrs, Cirs>::iau2006` must be a compile error.
//! Pins the Phase 3B guarantee that a placeholder EOP provider cannot
//! drive the precise IAU 2006 GCRS → CIRS constructor.

use kaname::earth::eop::NullEop;
use kaname::epoch::Epoch;
use kaname::frame::{Cirs, Gcrs, Rotation};

fn main() {
    let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let tt = utc.to_tt();
    // This must fail: NullEop : !NutationCorrections.
    let _q = Rotation::<Gcrs, Cirs>::iau2006(&tt, &utc, &NullEop);
}
