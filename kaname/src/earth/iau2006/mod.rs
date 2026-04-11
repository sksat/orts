//! IAU 2006 / 2000A_R06 precession-nutation supporting math.
//!
//! # Scope
//!
//! This module provides the pure-math building blocks for the IAU 2006 CIO-based
//! Earth rotation chain. In Phase 3 it is populated incrementally:
//!
//! - **Phase 3A-1**: typed angle primitives, fundamental arguments
//!   `F1..F14`, and IAU 2006 precession polynomial expressions from
//!   [IERS Conventions 2010 TN36](https://www.iers.org/IERS/EN/Publications/TechnicalNotes/tn36.html)
//!   Eq. (5.39), (5.40), (5.43), (5.44)
//! - **Phase 3A-2**: CIP `X`, `Y` and CIO locator `s + XY/2` series
//!   generated from the IERS electronic tables `tab5.2a.txt`,
//!   `tab5.2b.txt`, `tab5.2d.txt` — stored as the crate-private
//!   `tables_gen` submodule. The generator lives at
//!   `kaname/tools/generate_iau2006_tables.py`
//! - **Phase 3A-3** (this commit): [`cip`] evaluators — `cip_xy`,
//!   `cio_locator_s`, `cip_coordinates` — that consume the generated
//!   series and return `Rad`-typed CIP coordinates + CIO locator
//! - **Phase 3A-4**: GCRS→CIRS matrix composition using the CIP
//!   coordinates from `cip` plus the Fukushima-Williams precession
//!   angles from [`precession`]
//! - **Phase 3B**: `Rotation<Gcrs, Cirs>::iau2006(tt, utc, eop)` and
//!   related public constructors
//!
//! No public [`crate::frame::Rotation`] constructors are exposed from this
//! module yet — they appear in Phase 3B.
//!
//! # Independent variable
//!
//! All polynomial expressions in this module take `t = (JD_TT − 2451545.0) /
//! 36525`, i.e. Julian centuries of TT since J2000.0. Callers obtain this via
//! [`crate::epoch::Epoch::<crate::epoch::Tt>::centuries_since_j2000`].
//!
//! # Type-safe angle units
//!
//! The IERS Conventions mix arcseconds, milliarcseconds, and microarcseconds.
//! To prevent silent unit confusion, angular quantities in this module are
//! represented as [`Angle<U>`] where `U` is a phantom [`AngleUnit`] marker
//! ([`Radians`], [`Arcseconds`], [`Milliarcseconds`], [`Microarcseconds`]).
//!
//! - Type aliases [`Rad`], [`Arcsec`], [`Mas`], [`Uas`] are provided for
//!   convenience
//! - Conversion to radians is always explicit via
//!   [`Angle::to_radians`]
//! - Addition / subtraction are only defined between angles of the **same**
//!   unit — mixing requires explicit conversion
//! - Scalar multiplication (`Angle<U> * f64`) preserves the unit
//!
//! Example:
//! ```
//! use kaname::earth::iau2006::{Angle, Arcsec, Rad};
//! let ra_arcsec = Arcsec::new(3600.0);
//! let ra_rad: Rad = ra_arcsec.to_radians();
//! assert!((ra_rad.raw() - std::f64::consts::PI / 180.0).abs() < 1e-15);
//! ```

use std::f64::consts::PI;
use std::marker::PhantomData;
use std::ops::{Add, Div, Mul, Neg, Sub};

pub mod cip;
pub mod fundamental_arguments;
pub mod precession;
pub(crate) mod tables_gen;
#[cfg(test)]
mod tables_pin;

// ─── Angular-unit markers ────────────────────────────────────────

mod sealed {
    pub trait Sealed {}
}

/// Marker trait for an angular unit.
///
/// Implemented by [`Radians`], [`Arcseconds`], [`Milliarcseconds`],
/// [`Microarcseconds`]. Sealed so that new units can only be added inside
/// kaname (to keep the conversion lattice consistent).
pub trait AngleUnit: sealed::Sealed {
    /// Conversion factor from this unit to radians: one unit of `Self`
    /// equals `TO_RADIANS` radians.
    const TO_RADIANS: f64;
    /// Short human-readable name (e.g. `"rad"`, `"arcsec"`).
    const NAME: &'static str;
}

// Unit markers are pure phantom types: they are only ever used as the
// generic parameter `U` of `Angle<U>` and never instantiated as values, so
// they need no derived traits.

/// Radian angular unit (the canonical internal form).
pub struct Radians;
impl sealed::Sealed for Radians {}
impl AngleUnit for Radians {
    const TO_RADIANS: f64 = 1.0;
    const NAME: &'static str = "rad";
}

/// Arcsecond angular unit: `1 arcsec = π / 648_000 rad`.
pub struct Arcseconds;
impl sealed::Sealed for Arcseconds {}
impl AngleUnit for Arcseconds {
    const TO_RADIANS: f64 = DAS2R;
    const NAME: &'static str = "arcsec";
}

/// Milliarcsecond angular unit: `1 mas = 1e-3 arcsec`.
pub struct Milliarcseconds;
impl sealed::Sealed for Milliarcseconds {}
impl AngleUnit for Milliarcseconds {
    const TO_RADIANS: f64 = DMAS2R;
    const NAME: &'static str = "mas";
}

/// Microarcsecond angular unit: `1 µas = 1e-6 arcsec`.
pub struct Microarcseconds;
impl sealed::Sealed for Microarcseconds {}
impl AngleUnit for Microarcseconds {
    const TO_RADIANS: f64 = DUAS2R;
    const NAME: &'static str = "µas";
}

// ─── Angle<U> ────────────────────────────────────────────────────

/// A scalar angle tagged with its unit `U`.
///
/// `Angle<U>` is `#[repr(transparent)]` around `f64` so it compiles to a
/// bare float at runtime. Arithmetic operators are implemented between
/// angles of the **same** unit only; conversions are explicit via
/// [`Angle::to_radians`]. `Clone` and `Copy` are implemented manually so
/// that they do not require the unit markers (`Radians`, `Arcseconds`,
/// …) to be `Clone` / `Copy` themselves — those markers are pure
/// phantom types and deliberately carry no derives.
#[repr(transparent)]
pub struct Angle<U: AngleUnit>(f64, PhantomData<U>);

impl<U: AngleUnit> Clone for Angle<U> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<U: AngleUnit> Copy for Angle<U> {}

/// Type alias for an angle in radians.
pub type Rad = Angle<Radians>;
/// Type alias for an angle in arcseconds.
pub type Arcsec = Angle<Arcseconds>;
/// Type alias for an angle in milliarcseconds.
pub type Mas = Angle<Milliarcseconds>;
/// Type alias for an angle in microarcseconds.
pub type Uas = Angle<Microarcseconds>;

impl<U: AngleUnit> Angle<U> {
    /// Construct an angle directly from a raw scalar value interpreted in
    /// the unit `U`.
    #[inline]
    pub const fn new(value: f64) -> Self {
        Self(value, PhantomData)
    }

    /// Return the raw underlying value, interpreted in the unit `U`.
    #[inline]
    pub const fn raw(self) -> f64 {
        self.0
    }

    /// The zero angle in this unit.
    #[inline]
    pub const fn zero() -> Self {
        Self::new(0.0)
    }

    /// Convert to radians. Multiplies by `U::TO_RADIANS`.
    #[inline]
    pub fn to_radians(self) -> Rad {
        Rad::new(self.0 * U::TO_RADIANS)
    }

    /// True if the wrapped `f64` is finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl Rad {
    /// Sine of the angle.
    #[inline]
    pub fn sin(self) -> f64 {
        self.0.sin()
    }
    /// Cosine of the angle.
    #[inline]
    pub fn cos(self) -> f64 {
        self.0.cos()
    }
    /// Tangent of the angle.
    #[inline]
    pub fn tan(self) -> f64 {
        self.0.tan()
    }
}

// Debug prints the unit name so that `Arcsec` vs `Rad` is visible in test
// failures, without needing `std::any::type_name`.
impl<U: AngleUnit> std::fmt::Debug for Angle<U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Angle<{}>({})", U::NAME, self.0)
    }
}

// Same-unit arithmetic.
impl<U: AngleUnit> Add for Angle<U> {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.0 + rhs.0)
    }
}
impl<U: AngleUnit> Sub for Angle<U> {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.0 - rhs.0)
    }
}
impl<U: AngleUnit> Neg for Angle<U> {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.0)
    }
}
impl<U: AngleUnit> Mul<f64> for Angle<U> {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f64) -> Self {
        Self::new(self.0 * rhs)
    }
}
impl<U: AngleUnit> Mul<Angle<U>> for f64 {
    type Output = Angle<U>;
    #[inline]
    fn mul(self, rhs: Angle<U>) -> Angle<U> {
        Angle::new(self * rhs.0)
    }
}
impl<U: AngleUnit> Div<f64> for Angle<U> {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f64) -> Self {
        Self::new(self.0 / rhs)
    }
}

// ─── Angular-unit conversion constants ───────────────────────────

/// Arcseconds → radians. `PI / (180 × 3600)`.
///
/// SOFA convention `DAS2R = 4.848136811095359935899141e-6`.
pub const DAS2R: f64 = PI / (180.0 * 3600.0);

/// Milliarcseconds → radians. `DAS2R × 1e−3`.
pub const DMAS2R: f64 = DAS2R * 1e-3;

/// Microarcseconds → radians. `DAS2R × 1e−6`.
pub const DUAS2R: f64 = DAS2R * 1e-6;

/// Arcseconds in a full turn. `360 × 3600 = 1_296_000`.
///
/// Used to reduce polynomial evaluations in arcseconds modulo a full
/// circle before converting to radians, matching SOFA's `TURNAS`.
pub const TURNAS: f64 = 1_296_000.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the SOFA-published value of `DAS2R` (= π / (180 × 3600)) to
    /// 15 significant digits. Meaningful because it exercises the Rust
    /// `std::f64::consts::PI` constant and catches any future change to
    /// the definition of `DAS2R`.
    #[test]
    fn das2r_matches_sofa_published_value() {
        // SOFA / ERFA: DAS2R = 4.848136811095359935899141e-6
        let sofa_published = 4.848_136_811_095_359_9e-6;
        assert!(
            (DAS2R - sofa_published).abs() < 1e-22,
            "DAS2R = {DAS2R}, expected {sofa_published}"
        );
    }

    /// 1° = 3600 arcsec. Exercises `Arcseconds::TO_RADIANS` and the
    /// generic `to_radians` path.
    #[test]
    fn one_degree_roundtrips_via_arcsec() {
        let one_deg = Arcsec::new(3600.0).to_radians();
        assert!((one_deg.raw() - PI / 180.0).abs() < 1e-15);
    }

    /// 1000 mas = 1_000_000 µas = 1 arcsec (in radians). Non-trivially
    /// tests all three sub-arcsec units against each other.
    #[test]
    fn sub_arcsec_units_agree_with_arcsec() {
        let one_arcsec = Arcsec::new(1.0).to_radians();
        let mas_form = Mas::new(1000.0).to_radians();
        let uas_form = Uas::new(1_000_000.0).to_radians();
        assert!((mas_form.raw() - one_arcsec.raw()).abs() < 1e-20);
        assert!((uas_form.raw() - one_arcsec.raw()).abs() < 1e-18);
    }

    /// Test that same-unit arithmetic composes correctly. This is not
    /// just a wrapper sanity check: `Add` / `Sub` / `Mul<f64>` / `Neg`
    /// are hand-written `impl` blocks and a regression here would
    /// silently change the polynomial evaluators that rely on them.
    #[test]
    fn arithmetic_operators_match_underlying_f64() {
        let a = Arcsec::new(10.0);
        let b = Arcsec::new(3.0);
        assert_eq!((a + b).raw(), 13.0);
        assert_eq!((a - b).raw(), 7.0);
        assert_eq!((-a).raw(), -10.0);
        assert_eq!((a * 2.5).raw(), 25.0);
        assert_eq!((2.5 * a).raw(), 25.0);
        assert_eq!((a / 4.0).raw(), 2.5);
    }

    /// Confirm `Rad::sin` / `Rad::cos` forward to the `f64` math library.
    /// sin(π/2) = 1 and cos(π/2) = 0 are not a tautology: they catch a
    /// future refactor that accidentally doubles the angle or negates it.
    #[test]
    fn rad_trig_uses_radian_argument() {
        let r = Rad::new(PI / 2.0);
        assert!((r.sin() - 1.0).abs() < 1e-15);
        assert!(r.cos().abs() < 1e-15);
    }
}
