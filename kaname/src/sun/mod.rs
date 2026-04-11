//! Sun: physical constants, analytical ephemeris, and IAU rotation model.
//!
//! # Submodules
//!
//! - [`ephemeris`] — Meeus analytical sun ephemeris + Equation of Time
//! - [`rotation`] — IAU 2009 WGCCRE rotation model (`SUN` const)
//!
//! The ephemeris/rotation submodules are kept private; all public items
//! are re-exported explicitly below so that new `pub` items added inside
//! `ephemeris.rs` do NOT automatically leak into `kaname::sun::*`.

mod ephemeris;
mod rotation;

pub use ephemeris::{
    equation_of_time, sun_direction_eci, sun_direction_from_body, sun_distance_from_body,
    sun_distance_km, sun_position_eci,
};
pub use rotation::SUN;

// ─── Physical constants ──────────────────────────────────────────

/// Sun gravitational parameter [km³/s²].
pub const MU: f64 = 132712440018.0;

/// One astronomical unit in kilometres.
pub const AU_KM: f64 = 149_597_870.7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mu_is_positive() {
        assert!(MU > 0.0);
    }

    #[test]
    fn mu_sun_greater_than_mu_earth() {
        assert!(MU > crate::earth::MU);
    }
}
