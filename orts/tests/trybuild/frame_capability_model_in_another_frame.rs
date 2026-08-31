//! `Model` has no frame parameter of its own: it reports loads in the frame the
//! state is propagated in (`S::Frame`). So "a model whose loads frame differs
//! from its state frame" is no longer a type that can be written — which is why
//! the compile-fail case that used to guard it is gone rather than updated.
//!
//! The seam that remains is a model carrying its *own* frame: one bound to a
//! frame capability, like `AtmosphericDrag<F: EarthFixedTransform>`, which needs
//! `F::EopStorage` and `F::Fixed` to resolve the ECEF transform. Such a model
//! must be installed for a state in that same frame, and
//! `impl<F: EarthFixedTransform, S: HasFrame<Frame = F> + HasOrbit> Model<S>`
//! is what says so.
//!
//! This file is what that bound rejects: a drag model built for `Gcrs` handed to
//! a `SimpleEci` state, which would sample the atmosphere at a geodetic altitude
//! computed through the wrong Earth-rotation chain.

use arika::frame::{Gcrs, SimpleEci};
use orts::SpacecraftState;
use orts::model::Model;
use orts::perturbations::AtmosphericDrag;

/// Checks the bound by type alone — no drag instance, so nothing but the frame
/// mismatch can make this file fail.
fn install_for_simple_eci<M: Model<SpacecraftState<SimpleEci>>>() {}

fn main() {
    // This must fail: `AtmosphericDrag<Gcrs>` resolves its ECEF transform
    // through the IAU 2006 chain, and `SpacecraftState<SimpleEci>` has
    // `HasFrame::Frame = SimpleEci`. The model's frame and the state's frame
    // are one bound.
    install_for_simple_eci::<AtmosphericDrag<Gcrs>>();
}
