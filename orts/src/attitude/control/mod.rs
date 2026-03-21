pub mod bdot;
pub mod pd_controller;
pub mod reference;

pub use bdot::{BdotDetumbler, BdotFiniteDiff, CommandedMagnetorquer};
pub use pd_controller::{InertialPdController, TrackingPdController};
pub use reference::{AttitudeReference, InertialPointing, NadirPointing};
