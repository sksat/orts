//! Float math compatibility for `no_std`.
//!
//! In `#![no_std]` mode, `f64` inherent methods like `sin()`, `cos()`, `sqrt()`
//! are not available because they live in `std`. This module provides an
//! extension trait [`F64Ext`] that delegates to [`libm`] when `std` is absent.
//!
//! When the `std` feature is enabled, the inherent methods on `f64` take
//! priority over trait methods (Rust method resolution rules), so existing
//! call sites like `x.sin()` continue to use the std implementation with
//! zero overhead.

#[allow(dead_code)]
/// Extension trait providing transcendental math functions on `f64` for `no_std`.
///
/// Import this trait (`use crate::math::F64Ext;`) in any module that calls
/// `.sin()`, `.cos()`, `.sqrt()`, `.atan2()`, etc. on `f64` values.
///
/// When `std` is linked, inherent methods on `f64` shadow these trait methods,
/// so there is no overhead or ambiguity.
pub(crate) trait F64Ext {
    fn sin(self) -> f64;
    fn cos(self) -> f64;
    fn tan(self) -> f64;
    fn asin(self) -> f64;
    fn acos(self) -> f64;
    fn atan(self) -> f64;
    fn atan2(self, other: f64) -> f64;
    fn sqrt(self) -> f64;
    fn abs(self) -> f64;
    fn powi(self, n: i32) -> f64;
    fn powf(self, n: f64) -> f64;
    fn exp(self) -> f64;
    fn ln(self) -> f64;
    fn signum(self) -> f64;
    fn floor(self) -> f64;
    fn ceil(self) -> f64;
    fn round(self) -> f64;
    fn sin_cos(self) -> (f64, f64);
    fn to_radians(self) -> f64;
    fn to_degrees(self) -> f64;
}

impl F64Ext for f64 {
    #[inline]
    fn sin(self) -> f64 {
        libm::sin(self)
    }
    #[inline]
    fn cos(self) -> f64 {
        libm::cos(self)
    }
    #[inline]
    fn tan(self) -> f64 {
        libm::tan(self)
    }
    #[inline]
    fn asin(self) -> f64 {
        libm::asin(self)
    }
    #[inline]
    fn acos(self) -> f64 {
        libm::acos(self)
    }
    #[inline]
    fn atan(self) -> f64 {
        libm::atan(self)
    }
    #[inline]
    fn atan2(self, other: f64) -> f64 {
        libm::atan2(self, other)
    }
    #[inline]
    fn sqrt(self) -> f64 {
        libm::sqrt(self)
    }
    #[inline]
    fn abs(self) -> f64 {
        libm::fabs(self)
    }
    #[inline]
    fn powi(self, n: i32) -> f64 {
        libm::pow(self, n as f64)
    }
    #[inline]
    fn powf(self, n: f64) -> f64 {
        libm::pow(self, n)
    }
    #[inline]
    fn exp(self) -> f64 {
        libm::exp(self)
    }
    #[inline]
    fn ln(self) -> f64 {
        libm::log(self)
    }
    #[inline]
    fn signum(self) -> f64 {
        if self > 0.0 {
            1.0
        } else if self < 0.0 {
            -1.0
        } else if self.is_nan() {
            f64::NAN
        } else {
            // Preserve sign of zero: +0.0 → +1.0, -0.0 → -1.0
            // (matches std behavior)
            if self.is_sign_positive() { 1.0 } else { -1.0 }
        }
    }
    #[inline]
    fn floor(self) -> f64 {
        libm::floor(self)
    }
    #[inline]
    fn ceil(self) -> f64 {
        libm::ceil(self)
    }
    #[inline]
    fn round(self) -> f64 {
        libm::round(self)
    }
    #[inline]
    fn sin_cos(self) -> (f64, f64) {
        libm::sincos(self)
    }
    #[inline]
    fn to_radians(self) -> f64 {
        self * (core::f64::consts::PI / 180.0)
    }
    #[inline]
    fn to_degrees(self) -> f64 {
        self * (180.0 / core::f64::consts::PI)
    }
}
