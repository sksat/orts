//! `NullEop` does not implement `Ut1Offset`, so passing it to
//! `Epoch::<Utc>::to_ut1` must be a compile error. This pins the
//! guarantee that precise APIs cannot silently degrade to a no-op
//! provider.

use arika::earth::eop::NullEop;
use arika::epoch::Epoch;

fn main() {
    let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    // This must fail: NullEop : !Ut1Offset.
    let _ut1 = utc.to_ut1(&NullEop);
}
