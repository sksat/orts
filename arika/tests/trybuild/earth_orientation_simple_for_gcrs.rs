//! `EarthOrientation::simple` is constrained to `EopStorage = ()`, so it exists
//! only for a frame whose Earth-fixed transform needs no EOP data. `Gcrs`
//! needs a real EOP provider (`EopStorage = GcrsEopStorage`), so asking for its
//! "simple" orientation — i.e. claiming no EOP data is required — must be
//! rejected at compile time rather than silently substituting zeros.

use arika::earth::EarthOrientation;
use arika::epoch::Epoch;
use arika::frame::Gcrs;

fn main() {
    let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    // This must fail: <Gcrs as EarthFixedTransform>::EopStorage != ().
    let _o = EarthOrientation::<Gcrs>::simple(utc);
}
