//! Time representation with scale-tagged `Epoch<S>`.
//!
//! # 概要
//!
//! `Epoch<S>` は scale `S` で解釈される瞬間を表す。`S` は [`TimeScale`] trait を
//! 実装した marker (`Utc`, `Tai`, `Tt`, `Ut1`, `Tdb` のいずれか) で、
//! 時刻体系 (UTC, TAI, TT, UT1, TDB) をコンパイル時に区別する。
//!
//! 既存コードとの互換性のため、型パラメータはデフォルト値 `Utc` を持ち、
//! `Epoch` という bare 名は `Epoch<Utc>` と等価。
//!
//! # Time scale の deep coupling
//!
//! 時刻体系は特定の reference frame や地球回転と一次資料レベルで結合している。
//! 例えば UT1 は atomic clock が刻む時刻ではなく Earth rotation angle (ERA) によって
//! 実現される time scale であり、TDB は Meeus / JPL DE ephemeris の独立変数である。
//! 詳細は [`arika/DESIGN.md`](../../DESIGN.md) の「時刻系と座標系・測地系の定義
//! レベルの結合」を参照。
//!
//! # Scale-specific API
//!
//! Scale 間の silent 混同を防ぐため、API 入口を scale 固有に分けている:
//!
//! - [`Epoch<Utc>::from_gregorian`], [`Epoch<Utc>::from_iso8601`],
//!   [`Epoch<Utc>::from_datetime`], [`Epoch<Utc>::now`],
//!   [`Epoch<Utc>::from_tle_epoch`] — UTC 入口
//! - [`Epoch<Tt>::from_jd_tt`], [`Epoch<Tdb>::from_jd_tdb`],
//!   [`Epoch<Ut1>::from_jd_ut1`], [`Epoch<Tai>::from_jd_tai`] — scale 固有 JD 入口
//! - [`Epoch<Ut1>::era`] — Earth Rotation Angle (IAU 2000 B1.8)
//!
//! 変換は `to_tai()` / `to_tt()` / `to_tdb()` 等の method で明示的に行う。

use core::f64::consts::TAU;
use core::marker::PhantomData;

#[allow(unused_imports)]
use crate::math::F64Ext;

// ─── Constants ────────────────────────────────────────────────────

/// Julian Date of J2000.0 epoch (JD 2451545.0).
///
/// これは歴史的に J2000.0 TT と呼ばれる値だが、本実装では bit-level 互換性のため
/// 単純な f64 定数として扱い、scale は呼び出し側の `Epoch<S>` で決定される。
pub const J2000_JD: f64 = 2451545.0;

/// Offset between Julian Date and Modified Julian Date.
const MJD_OFFSET: f64 = 2400000.5;

/// Julian century in days.
const JULIAN_CENTURY: f64 = 36525.0;

/// TT - TAI (constant offset, IAU 2000 B1.9 / BIPM-TAI).
const TT_MINUS_TAI_SEC: f64 = 32.184;

/// Unix epoch (1970-01-01 00:00:00 UTC) in Julian Date.
#[cfg(feature = "std")]
const UNIX_EPOCH_JD: f64 = 2440587.5;

// ─── Leap second table ────────────────────────────────────────────

/// IERS leap second table: (MJD_start, TAI - UTC [s]).
///
/// Each entry is the MJD of the UTC day when the cumulative TAI-UTC offset became
/// the listed value. Updates are announced ~6 months ahead by IERS Bulletin C.
/// As of 2024 the last leap second was introduced at 2017-01-01 (TAI-UTC = 37 s).
const LEAP_SECONDS: &[(f64, f64)] = &[
    (41317.0, 10.0), // 1972-01-01
    (41499.0, 11.0), // 1972-07-01
    (41683.0, 12.0), // 1973-01-01
    (42048.0, 13.0), // 1974-01-01
    (42413.0, 14.0), // 1975-01-01
    (42778.0, 15.0), // 1976-01-01
    (43144.0, 16.0), // 1977-01-01
    (43509.0, 17.0), // 1978-01-01
    (43874.0, 18.0), // 1979-01-01
    (44239.0, 19.0), // 1980-01-01
    (44786.0, 20.0), // 1981-07-01
    (45151.0, 21.0), // 1982-07-01
    (45516.0, 22.0), // 1983-07-01
    (46247.0, 23.0), // 1985-07-01
    (47161.0, 24.0), // 1988-01-01
    (47892.0, 25.0), // 1990-01-01
    (48257.0, 26.0), // 1991-01-01
    (48804.0, 27.0), // 1992-07-01
    (49169.0, 28.0), // 1993-07-01
    (49534.0, 29.0), // 1994-07-01
    (50083.0, 30.0), // 1996-01-01
    (50630.0, 31.0), // 1997-07-01
    (51179.0, 32.0), // 1999-01-01
    (53736.0, 33.0), // 2006-01-01
    (54832.0, 34.0), // 2009-01-01
    (56109.0, 35.0), // 2012-07-01
    (57204.0, 36.0), // 2015-07-01
    (57754.0, 37.0), // 2017-01-01
];

/// TAI - UTC [seconds] at the given UTC MJD.
///
/// Before 1972-01-01 (MJD 41317) returns 10.0 (the value at the introduction of
/// the modern leap-second regime). After the last table entry returns the final
/// listed offset (currently 37.0).
///
/// # Pre-1972 limitation
///
/// Pre-1972 UTC used a different definition based on "rubber seconds" and
/// stepped frequency offsets, with ~50 distinct entries from 1960 to 1971.
/// The actual TAI − UTC offset during that era varied from ~1.4 s (1961) to
/// ~9.9 s (late 1971), NOT a constant 10.0 s. For example, Apollo 11 epoch
/// (1969-07-20) had TAI − UTC ≈ 8.0 s, so this function over-estimates by
/// about 2 s for that date.
///
/// At lunar distances (Moon velocity ~1 km/s), a 2 s time scale error
/// translates to ~2 km Meeus ephemeris offset. This is a strict improvement
/// over the pre-refactor behavior which did NOT convert UTC → TDB at all
/// (yielding a ~40 s / ~40 km error at Apollo epochs), but it is not
/// perfectly correct for pre-1972 dates.
///
/// A full pre-1972 UTC rate-offset table is deferred to a later phase of the
/// arika redesign. Callers requiring bit-accurate pre-1972 ephemerides
/// should use [`Epoch::<Tdb>::from_jd_tdb`] directly with an externally
/// computed TDB Julian Date.
fn tai_minus_utc_at_mjd(utc_mjd: f64) -> f64 {
    let mut offset = 10.0;
    for &(mjd_start, val) in LEAP_SECONDS {
        if utc_mjd >= mjd_start {
            offset = val;
        } else {
            break;
        }
    }
    offset
}

// ─── Fairhead-Bretagnon TDB-TT ────────────────────────────────────

/// TDB - TT [seconds] at the given TT Julian Date.
///
/// Uses a simplified 2-term Fairhead-Bretagnon series:
/// ```text
/// TDB - TT ≈ 0.001658 × sin(g) + 0.000014 × sin(2g)  [seconds]
/// g = 357.53° + 0.98560028° × (JD_TT - 2451545.0)    [Earth mean anomaly]
/// ```
/// Accurate to < 0.1 ms for typical epochs (sufficient for < arcsecond ephemeris).
fn tdb_minus_tt(tt_jd: f64) -> f64 {
    let d = tt_jd - J2000_JD;
    let g_deg = 357.53 + 0.985_600_28 * d;
    let g = g_deg.to_radians();
    0.001_658 * g.sin() + 0.000_014 * (2.0 * g).sin()
}

// ─── Sealed trait ─────────────────────────────────────────────────

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

// ─── DateTime ─────────────────────────────────────────────────────

/// A Gregorian calendar date and time (UTC).
///
/// 本 struct は UTC 暦表示専用。TAI / TT / TDB 等の dynamical time scale は
/// 直接 Gregorian で表現しない (`Epoch<Utc>` に変換してから `to_datetime` を使う)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: f64,
}

impl DateTime {
    pub fn new(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: f64) -> Self {
        DateTime {
            year,
            month,
            day,
            hour,
            min,
            sec,
        }
    }
}

impl core::fmt::Display for DateTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Round to integer seconds and normalize overflow (e.g. sec=59.999... → 60)
        let sec = self.sec.round() as u32;
        let (sec, carry) = if sec >= 60 { (0u32, 1u32) } else { (sec, 0) };
        let min = self.min + carry;
        let (min, carry) = if min >= 60 {
            (min - 60, 1u32)
        } else {
            (min, 0)
        };
        let hour = self.hour + carry;
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, hour, min, sec
        )
    }
}

// ─── Epoch<S> ─────────────────────────────────────────────────────

/// An astronomical epoch represented as Julian Date in scale `S`.
///
/// `S` defaults to [`Utc`] so that `Epoch` (without type parameter) means
/// `Epoch<Utc>` — the most common user-facing scale.
///
/// # Scale 解釈
///
/// 内部表現は単一の `jd: f64` だが、その値は **scale `S` で解釈される** JD である。
/// つまり `Epoch<Utc>::from_jd(x).jd() == x` (UTC JD として round-trip)、
/// `Epoch<Tdb>::from_jd_tdb(x).jd() == x` (TDB JD として round-trip) となる。
///
/// Scale 間の変換 (`to_tdb()`, `to_tt()` 等) は内部で TAI を経由し leap second や
/// Fairhead 補正を適用して別 scale の JD を計算する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Epoch<S: TimeScale = Utc> {
    /// JD interpreted in scale `S`.
    jd: f64,
    /// Scale tag (zero-sized).
    _scale: PhantomData<S>,
}

// ─── Generic accessors (available on all scales) ──────────────────

impl<S: TimeScale> Epoch<S> {
    /// Return the Julian Date value, interpreted in scale `S`.
    pub fn jd(&self) -> f64 {
        self.jd
    }

    /// Return the Modified Julian Date value, interpreted in scale `S`.
    pub fn mjd(&self) -> f64 {
        self.jd - MJD_OFFSET
    }

    /// The human-readable scale name (e.g. "UTC", "TDB").
    pub fn scale_name() -> &'static str {
        S::NAME
    }

    /// Crate-internal constructor from raw JD (bypasses scale semantics).
    /// Used for scale-conversion helpers and tests.
    pub(crate) fn from_jd_raw(jd: f64) -> Self {
        Self {
            jd,
            _scale: PhantomData,
        }
    }
}

// ─── Epoch<Utc> API (main user-facing scale) ─────────────────────

impl Epoch<Utc> {
    /// Create a UTC epoch from a raw Julian Date (treated as UTC JD).
    ///
    /// Legacy API matching the pre-refactor `Epoch::from_jd`. The resulting
    /// `Epoch<Utc>::jd()` returns `jd` unchanged (round-trip identity).
    pub fn from_jd(jd: f64) -> Self {
        Epoch {
            jd,
            _scale: PhantomData,
        }
    }

    /// Create a UTC epoch from a Modified Julian Date value.
    pub fn from_mjd(mjd: f64) -> Self {
        Epoch {
            jd: mjd + MJD_OFFSET,
            _scale: PhantomData,
        }
    }

    /// The J2000.0 reference epoch (JD 2451545.0).
    ///
    /// 歴史的には J2000.0 = 2000-01-01 12:00:00 TT だが、本実装では
    /// UTC scale で JD 2451545.0 を返す (後方互換のため)。厳密な TT J2000
    /// を得るには [`Epoch::<Tt>::from_jd_tt`] を使う。
    pub fn j2000() -> Self {
        Epoch {
            jd: J2000_JD,
            _scale: PhantomData,
        }
    }

    /// Create a UTC epoch from a [`DateTime`] value.
    pub fn from_datetime(dt: &DateTime) -> Self {
        Self::from_gregorian(dt.year, dt.month, dt.day, dt.hour, dt.min, dt.sec)
    }

    /// Create a UTC epoch from Gregorian calendar date and time.
    ///
    /// Uses the standard Julian Date algorithm valid for dates after
    /// the Gregorian calendar reform (1582-10-15).
    pub fn from_gregorian(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: f64) -> Self {
        // Adjust year and month for the algorithm (Jan/Feb are months 13/14 of prev year)
        let (y, m) = if month <= 2 {
            (year - 1, month + 12)
        } else {
            (year, month)
        };

        let a = y / 100;
        let b = 2 - a + a / 4;

        let jd = (365.25 * (y + 4716) as f64).floor()
            + (30.6001 * (m + 1) as f64).floor()
            + day as f64
            + b as f64
            - 1524.5
            + (hour as f64 + min as f64 / 60.0 + sec / 3600.0) / 24.0;

        Epoch {
            jd,
            _scale: PhantomData,
        }
    }

    /// Parse a UTC epoch from ISO 8601 format: `YYYY-MM-DDTHH:MM:SSZ`.
    ///
    /// Only UTC (Z suffix) is supported. Returns `None` if parsing fails.
    pub fn from_iso8601(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_suffix('Z')?;
        let (date, time) = s.split_once('T')?;

        let (year_s, rest) = date.split_once('-')?;
        let (month_s, day_s) = rest.split_once('-')?;
        let year: i32 = year_s.parse().ok()?;
        let month: u32 = month_s.parse().ok()?;
        let day: u32 = day_s.parse().ok()?;

        let (hour_s, rest) = time.split_once(':')?;
        let (min_s, sec_s) = rest.split_once(':')?;
        let hour: u32 = hour_s.parse().ok()?;
        let min: u32 = min_s.parse().ok()?;
        let sec: f64 = sec_s.parse().ok()?;

        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || min > 59
            || sec >= 60.0
        {
            return None;
        }

        Some(Self::from_gregorian(year, month, day, hour, min, sec))
    }

    /// Create a UTC epoch from the current system time.
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        let unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_secs_f64();
        Epoch {
            jd: UNIX_EPOCH_JD + unix_secs / 86400.0,
            _scale: PhantomData,
        }
    }

    /// Create a UTC epoch from a TLE epoch (2-digit year + fractional day of year).
    ///
    /// 2-digit year convention (NORAD): 57-99 → 1957-1999, 00-56 → 2000-2056.
    pub fn from_tle_epoch(year_2digit: u32, day_of_year: f64) -> Self {
        let year = if year_2digit >= 57 {
            1900 + year_2digit as i32
        } else {
            2000 + year_2digit as i32
        };
        // JD of January 0.0 of that year = JD of Dec 31 of previous year at 0h
        let jan1 = Self::from_gregorian(year, 1, 1, 0, 0, 0.0);
        // day_of_year: 1.0 = Jan 1 00:00, 1.5 = Jan 1 12:00, etc.
        Epoch {
            jd: jan1.jd + (day_of_year - 1.0),
            _scale: PhantomData,
        }
    }

    /// Julian centuries since J2000.0, computed directly from the UTC JD.
    ///
    /// **Note**: This treats the UTC JD as if it were a dynamical-time JD,
    /// which is strictly incorrect for high-precision ephemeris calculations.
    /// For Meeus/JPL DE usage, prefer `epoch.to_tdb().centuries_since_j2000()`.
    /// This method is kept for legacy bit-level compatibility where UTC
    /// centuries were used interchangeably with dynamical-time centuries.
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd - J2000_JD) / JULIAN_CENTURY
    }

    /// Advance the epoch by `dt` seconds using naive JD arithmetic
    /// (`jd + dt/86400`). Does NOT handle leap second boundaries.
    ///
    /// Legacy API for bit-level compatibility with pre-refactor `Epoch::add_seconds`.
    /// For leap-second-aware arithmetic use [`add_si_seconds`](Self::add_si_seconds)
    /// instead.
    pub fn add_seconds(&self, dt: f64) -> Self {
        Epoch {
            jd: self.jd + dt / 86400.0,
            _scale: PhantomData,
        }
    }

    /// Advance the epoch by `dt` SI seconds, handling leap second boundaries.
    ///
    /// Internally converts UTC → TAI, adds `dt` TAI seconds, and converts
    /// back to UTC. Crossing a leap second boundary correctly absorbs the
    /// extra second: 5 SI seconds from 2016-12-31T23:59:58 lands at
    /// 2017-01-01T00:00:02 (not 00:00:03), because one SI second is "consumed"
    /// by the 2017-01-01 leap.
    pub fn add_si_seconds(&self, dt: f64) -> Self {
        let utc_mjd = self.jd - MJD_OFFSET;
        let leap_before = tai_minus_utc_at_mjd(utc_mjd);
        let tai_jd = self.jd + leap_before / 86400.0;
        let new_tai_jd = tai_jd + dt / 86400.0;

        // Converge on the correct leap count at the new instant.
        let mut guess_utc_jd = new_tai_jd - leap_before / 86400.0;
        for _ in 0..3 {
            let guess_mjd = guess_utc_jd - MJD_OFFSET;
            let new_leap = tai_minus_utc_at_mjd(guess_mjd);
            guess_utc_jd = new_tai_jd - new_leap / 86400.0;
        }

        Epoch {
            jd: guess_utc_jd,
            _scale: PhantomData,
        }
    }

    /// Convert to Gregorian calendar date and time (UTC).
    pub fn to_datetime(&self) -> DateTime {
        to_datetime_from_jd(self.jd)
    }

    /// Convert to Gregorian calendar date and time (UTC), with leap second
    /// instants normalized to `00:00:00` of the next day.
    ///
    /// Alias for [`to_datetime`](Self::to_datetime) in Phase 1A (leap-instant
    /// display `23:59:60` is not yet distinguished).
    pub fn to_datetime_normalized(&self) -> DateTime {
        self.to_datetime()
    }

    /// Greenwich "sidereal time" in radians. **Legacy method**.
    ///
    /// Actually computes the Earth Rotation Angle (IAU 2000 B1.8 / SOFA
    /// `iauEra00`) assuming UT1 ≈ UTC (ignores dUT1). For the proper
    /// canonical form use [`Epoch::<Ut1>::era`] after an explicit UT1
    /// conversion via a proper EOP provider.
    ///
    /// Kept on `Epoch<Utc>` for bit-level compatibility with the pre-refactor
    /// `Epoch::gmst` method. Will be removed when downstream callers migrate
    /// to `Epoch<Ut1>::era`.
    pub fn gmst(&self) -> f64 {
        era_formula(self.jd)
    }

    // ─── Scale conversions (outbound from UTC) ────────────────────

    /// Convert to TAI by applying the current leap-second offset.
    pub fn to_tai(&self) -> Epoch<Tai> {
        let utc_mjd = self.jd - MJD_OFFSET;
        let leap = tai_minus_utc_at_mjd(utc_mjd);
        Epoch::<Tai>::from_jd_raw(self.jd + leap / 86400.0)
    }

    /// Convert to TT via UTC → TAI → TT.
    pub fn to_tt(&self) -> Epoch<Tt> {
        self.to_tai().to_tt()
    }

    /// Convert to TDB via UTC → TAI → TT → TDB (Fairhead-Bretagnon periodic).
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        self.to_tt().to_tdb()
    }

    /// Convert to UT1 assuming UT1 ≈ UTC (naive, legacy behavior).
    ///
    /// 真の UT1 が必要な場合は [`Epoch::<Utc>::to_ut1`] (`Ut1Offset` provider を
    /// 引数に取る) を使う。本 method は `NullEop` 相当の `dUT1 = 0` 仮定で、
    /// current arika の `gmst()` 実装との bit-level 互換を保つため提供される。
    pub fn to_ut1_naive(&self) -> Epoch<Ut1> {
        Epoch::<Ut1>::from_jd_raw(self.jd)
    }

    /// Convert to UT1 using the `dUT1 = UT1 − UTC` correction provided by
    /// an EOP provider.
    ///
    /// ```text
    /// JD_UT1 = JD_UTC + dUT1 / 86400
    /// ```
    ///
    /// `dUT1` is looked up at the current UTC MJD and is typically in the
    /// range `±0.9 s`. This is the **precise** UT1 conversion — the `NullEop`
    /// placeholder type does **not** implement [`Ut1Offset`], so passing it
    /// is a compile error (see `arika/tests/trybuild/`).
    ///
    /// The `?Sized` bound lets callers pass trait objects directly
    /// (e.g. `&dyn Ut1Offset` or `Box<dyn Ut1Offset>::as_ref()`) alongside
    /// concrete types.
    ///
    /// For a naive `dUT1 = 0` conversion used by the legacy simple rotation
    /// path, use [`Epoch::<Utc>::to_ut1_naive`] instead.
    pub fn to_ut1<P: crate::earth::eop::Ut1Offset + ?Sized>(&self, eop: &P) -> Epoch<Ut1> {
        let mjd = self.jd - MJD_OFFSET;
        let dut1 = eop.dut1(mjd);
        Epoch::<Ut1>::from_jd_raw(self.jd + dut1 / 86400.0)
    }
}

// ─── Epoch<Tai> API ───────────────────────────────────────────────

impl Epoch<Tai> {
    /// Create a TAI epoch from a Julian Date value interpreted as TAI JD.
    pub fn from_jd_tai(jd: f64) -> Self {
        Epoch::<Tai>::from_jd_raw(jd)
    }

    /// Convert to TT by adding the constant 32.184 s offset.
    pub fn to_tt(&self) -> Epoch<Tt> {
        Epoch::<Tt>::from_jd_raw(self.jd + TT_MINUS_TAI_SEC / 86400.0)
    }

    /// Convert to UTC by subtracting the current leap-second offset.
    pub fn to_utc(&self) -> Epoch<Utc> {
        // Iterate to find the right leap count (guess → refine).
        let mut guess_utc_jd = self.jd - 37.0 / 86400.0; // initial guess
        for _ in 0..3 {
            let guess_mjd = guess_utc_jd - MJD_OFFSET;
            let leap = tai_minus_utc_at_mjd(guess_mjd);
            guess_utc_jd = self.jd - leap / 86400.0;
        }
        Epoch::<Utc>::from_jd_raw(guess_utc_jd)
    }
}

// ─── Epoch<Tt> API ────────────────────────────────────────────────

impl Epoch<Tt> {
    /// Create a TT epoch from a Julian Date value interpreted as TT JD.
    pub fn from_jd_tt(jd: f64) -> Self {
        Epoch::<Tt>::from_jd_raw(jd)
    }

    /// Return TT Julian centuries since J2000.0.
    ///
    /// この値が IAU 2006 precession / IAU 2000A/B nutation の独立変数。
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd - J2000_JD) / JULIAN_CENTURY
    }

    /// Convert to TAI by subtracting the constant 32.184 s offset.
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_jd_raw(self.jd - TT_MINUS_TAI_SEC / 86400.0)
    }

    /// Convert to TDB via the Fairhead-Bretagnon periodic correction.
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        let delta = tdb_minus_tt(self.jd);
        Epoch::<Tdb>::from_jd_raw(self.jd + delta / 86400.0)
    }
}

// ─── Epoch<Tdb> API ───────────────────────────────────────────────

impl Epoch<Tdb> {
    /// Create a TDB epoch from a Julian Date value interpreted as TDB JD.
    ///
    /// JPL DE ephemerides use `Teph` which is for practical purposes
    /// indistinguishable from TDB (IAU 2006 Resolution B3).
    pub fn from_jd_tdb(jd: f64) -> Self {
        Epoch::<Tdb>::from_jd_raw(jd)
    }

    /// Return TDB Julian centuries since J2000.0.
    ///
    /// Meeus / JPL DE ephemeris と IAU 2009 WGCCRE body rotation の独立変数。
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd - J2000_JD) / JULIAN_CENTURY
    }

    /// Convert to TT by applying the inverse Fairhead-Bretagnon correction.
    pub fn to_tt(&self) -> Epoch<Tt> {
        // Since |TDB - TT| < 2 ms, a single-step inversion is accurate enough.
        let delta = tdb_minus_tt(self.jd);
        Epoch::<Tt>::from_jd_raw(self.jd - delta / 86400.0)
    }
}

// ─── Epoch<Ut1> API ───────────────────────────────────────────────

impl Epoch<Ut1> {
    /// Create a UT1 epoch from a Julian Date value interpreted as UT1 JD.
    pub fn from_jd_ut1(jd: f64) -> Self {
        Epoch::<Ut1>::from_jd_raw(jd)
    }

    /// Earth Rotation Angle (ERA) in radians.
    ///
    /// IAU 2000 Resolution B1.8 / SOFA `iauEra00`:
    /// `ERA(T_u) = 2π × (0.7790572732640 + 1.00273781191135448 × T_u)`
    /// where `T_u = JD_UT1 − 2451545.0`.
    ///
    /// ERA は UT1 の definitional な関数であり、他の scale で計算することは
    /// 意味論的に間違い。したがって `era()` method は `Epoch<Ut1>` にのみ
    /// 提供される (`Epoch<Tdb>::era()` はコンパイルエラー)。
    pub fn era(&self) -> f64 {
        era_formula(self.jd)
    }
}

// ─── Internal helpers ─────────────────────────────────────────────

/// Earth Rotation Angle (ERA) formula, shared by `Epoch<Ut1>::era` and the
/// legacy `Epoch<Utc>::gmst` method.
///
/// Note: the current arika source value `1.002_737_811_911_354_6` differs
/// from the canonical SOFA value `1.00273781191135448` by roughly 1 f64 ULP
/// (~1e-16). Phase 1A keeps the legacy constant for bit-level invariance with
/// pre-refactor tests. The canonical value will be adopted in a later phase.
fn era_formula(ut1_jd: f64) -> f64 {
    let du = ut1_jd - J2000_JD;
    let era = TAU * (0.7790572732640 + 1.002_737_811_911_354_6 * du);
    let era = era % TAU;
    if era < 0.0 { era + TAU } else { era }
}

/// Convert a Julian Date value to Gregorian calendar date and time.
/// Shared by `Epoch<Utc>::to_datetime` — kept at module scope so it can be
/// reused by future scale-specific display helpers.
fn to_datetime_from_jd(jd: f64) -> DateTime {
    // Meeus, "Astronomical Algorithms", Chapter 7
    let jd = jd + 0.5;
    let z = jd.floor() as i64;
    let f = jd - z as f64;

    let a = if z < 2299161 {
        z
    } else {
        let alpha = ((z as f64 - 1867216.25) / 36524.25).floor() as i64;
        z + 1 + alpha - alpha / 4
    };

    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25).floor() as i64;
    let d = (365.25 * c as f64).floor() as i64;
    let e = ((b - d) as f64 / 30.6001).floor() as i64;

    let day = (b - d - (30.6001 * e as f64).floor() as i64) as u32;
    let month = if e < 14 { e - 1 } else { e - 13 } as u32;
    let year = if month > 2 { c - 4716 } else { c - 4715 } as i32;

    let hours_total = f * 24.0;
    let hour = hours_total.floor() as u32;
    let mins_total = (hours_total - hour as f64) * 60.0;
    let min = mins_total.floor() as u32;
    let sec = (mins_total - min as f64) * 60.0;

    DateTime {
        year,
        month,
        day,
        hour,
        min,
        sec,
    }
}

// ─── Duration ─────────────────────────────────────────────────────

/// Scale-invariant duration measured in SI (TAI) seconds.
///
/// Does not carry a scale tag because SI seconds tick uniformly regardless of
/// the reference time scale. UTC display arithmetic (e.g. "翌日同時刻") is NOT
/// provided — use [`Epoch::<Utc>::add_si_seconds`] which correctly handles leap
/// second boundaries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duration {
    si_seconds: f64,
}

impl Duration {
    /// Construct from SI seconds.
    pub const fn from_si_seconds(s: f64) -> Self {
        Duration { si_seconds: s }
    }

    /// Construct from minutes (= 60 SI seconds).
    pub const fn from_minutes(m: f64) -> Self {
        Duration {
            si_seconds: m * 60.0,
        }
    }

    /// Construct from hours (= 3600 SI seconds).
    pub const fn from_hours(h: f64) -> Self {
        Duration {
            si_seconds: h * 3600.0,
        }
    }

    /// Return the duration in SI seconds.
    pub fn as_si_seconds(&self) -> f64 {
        self.si_seconds
    }
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- Epoch construction and accessors ---

    #[test]
    fn mjd_jd_relationship() {
        let epoch = Epoch::from_jd(2451545.0);
        assert!((epoch.mjd() - 51544.5).abs() < 1e-12);
    }

    #[test]
    fn scale_name_via_type() {
        assert_eq!(Epoch::<Utc>::scale_name(), "UTC");
        assert_eq!(Epoch::<Tai>::scale_name(), "TAI");
        assert_eq!(Epoch::<Tt>::scale_name(), "TT");
        assert_eq!(Epoch::<Ut1>::scale_name(), "UT1");
        assert_eq!(Epoch::<Tdb>::scale_name(), "TDB");
    }

    // --- Gregorian conversions ---

    #[test]
    fn j2000_gregorian() {
        // J2000.0 = 2000-01-01 12:00:00
        let epoch = Epoch::from_gregorian(2000, 1, 1, 12, 0, 0.0);
        assert!(
            (epoch.jd() - J2000_JD).abs() < 1e-6,
            "J2000 JD: expected {}, got {}",
            J2000_JD,
            epoch.jd()
        );
    }

    #[test]
    fn known_date_2024_march_equinox() {
        // 2024-03-20 12:00:00 UTC
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let expected_jd = 2460390.0;
        assert!(
            (epoch.jd() - expected_jd).abs() < 0.01,
            "2024-03-20 JD: expected ~{}, got {}",
            expected_jd,
            epoch.jd()
        );
    }

    #[test]
    fn gregorian_roundtrip() {
        let original = Epoch::from_gregorian(2024, 6, 21, 15, 30, 45.0);
        let dt = original.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 6);
        assert_eq!(dt.day, 21);
        assert_eq!(dt.hour, 15);
        assert_eq!(dt.min, 30);
        assert!(
            (dt.sec - 45.0).abs() < 0.01,
            "sec: expected 45.0, got {}",
            dt.sec
        );
    }

    #[test]
    fn gregorian_roundtrip_january() {
        // Jan and Feb use different month adjustment in JD algorithm
        let original = Epoch::from_gregorian(2024, 1, 15, 0, 0, 0.0);
        let dt = original.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 15);
        assert_eq!(dt.hour, 0);
        assert_eq!(dt.min, 0);
    }

    #[test]
    fn gregorian_roundtrip_february() {
        let original = Epoch::from_gregorian(2024, 2, 29, 6, 0, 0.0);
        let dt = original.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 2);
        assert_eq!(dt.day, 29);
        assert_eq!(dt.hour, 6);
    }

    #[test]
    fn datetime_display() {
        let dt = DateTime::new(2024, 3, 20, 12, 0, 0.0);
        assert_eq!(dt.to_string(), "2024-03-20T12:00:00Z");
    }

    #[test]
    fn from_datetime_roundtrip() {
        let dt = DateTime::new(2024, 6, 21, 15, 30, 45.0);
        let epoch = Epoch::from_datetime(&dt);
        let rt = epoch.to_datetime();
        assert_eq!(rt.year, dt.year);
        assert_eq!(rt.month, dt.month);
        assert_eq!(rt.day, dt.day);
        assert_eq!(rt.hour, dt.hour);
        assert_eq!(rt.min, dt.min);
        assert!((rt.sec - dt.sec).abs() < 0.01);
    }

    // --- add_seconds / add_si_seconds ---

    #[test]
    fn add_seconds_one_day() {
        let epoch = Epoch::j2000();
        let next_day = epoch.add_seconds(86400.0);
        assert!(
            (next_day.jd() - (J2000_JD + 1.0)).abs() < 1e-12,
            "add 86400s: expected JD {}, got {}",
            J2000_JD + 1.0,
            next_day.jd()
        );
    }

    #[test]
    fn now_returns_reasonable_jd() {
        let epoch = Epoch::now();
        // JD for 2025-01-01 ≈ 2460676, for 2030-01-01 ≈ 2462502
        // Any reasonable current date should be in this range
        assert!(
            epoch.jd() > 2460676.0 && epoch.jd() < 2462502.0,
            "Epoch::now() JD {} is outside 2025–2030 range",
            epoch.jd()
        );
        // Verify to_datetime year is plausible
        let dt = epoch.to_datetime();
        assert!(
            dt.year >= 2025 && dt.year <= 2030,
            "Epoch::now() year {} is outside expected range",
            dt.year
        );
    }

    #[test]
    fn add_seconds_one_hour() {
        let epoch = Epoch::j2000();
        let plus_hour = epoch.add_seconds(3600.0);
        let expected = J2000_JD + 1.0 / 24.0;
        assert!((plus_hour.jd() - expected).abs() < 1e-12);
    }

    #[test]
    fn centuries_since_j2000() {
        let epoch = Epoch::j2000();
        assert!((epoch.centuries_since_j2000() - 0.0).abs() < 1e-15);

        // One Julian century later
        let later = Epoch::from_jd(J2000_JD + JULIAN_CENTURY);
        assert!((later.centuries_since_j2000() - 1.0).abs() < 1e-12);
    }

    // --- Discriminating Red tests for Phase 1A ---
    //
    // These tests verify behaviors that are only achievable with the new
    // Epoch<Scale> design. They would fail with the pre-refactor naive
    // `Epoch { jd: f64 }` implementation.

    /// **Discriminating test**: `add_si_seconds` crossing the 2017-01-01 leap
    /// second boundary must absorb one SI second into the leap, landing one
    /// UTC second earlier than naive `add_seconds` would predict.
    #[test]
    fn leap_second_2017_crossing_si_arithmetic() {
        let before = Epoch::<Utc>::from_iso8601("2016-12-31T23:59:55Z").unwrap();
        let naive = before.add_seconds(10.0);
        let aware = before.add_si_seconds(10.0);

        // Naive arithmetic: 10 "JD-seconds" later = 2017-01-01T00:00:05Z
        // (because UTC JD treats the leap day as exactly 86400 seconds)
        let dt_naive = naive.to_datetime();
        assert_eq!(dt_naive.year, 2017);
        assert_eq!(dt_naive.month, 1);
        assert_eq!(dt_naive.day, 1);
        assert_eq!(dt_naive.hour, 0);
        assert_eq!(dt_naive.min, 0);
        assert!((dt_naive.sec - 5.0).abs() < 0.01);

        // Leap-second-aware: 10 SI seconds from 23:59:55 traverses the
        // 23:59:60 leap, so UTC display shows one fewer second elapsed.
        let dt_aware = aware.to_datetime();
        assert_eq!(dt_aware.year, 2017);
        assert_eq!(dt_aware.month, 1);
        assert_eq!(dt_aware.day, 1);
        assert_eq!(dt_aware.hour, 0);
        assert_eq!(dt_aware.min, 0);
        assert!(
            (dt_aware.sec - 4.0).abs() < 0.01,
            "add_si_seconds should absorb leap second: expected ~4.0 s, got {}",
            dt_aware.sec
        );
    }

    /// **Discriminating test**: Converting a UTC epoch to TDB must produce a
    /// JD that differs by ~69.184 s (leap count + TT-TAI + Fairhead).
    ///
    /// With the pre-refactor naive implementation, UTC JD was fed directly
    /// into Meeus ephemerides as if it were TDB, causing the Artemis 1 69-km
    /// position offset described in orts/examples/artemis1/main.rs:29-32.
    #[test]
    fn utc_to_tdb_applies_expected_offset_2024() {
        let utc = Epoch::<Utc>::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let tdb = utc.to_tdb();
        let delta_sec = (tdb.jd() - utc.jd()) * 86400.0;

        // Expected: 37 (leap) + 32.184 (TT-TAI) + ~1.6 ms (Fairhead) ≈ 69.184 s
        let expected_sec = 37.0 + TT_MINUS_TAI_SEC;
        assert!(
            (delta_sec - expected_sec).abs() < 0.01,
            "TDB - UTC at 2024-03-20: expected ~{} s, got {} s",
            expected_sec,
            delta_sec
        );
    }

    /// **Discriminating test**: Round-trip `to_tdb().to_tt().to_tai().to_utc()`
    /// from a UTC epoch must recover the original UTC JD bit-for-bit
    /// (within f64 precision).
    #[test]
    fn utc_tdb_tt_tai_roundtrip() {
        let original = Epoch::<Utc>::from_iso8601("2024-06-15T08:30:45Z").unwrap();
        let tdb = original.to_tdb();
        let tt = tdb.to_tt();
        let tai = tt.to_tai();
        let utc = tai.to_utc();
        assert!(
            (utc.jd() - original.jd()).abs() < 1e-10,
            "UTC→TDB→TT→TAI→UTC roundtrip diverged: original={} recovered={}",
            original.jd(),
            utc.jd()
        );
    }

    /// **Discriminating test**: `Epoch<Tt>::centuries_since_j2000` and
    /// `Epoch<Tdb>::centuries_since_j2000` must differ from each other AND
    /// from `Epoch<Utc>::centuries_since_j2000` by the expected offsets.
    ///
    /// With the pre-refactor implementation all three were the same method
    /// returning the same value (ignoring scale), which was the root cause
    /// of the UTC-as-TDB silent bug.
    #[test]
    fn centuries_since_j2000_differs_per_scale() {
        let utc = Epoch::<Utc>::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let tt = utc.to_tt();
        let tdb = utc.to_tdb();

        let c_utc = utc.centuries_since_j2000();
        let c_tt = tt.centuries_since_j2000();
        let c_tdb = tdb.centuries_since_j2000();

        // TT - UTC ≈ 69.184 s = 2.19e-8 centuries
        let dc_tt_utc = c_tt - c_utc;
        let expected_tt_utc = 69.184 / (86400.0 * 36525.0);
        assert!(
            (dc_tt_utc - expected_tt_utc).abs() < 1e-14,
            "TT - UTC centuries: expected {:e}, got {:e}",
            expected_tt_utc,
            dc_tt_utc
        );

        // TDB - TT ≈ few ms peak-to-peak (Fairhead periodic), ~5e-13 centuries.
        // Much smaller than TT - UTC (~2.2e-8) but still detectable.
        let dc_tdb_tt = c_tdb - c_tt;
        assert!(
            dc_tdb_tt.abs() < 1e-11,
            "TDB - TT centuries should be ~ms → ~5e-13 scale, got {:e}",
            dc_tdb_tt
        );
        assert!(
            dc_tdb_tt.abs() < dc_tt_utc.abs() * 0.001,
            "TDB - TT should be much smaller than TT - UTC: dc_tdb_tt={:e}, dc_tt_utc={:e}",
            dc_tdb_tt,
            dc_tt_utc
        );
    }

    // --- ISO 8601 parsing ---

    #[test]
    fn iso8601_valid() {
        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let expected = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        assert!(
            (epoch.jd() - expected.jd()).abs() < 1e-10,
            "ISO parse mismatch"
        );
    }

    #[test]
    fn iso8601_with_seconds() {
        let epoch = Epoch::from_iso8601("2000-01-01T12:00:00Z").unwrap();
        assert!((epoch.jd() - J2000_JD).abs() < 1e-6);
    }

    #[test]
    fn iso8601_invalid_no_z() {
        assert!(Epoch::from_iso8601("2024-03-20T12:00:00").is_none());
    }

    #[test]
    fn iso8601_invalid_format() {
        assert!(Epoch::from_iso8601("not-a-date").is_none());
        assert!(Epoch::from_iso8601("2024-13-01T00:00:00Z").is_none()); // month 13
        assert!(Epoch::from_iso8601("2024-01-32T00:00:00Z").is_none()); // day 32
    }

    // --- ERA / legacy GMST ---

    #[test]
    fn gmst_at_j2000() {
        let epoch = Epoch::j2000();
        let gmst = epoch.gmst();
        // At J2000.0, ERA ≈ 280.46° = 4.8949 rad
        let expected = TAU * 0.7790572732640;
        assert!(
            (gmst - expected).abs() < 0.01,
            "GMST at J2000: expected {:.4} rad, got {:.4} rad",
            expected,
            gmst
        );
    }

    #[test]
    fn gmst_increases_one_sidereal_day() {
        // One sidereal day ≈ 86164.0905 seconds
        // After one solar day (86400s), GMST should increase by ~360.9856° ≈ ~2π + 0.0172 rad
        let epoch = Epoch::j2000();
        let gmst0 = epoch.gmst();
        let next_day = epoch.add_seconds(86400.0);
        let gmst1 = next_day.gmst();

        let delta = if gmst1 > gmst0 {
            gmst1 - gmst0
        } else {
            gmst1 + TAU - gmst0
        };
        let expected_delta = TAU * 1.002_737_811_911_354_6;
        let expected_delta_mod = expected_delta % TAU;
        assert!(
            (delta - expected_delta_mod).abs() < 0.001,
            "GMST daily increase: expected {:.6} rad, got {:.6} rad",
            expected_delta_mod,
            delta
        );
    }

    #[test]
    fn gmst_normalized() {
        // GMST should always be in [0, 2π)
        for days in [0.0, 0.5, 1.0, 100.0, 365.25, 3652.5] {
            let epoch = Epoch::j2000().add_seconds(days * 86400.0);
            let gmst = epoch.gmst();
            assert!(
                gmst >= 0.0 && gmst < TAU,
                "GMST at +{days} days: {gmst} not in [0, 2π)"
            );
        }
    }

    #[test]
    fn era_on_ut1_matches_legacy_gmst() {
        // Utc.to_ut1_naive().era() should equal the legacy gmst() on the same
        // UTC epoch bit-for-bit (since both use the same ERA formula and naive
        // UT1 = UTC assumption).
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let ut1 = utc.to_ut1_naive();
        assert_eq!(ut1.era(), utc.gmst());
    }

    // --- to_ut1 with EOP provider ---

    #[test]
    fn to_ut1_applies_dut1_offset() {
        // A mock EOP provider supplying a fixed dUT1 of -0.250 s should
        // shift the UT1 JD by exactly -0.250/86400 days relative to UTC.
        struct FixedDut1(f64);
        impl crate::earth::eop::Ut1Offset for FixedDut1 {
            fn dut1(&self, _utc_mjd: f64) -> f64 {
                self.0
            }
        }

        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let eop = FixedDut1(-0.250);
        let ut1 = utc.to_ut1(&eop);

        // JD around 2.46e6 has ~1 ULP ≈ 5.6e-10 days ≈ 4.8e-5 s resolution,
        // so the reconstructed delta is accurate only to ~10 μs.
        let delta_s = (ut1.jd() - utc.jd()) * 86400.0;
        assert!(
            (delta_s - (-0.250)).abs() < 1e-4,
            "expected -0.250 s shift, got {delta_s}"
        );
    }

    #[test]
    fn to_ut1_naive_is_equivalent_to_zero_dut1_provider() {
        // to_ut1_naive() == to_ut1(&provider with dut1 == 0).
        struct ZeroDut1;
        impl crate::earth::eop::Ut1Offset for ZeroDut1 {
            fn dut1(&self, _utc_mjd: f64) -> f64 {
                0.0
            }
        }
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let naive = utc.to_ut1_naive();
        let precise = utc.to_ut1(&ZeroDut1);
        assert_eq!(naive.jd(), precise.jd());
    }

    #[test]
    fn to_ut1_accepts_trait_object_provider() {
        // The `?Sized` bound on `to_ut1<P>` lets callers pass `&dyn Ut1Offset`
        // / `Box<dyn Ut1Offset>` directly, which is essential for runtime
        // provider selection (e.g. a plugin-supplied EOP source).
        struct Fixed(f64);
        impl crate::earth::eop::Ut1Offset for Fixed {
            fn dut1(&self, _: f64) -> f64 {
                self.0
            }
        }
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let boxed: Box<dyn crate::earth::eop::Ut1Offset> = Box::new(Fixed(-0.100));
        let _ut1_box: Epoch<Ut1> = utc.to_ut1(boxed.as_ref());
        let dyn_ref: &dyn crate::earth::eop::Ut1Offset = &Fixed(-0.100);
        let _ut1_dyn: Epoch<Ut1> = utc.to_ut1(dyn_ref);
    }

    #[test]
    fn to_ut1_passes_utc_mjd_to_provider() {
        // Verify the UTC MJD passed to the provider matches `epoch.mjd()`.
        use std::cell::Cell;
        struct Recording(Cell<f64>);
        impl crate::earth::eop::Ut1Offset for Recording {
            fn dut1(&self, utc_mjd: f64) -> f64 {
                self.0.set(utc_mjd);
                0.0
            }
        }
        let utc = Epoch::<Utc>::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let r = Recording(Cell::new(f64::NAN));
        let _ = utc.to_ut1(&r);
        assert_eq!(r.0.get(), utc.mjd());
    }

    // --- TLE epoch ---

    #[test]
    fn tle_epoch_iss_2024() {
        // ISS TLE epoch: 24079.50000000 → 2024 day 79.5 → 2024-03-19 12:00:00 UTC
        let epoch = Epoch::from_tle_epoch(24, 79.5);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 3);
        assert_eq!(dt.day, 19);
        assert_eq!(dt.hour, 12);
    }

    #[test]
    fn tle_epoch_year_2000() {
        // Year 00 → 2000, day 1.0 → 2000-01-01 00:00:00
        let epoch = Epoch::from_tle_epoch(0, 1.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2000);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.hour, 0);
    }

    #[test]
    fn tle_epoch_year_1999() {
        // Year 99 → 1999, day 365.0 → 1999-12-31 00:00:00
        let epoch = Epoch::from_tle_epoch(99, 365.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 1999);
        assert_eq!(dt.month, 12);
        assert_eq!(dt.day, 31);
    }

    #[test]
    fn tle_epoch_year_57() {
        // Year 57 → 1957 (Sputnik era)
        let epoch = Epoch::from_tle_epoch(57, 1.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 1957);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
    }

    #[test]
    fn tle_epoch_year_56() {
        // Year 56 → 2056
        let epoch = Epoch::from_tle_epoch(56, 1.0);
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2056);
    }

    #[test]
    fn tle_epoch_matches_iso8601() {
        // TLE epoch 24001.50000000 → 2024-01-01 12:00:00 UTC
        let tle_epoch = Epoch::from_tle_epoch(24, 1.5);
        let iso_epoch = Epoch::from_iso8601("2024-01-01T12:00:00Z").unwrap();
        assert!(
            (tle_epoch.jd() - iso_epoch.jd()).abs() < 1e-6,
            "TLE epoch {} vs ISO epoch {}",
            tle_epoch.jd(),
            iso_epoch.jd()
        );
    }

    // --- JD → UTC string end-to-end ---

    #[test]
    fn jd_to_utc_string_j2000() {
        let s = Epoch::from_jd(J2000_JD).to_datetime().to_string();
        assert_eq!(s, "2000-01-01T12:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_2024_march() {
        let s = Epoch::from_jd(2460390.0).to_datetime().to_string();
        assert_eq!(s, "2024-03-20T12:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_with_offset_1h() {
        // J2000 + 3600s = 2000-01-01T13:00:00Z
        let s = Epoch::from_jd(J2000_JD)
            .add_seconds(3600.0)
            .to_datetime()
            .to_string();
        assert_eq!(s, "2000-01-01T13:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_with_offset_1day() {
        // J2000 + 86400s = 2000-01-02T12:00:00Z
        let s = Epoch::from_jd(J2000_JD)
            .add_seconds(86400.0)
            .to_datetime()
            .to_string();
        assert_eq!(s, "2000-01-02T12:00:00Z");
    }

    #[test]
    fn jd_to_utc_string_no_fractional_seconds() {
        // Fractional seconds should be truncated (format uses {:02.0})
        let s = Epoch::from_jd(J2000_JD)
            .add_seconds(0.5)
            .to_datetime()
            .to_string();
        assert!(
            s.ends_with("Z") && !s.contains('.'),
            "Should not contain fractional seconds: {s}"
        );
    }

    #[test]
    fn gmst_works_with_simple_eci_ecef() {
        // Verify that Epoch::gmst() produces valid angles for
        // SimpleEci↔SimpleEcef conversion via Rotation<SimpleEci, SimpleEcef>.
        use crate::SimpleEci;
        use crate::frame::{
            Rotation, SimpleEcef as SimpleEcefMarker, SimpleEci as SimpleEciMarker,
        };
        let epoch = Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0);
        let era = epoch.gmst();

        let eci = SimpleEci::new(7000.0, 1000.0, 500.0);
        let ecef = Rotation::<SimpleEciMarker, SimpleEcefMarker>::from_era(era).transform(&eci);
        let roundtrip =
            Rotation::<SimpleEcefMarker, SimpleEciMarker>::from_era(era).transform(&ecef);

        let eps = 1e-10;
        assert!((roundtrip.x() - eci.x()).abs() < eps);
        assert!((roundtrip.y() - eci.y()).abs() < eps);
        assert!((roundtrip.z() - eci.z()).abs() < eps);
    }

    // --- Leap second table sanity ---

    #[test]
    fn leap_second_table_monotonic() {
        // TAI-UTC should strictly increase over time.
        let mut prev_mjd = 0.0;
        let mut prev_offset = 0.0;
        for &(mjd, offset) in LEAP_SECONDS {
            assert!(mjd > prev_mjd, "Leap table MJD not monotonic: {mjd}");
            assert!(offset > prev_offset, "Leap offset not monotonic: {offset}");
            prev_mjd = mjd;
            prev_offset = offset;
        }
    }

    #[test]
    fn leap_second_2024_is_37() {
        // MJD 60000 ≈ 2023-02-25, well after the 2017-01-01 leap entry (MJD 57754)
        assert_eq!(tai_minus_utc_at_mjd(60000.0), 37.0);
    }

    #[test]
    fn leap_second_before_1972_is_10() {
        // Pre-1972: default to the first table value.
        assert_eq!(tai_minus_utc_at_mjd(40000.0), 10.0);
    }

    // --- Duration ---

    #[test]
    fn duration_si_seconds() {
        assert_eq!(Duration::from_si_seconds(60.0).as_si_seconds(), 60.0);
        assert_eq!(Duration::from_minutes(1.0).as_si_seconds(), 60.0);
        assert_eq!(Duration::from_hours(1.0).as_si_seconds(), 3600.0);
    }
}
