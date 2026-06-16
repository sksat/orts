//! Time scale markers and the sealed [`TimeScale`] trait.

mod sealed {
    pub trait Sealed {}
}

/// A time scale marker.
///
/// Sealed: 新しい scale は arika 内でのみ追加できる。
pub trait TimeScale: sealed::Sealed {
    /// Human-readable scale name (e.g. "UTC", "TAI").
    const NAME: &'static str;
}

macro_rules! define_scale {
    ($name:ident, $display:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name;
        impl sealed::Sealed for $name {}
        impl TimeScale for $name {
            const NAME: &'static str = $display;
        }
    };
}

define_scale!(
    Utc,
    "UTC",
    "Coordinated Universal Time. Operational hybrid scale: rate = SI (TAI) \
     with leap seconds to stay within 0.9 s of UT1. 一般的な入口 scale。"
);
define_scale!(
    Tai,
    "TAI",
    "International Atomic Time. Proper-time-like scale realized by a global \
     ensemble of atomic clocks. `TT = TAI + 32.184 s`."
);
define_scale!(
    Tt,
    "TT",
    "Terrestrial Time. Coordinate-derived time (linear scale of TCG, \
     `dTT/dTCG = 1 - L_G`, `L_G = 6.969290134e-10`; IAU 2000 B1.9). \
     IAU 2006 precession と IAU 2000A/B nutation の独立変数。"
);
define_scale!(
    Ut1,
    "UT1",
    "Universal Time (UT1). Earth rotation angle time scale — defining \
     observable は ERA (IAU 2000 B1.8 / SOFA iauEra00)。atomic clock が \
     刻む時刻ではなく Earth の瞬間的な向きを時間単位で表現したもの。"
);
define_scale!(
    Tdb,
    "TDB",
    "Barycentric Dynamical Time. Coordinate-derived time (linear scale of TCB; \
     IAU 2006 Resolution B3)。Meeus / JPL DE (Teph ≈ TDB) ephemeris と \
     IAU 2009 body rotation の formally な独立変数。"
);
