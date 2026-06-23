//! Time scale markers and the sealed [`TimeScale`] trait.

mod sealed {
    pub trait Sealed {}
}

/// Category of a time scale, exposed as introspection metadata.
///
/// This is **descriptive only** — it is deliberately not used as a trait
/// bound. Conversion capability is modelled by the
/// [`FixedOffsetFromTai`](super::FixedOffsetFromTai) /  `TaiConvertible`
/// (crate-internal) traits instead, because what generic conversion code
/// needs to dispatch on is "how does this scale bridge to TAI", not its
/// physical category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeScaleKind {
    /// Atomic-clock-realized scale (TAI, GPS).
    Atomic,
    /// Coordinate-derived dynamical scale (TT, TDB) — a defined linear scale
    /// of a relativistic coordinate time.
    CoordinateDerived,
    /// Earth-rotation scale (UT1) — realized by Earth's orientation, not
    /// atomic clocks; reachable only through an EOP `dUT1` provider.
    EarthRotation,
    /// Operational hybrid scale (UTC) — SI rate with inserted leap seconds.
    Hybrid,
}

/// A time scale marker.
///
/// Sealed: 新しい scale は arika 内でのみ追加できる。
pub trait TimeScale: sealed::Sealed {
    /// Human-readable scale name (e.g. "UTC", "TAI").
    const NAME: &'static str;
    /// Descriptive category of this scale (introspection metadata).
    const KIND: TimeScaleKind;
}

macro_rules! define_scale {
    ($name:ident, $display:expr, $kind:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name;
        impl sealed::Sealed for $name {}
        impl TimeScale for $name {
            const NAME: &'static str = $display;
            const KIND: TimeScaleKind = $kind;
        }
    };
}

define_scale!(
    Utc,
    "UTC",
    TimeScaleKind::Hybrid,
    "Coordinated Universal Time. Operational hybrid scale: rate = SI (TAI) \
     with leap seconds to stay within 0.9 s of UT1. 一般的な入口 scale。"
);
define_scale!(
    Tai,
    "TAI",
    TimeScaleKind::Atomic,
    "International Atomic Time. Proper-time-like scale realized by a global \
     ensemble of atomic clocks. `TT = TAI + 32.184 s`."
);
define_scale!(
    Tt,
    "TT",
    TimeScaleKind::CoordinateDerived,
    "Terrestrial Time. Coordinate-derived time (linear scale of TCG, \
     `dTT/dTCG = 1 - L_G`, `L_G = 6.969290134e-10`; IAU 2000 B1.9). \
     IAU 2006 precession と IAU 2000A/B nutation の独立変数。"
);
define_scale!(
    Ut1,
    "UT1",
    TimeScaleKind::EarthRotation,
    "Universal Time (UT1). Earth rotation angle time scale — defining \
     observable は ERA (IAU 2000 B1.8 / SOFA iauEra00)。atomic clock が \
     刻む時刻ではなく Earth の瞬間的な向きを時間単位で表現したもの。"
);
define_scale!(
    Tdb,
    "TDB",
    TimeScaleKind::CoordinateDerived,
    "Barycentric Dynamical Time. Coordinate-derived time (linear scale of TCB; \
     IAU 2006 Resolution B3)。Meeus / JPL DE (Teph ≈ TDB) ephemeris と \
     IAU 2009 body rotation の formally な独立変数。"
);
define_scale!(
    Gps,
    "GPS",
    TimeScaleKind::Atomic,
    "GPS Time. Atomic scale realized by the GPS control segment; a fixed \
     `TAI − 19 s` with NO leap seconds. Continuous since the GPS epoch \
     1980-01-06 00:00:00 UTC. GNSS 受信機・機上時刻の入口。`GPS − UTC` は \
     leap second の挿入ごとに 1 s ずつ増える (2017-01-01 以降は 18 s)。"
);
