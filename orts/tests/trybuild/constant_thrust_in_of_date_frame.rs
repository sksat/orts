//! `ConstantThrust` holds the burn direction fixed in its frame for the whole
//! burn, which is a claim about that frame's axes. The `Eci` category does not
//! make it: `Cirs` axes are the celestial intermediate pole and origin *of date*,
//! and `Teme`'s are the true equator of date, so components held constant in
//! either drift inertially while the burn runs.
//!
//! The model is therefore implemented per frame — `SimpleEci` and `Gcrs`, whose
//! axes are fixed to the precision it works at — rather than over `F: Eci`.
//! A blanket `impl<F: Eci, ..>` compiles this file, and a burn specified in
//! `Cirs` would then quietly rotate away from the direction that was asked for.

use arika::epoch::Epoch;
use arika::frame::{Cirs, Vec3};
use orts::SpacecraftState;
use orts::model::Model;
use orts::perturbations::ConstantThrust;

fn install_for_cirs<M: Model<SpacecraftState<Cirs>>>(_m: M) {}

fn main() {
    let dv = Vec3::<Cirs>::new(0.1, 0.0, 0.0);
    let burn = ConstantThrust::new(
        "dri",
        Epoch::from_jd(2_451_545.0),
        Epoch::from_jd(2_451_545.1),
        dv,
    );
    // This must fail: `Cirs` is an of-date frame, so `ConstantThrust` has no
    // `Model` impl for it.
    install_for_cirs(burn);
}
