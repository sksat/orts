//! `SphericalHarmonicGravity<F: EarthFixedTransform>` carries its own frame:
//! the tesseral terms are rotated from `F::Fixed` into `F` with `F`'s
//! Earth-orientation chain and EOP storage. Installing a `Gcrs` field on a
//! `SimpleEci` state would evaluate the geoid at a longitude computed through
//! the wrong chain, so `impl<F: EarthFixedTransform, S: HasFrame<Frame = F> +
//! HasOrbit> Model<S>` must reject it.

use arika::frame::{Gcrs, SimpleEci};
use orts::SpacecraftState;
use orts::model::Model;
use orts::perturbations::SphericalHarmonicGravity;

/// Checks the bound by type alone — no field instance, so nothing but the
/// frame mismatch can make this file fail.
fn install_for_simple_eci<M: Model<SpacecraftState<SimpleEci>>>() {}

fn main() {
    install_for_simple_eci::<SphericalHarmonicGravity<Gcrs>>();
}
