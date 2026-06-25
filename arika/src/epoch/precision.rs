//! Precision tiers for [`Epoch`](super::Epoch): the storage of its canonical
//! TAI instant.
//!
//! An `Epoch<S, P>` stores its TAI instant as `P::Repr`. The default tier
//! [`Precise`] carries a two-part JD ([`TwoPartJd`]) for sub-nanosecond
//! resolution; [`Coarse`] carries a single [`f64`], trading that resolution for
//! a lighter footprint (8 vs 16 bytes, no two-sum) on wasm / `no_std` embedded
//! targets where RAM, flash and cycles are scarce and ~tens-of-µs time is
//! enough. The tier is part of the type, so the choice is visible at every use
//! site and mixing is explicit — and it is zero runtime cost (monomorphized).
//!
//! The rich constructors (`from_jd`, `from_gregorian`, …) build the default
//! [`Precise`] tier; switch tier with
//! [`Epoch::to_precision`](super::Epoch::to_precision)
//! (e.g. `epoch.to_precision::<Coarse>()`).

use super::jd2::{JdRepr, TwoPartJd};

mod sealed {
    pub trait Sealed {}
}

/// The precision tier of an [`Epoch`](super::Epoch): selects the Julian Date
/// storage [`Repr`](Self::Repr) for its canonical TAI instant. Sealed — the only
/// tiers are [`Coarse`] and [`Precise`].
pub trait Precision: sealed::Sealed {
    /// The Julian Date storage representation for this tier.
    type Repr: JdRepr;

    /// Human-readable tier name (`"coarse"` / `"precise"`), for diagnostics.
    const NAME: &'static str;
}

/// Sub-nanosecond two-part tier (the default): stores a [`TwoPartJd`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Precise;

/// Single-`f64` tier: lighter for wasm / `no_std` embedded, ~tens-of-µs floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coarse;

impl sealed::Sealed for Precise {}
impl Precision for Precise {
    type Repr = TwoPartJd;
    const NAME: &'static str = "precise";
}

impl sealed::Sealed for Coarse {}
impl Precision for Coarse {
    type Repr = f64;
    const NAME: &'static str = "coarse";
}
