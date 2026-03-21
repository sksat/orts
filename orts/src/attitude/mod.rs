pub mod augmented;
pub mod control;
pub mod decoupled;
pub mod gravity_gradient;
pub mod state;
pub mod system;

pub use augmented::AugmentedAttitudeSystem;
pub use control::*;
pub use decoupled::{DecoupledAttitudeSystem, DecoupledContext};
pub use gravity_gradient::GravityGradientTorque;
pub use state::AttitudeState;
pub use system::AttitudeSystem;
