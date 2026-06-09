//! Orbit Mean-Elements Message (OMM) — the shared mean-element record.
//!
//! [`Omm`] is the in-memory model that every element-set format decodes into
//! (TLE in [`crate::tle`], OMM JSON/KVN/XML in the submodules here). It mirrors
//! the CCSDS OMM mean-element set: a satellite identity, an epoch, and the six
//! SGP4 mean Keplerian elements plus the B* drag term.
//!
//! Angles are stored in **radians** and mean motion in **rad/s** (orts
//! conventions), converted from each format's native units (degrees, rev/day)
//! at parse time. Convert to classical elements with
//! [`Omm::to_keplerian_elements`].

pub mod json;
pub mod kvn;
pub mod xml;

use alloc::string::String;

// In `no_std` builds f64 transcendentals (`cbrt`) resolve via libm through this
// trait; under `std` the inherent methods shadow it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::epoch::{Epoch, Utc};
use crate::kepler::{KeplerianElements, mean_to_true_anomaly};

/// Mean orbital element set (CCSDS OMM data model).
///
/// The canonical output of all element-set parsers. TLE is treated as a legacy
/// serialization of the same mean-element data, so [`crate::tle`] also produces
/// an `Omm`.
#[derive(Debug, Clone, PartialEq)]
pub struct Omm {
    /// `OBJECT_NAME` — satellite name, if present.
    pub object_name: Option<String>,
    /// `OBJECT_ID` — international designator (e.g. `"1998-067A"`), if present.
    pub object_id: Option<String>,
    /// `NORAD_CAT_ID` — catalog number. Alpha-5 alphanumeric ids are decoded to
    /// their numeric value (e.g. `"A0000"` → `100000`).
    pub norad_cat_id: u32,
    /// Element-set epoch (UTC).
    pub epoch: Epoch<Utc>,
    /// Mean motion [rad/s].
    pub mean_motion: f64,
    /// Eccentricity (dimensionless).
    pub eccentricity: f64,
    /// Inclination [rad].
    pub inclination: f64,
    /// Right ascension of the ascending node [rad].
    pub raan: f64,
    /// Argument of perigee [rad].
    pub argument_of_perigee: f64,
    /// Mean anomaly [rad].
    pub mean_anomaly: f64,
    /// B* drag term [1/earth radii].
    pub bstar: f64,
}

impl Omm {
    /// Semi-major axis [km] from mean motion: `a = (μ/n²)^(1/3)`.
    pub fn semi_major_axis(&self, mu: f64) -> f64 {
        (mu / (self.mean_motion * self.mean_motion)).cbrt()
    }

    /// Convert to classical Keplerian elements.
    ///
    /// Derives the semi-major axis from mean motion and converts the mean
    /// anomaly to true anomaly via Kepler's equation.
    pub fn to_keplerian_elements(&self, mu: f64) -> KeplerianElements {
        KeplerianElements {
            semi_major_axis: self.semi_major_axis(mu),
            eccentricity: self.eccentricity,
            inclination: self.inclination,
            raan: self.raan,
            argument_of_periapsis: self.argument_of_perigee,
            true_anomaly: mean_to_true_anomaly(self.mean_anomaly, self.eccentricity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;
    use core::f64::consts::PI;

    /// ISS-like OMM, values from the canonical ISS TLE at epoch 2024-079.5.
    fn iss_omm() -> Omm {
        Omm {
            object_name: Some(String::from("ISS (ZARYA)")),
            object_id: Some(String::from("1998-067A")),
            norad_cat_id: 25544,
            epoch: Epoch::from_tle_epoch(24, 79.5),
            mean_motion: 15.49561654 * 2.0 * PI / 86400.0,
            eccentricity: 0.0007417,
            inclination: 51.6400_f64.to_radians(),
            raan: 208.6520_f64.to_radians(),
            argument_of_perigee: 35.3910_f64.to_radians(),
            mean_anomaly: 324.7580_f64.to_radians(),
            bstar: 3.0e-5,
        }
    }

    #[test]
    fn iss_semi_major_axis() {
        let a = iss_omm().semi_major_axis(MU_EARTH);
        // ISS orbits at ~420 km altitude → a ≈ 6796 km.
        assert!(
            (a - 6796.0).abs() < 5.0,
            "ISS semi-major axis should be ≈6796 km, got {a}"
        );
    }

    #[test]
    fn iss_to_keplerian_elements() {
        let omm = iss_omm();
        let kep = omm.to_keplerian_elements(MU_EARTH);

        assert!((kep.semi_major_axis - omm.semi_major_axis(MU_EARTH)).abs() < 1e-9);
        assert_eq!(kep.eccentricity, omm.eccentricity);
        assert_eq!(kep.inclination, omm.inclination);
        assert_eq!(kep.raan, omm.raan);
        assert_eq!(kep.argument_of_periapsis, omm.argument_of_perigee);
        // Near-circular orbit (e ≈ 0.0007): true anomaly ≈ mean anomaly.
        let d_nu = (kep.true_anomaly - omm.mean_anomaly).abs();
        assert!(
            d_nu < 0.01,
            "ν should be ≈ M for near-circular orbit, Δ={d_nu}"
        );
    }
}
