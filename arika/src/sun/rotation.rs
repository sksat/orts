//! IAU 2009 WGCCRE rotation model for the Sun.

use crate::rotation::IauRotationModel;

/// IAU 2009 rotation model for the Sun.
pub const SUN: IauRotationModel = IauRotationModel {
    alpha0: 286.13,
    alpha1: 0.0,
    delta0: 63.87,
    delta1: 0.0,
    w0: 84.176,
    wd: 14.1844000,
};
