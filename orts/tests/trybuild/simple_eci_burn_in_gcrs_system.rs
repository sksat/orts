//! `ConstantThrust` holds the burn's Δv direction fixed in the inertial frame it
//! was given in, so that frame is part of the model's type. Installing a burn
//! specified in `SimpleEci` into a `Gcrs` propagation would point the thrust
//! ~484 arcsec (2024) off the intended direction for the whole burn.
//!
//! Before the Δv was frame-typed, the stored vector was a bare `Vector3<f64>`
//! and this compiled: the loads came back labelled with whatever frame the state
//! used, without rotating the numbers.

use arika::epoch::Epoch;
use arika::frame::{Gcrs, SimpleEci, Vec3};
use orts::SpacecraftState;
use orts::model::Model;
use orts::perturbations::ConstantThrust;

fn install_for_gcrs<M: Model<SpacecraftState<Gcrs>>>(_m: M) {}

fn main() {
    let dv = Vec3::<SimpleEci>::new(0.1, 0.0, 0.0);
    let burn = ConstantThrust::new(
        "dri",
        Epoch::from_jd(2_451_545.0),
        Epoch::from_jd(2_451_545.1),
        dv,
    );
    // This must fail: the Δv is in `SimpleEci`, the propagation is in `Gcrs`.
    install_for_gcrs(burn);
}
