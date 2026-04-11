//! Moon: physical constants, analytical ephemeris, and IAU rotation model.
//!
//! # Submodules
//!
//! - [`ephemeris`] — Meeus analytical Moon ephemeris + `MoonEphemeris` trait
//! - [`rotation`] — IAU 2009 WGCCRE rotation model (`MOON` const + libration)
//!
//! The submodules are kept private; all public items are re-exported
//! explicitly below so that new `pub` items added inside `ephemeris.rs`
//! or `rotation.rs` do NOT automatically leak into `arika::moon::*`.

mod ephemeris;
mod rotation;

pub use ephemeris::{HorizonsMoonEphemeris, MeeusMoonEphemeris, MoonEphemeris, moon_position_eci};
pub use rotation::{MOON, moon_orientation};

// ─── Physical constants ──────────────────────────────────────────

/// Moon gravitational parameter [km³/s²].
///
/// Source: IAU 2015 / JPL DE440. The extra significant digits (`.800066`)
/// beyond the nominal `4902.8` matter for long-duration lunar missions such
/// as Apollo 11 and Artemis 1, where the integrated effect of the tiny
/// fractional change accumulates over days of propagation.
pub const MU: f64 = 4902.800066;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mu_is_positive() {
        assert!(MU > 0.0);
    }

    #[test]
    fn mu_moon_less_than_mu_earth() {
        // Sanity cross-check: Moon is less massive than Earth.
        assert!(MU < crate::earth::MU);
    }
}
