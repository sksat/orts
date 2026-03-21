pub mod bdot;
pub mod decoupled;
pub mod gravity_gradient;
pub mod pd_controller;
pub mod reference;
pub mod state;
pub mod system;

pub use bdot::{BdotDetumbler, BdotFiniteDiff, CommandedMagnetorquer};
pub use decoupled::{DecoupledAttitudeSystem, DecoupledContext};
pub use gravity_gradient::GravityGradientTorque;
pub use pd_controller::{InertialPdController, TrackingPdController};
pub use reference::{AttitudeReference, InertialPointing, NadirPointing};
pub use state::AttitudeState;
pub use system::AttitudeSystem;
