//! The combined `iau2006_full_from_utc` constructor requires the
//! strictest EOP bound (`Ut1Offset + NutationCorrections + PolarMotion`)
//! since it derives `tt` / `ut1` internally. `NullEop` satisfies none
//! of these traits, so the call must be rejected at compile time.
//! This is the "belt-and-braces" trybuild pin covering all three
//! capability traits at once.

use arika::earth::eop::NullEop;
use arika::epoch::Epoch;
use arika::frame::{Gcrs, Itrs, Rotation};

fn main() {
    let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    // This must fail: NullEop : !Ut1Offset + !NutationCorrections + !PolarMotion.
    let _m = Rotation::<Gcrs, Itrs>::iau2006_full_from_utc(&utc, &NullEop);
}
