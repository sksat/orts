//! Float math compatibility for `no_std`.
//!
//! See `arika/src/math.rs` for the full rationale. When `std` is linked,
//! inherent `f64` methods shadow these trait methods with zero overhead.

#[allow(dead_code)]
pub(crate) trait F64Ext {
    fn abs(self) -> f64;
    fn sqrt(self) -> f64;
    fn powf(self, n: f64) -> f64;
    fn clamp(self, min: f64, max: f64) -> f64;
}

impl F64Ext for f64 {
    #[inline]
    fn abs(self) -> f64 {
        libm::fabs(self)
    }
    #[inline]
    fn sqrt(self) -> f64 {
        libm::sqrt(self)
    }
    #[inline]
    fn powf(self, n: f64) -> f64 {
        libm::pow(self, n)
    }
    #[inline]
    fn clamp(self, min: f64, max: f64) -> f64 {
        if self < min {
            min
        } else if self > max {
            max
        } else {
            self
        }
    }
}
