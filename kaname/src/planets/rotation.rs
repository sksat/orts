//! IAU 2009 WGCCRE rotation models for the planets.
//!
//! Currently only Mars has a populated entry; Jupiter / Saturn / Uranus /
//! Neptune constants can be added later as needed.

use crate::rotation::IauRotationModel;

/// IAU 2009 rotation model for Mars.
pub const MARS: IauRotationModel = IauRotationModel {
    alpha0: 317.68143,
    alpha1: -0.1061,
    delta0: 52.8865,
    delta1: -0.0609,
    w0: 176.630,
    wd: 350.89198226,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::{Epoch, Tdb};

    #[test]
    fn mars_rotation_period_approximately_24h37m() {
        // Mars sidereal rotation period ≈ 24h 37m 22s ≈ 88642 seconds.
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0).to_tdb();
        let w0 = MARS.prime_meridian_angle(&epoch0);

        let mars_day_s = 88642.0;
        let epoch1 = Epoch::<Tdb>::from_jd_tdb(epoch0.jd() + mars_day_s / 86400.0);
        let w1 = MARS.prime_meridian_angle(&epoch1);

        let dw = (w1 - w0).to_degrees();
        assert!(
            (dw - 360.0).abs() < 1.0,
            "Mars should rotate ~360° in one sol, got {dw:.2}°"
        );
    }
}
