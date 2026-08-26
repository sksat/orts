//! Time representation with scale-tagged `Epoch<S>`.
//!
//! # 概要
//!
//! `Epoch<S>` は scale `S` で解釈される瞬間を表す。`S` は [`TimeScale`] trait を
//! 実装した marker (`Utc`, `Tai`, `Tt`, `Tdb`, `Gps` のいずれか) で、
//! 時刻体系をコンパイル時に区別する。
//!
//! UT1 は EOP (測定された dUT1) 依存で他の scale と data-free に換算できないため
//! `Epoch<S>` の仲間ではなく独立型 [`Ut1Epoch`] として扱う ([`Epoch<Utc>::to_ut1`]
//! 経由で生成)。
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
//!   [`Epoch<Tai>::from_jd_tai`], [`Ut1Epoch::from_jd_ut1`] — scale 固有 JD 入口
//! - [`Ut1Epoch::era`] — Earth Rotation Angle (IAU 2000 B1.8)
//!
//! 変換は `to_tai()` / `to_tt()` / `to_tdb()` 等の method で明示的に行う。

use core::marker::PhantomData;

#[allow(unused_imports)]
use crate::math::F64Ext;

mod convert;
mod datetime;
mod duration;
mod gps;
mod jd2;
mod leap;
mod precision;
mod scale;

pub use convert::{FixedOffsetFromTai, Ut1Epoch};
pub use datetime::DateTime;
pub use duration::Duration;
pub use gps::{GpsWeek, SecondsOfWeek};
pub use jd2::{JdRepr, TwoPartJd};
pub use precision::{Coarse, Precise, Precision};
pub use scale::{Gps, Tai, Tdb, TimeScale, Tt, Utc};

// Consumed by the EOP table's dUT1 interpolation (`alloc`-gated) and by
// `convert.rs` via the private `leap` module path.
#[cfg(feature = "alloc")]
pub(crate) use leap::tai_minus_utc_at_mjd;

use convert::TaiLens;
use convert::era_formula;
use datetime::to_datetime_from_jd;

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

/// GPS - TAI (constant offset). GPS Time runs `TAI − 19 s` with no leap
/// seconds, fixed at the GPS epoch (1980-01-06) and unchanged since.
const GPS_MINUS_TAI_SEC: f64 = -19.0;

/// Julian Date of the GPS epoch: 1980-01-06 00:00:00 UTC.
///
/// At that instant `TAI − UTC` was 19 s, so `GPS = UTC` there and the GPS-scale
/// JD of the epoch equals the UTC JD. GPS week / seconds-of-week are measured
/// from this instant.
pub const GPS_EPOCH_JD: f64 = 2444244.5;

/// Unix epoch (1970-01-01 00:00:00 UTC) in Julian Date.
#[cfg(feature = "std")]
const UNIX_EPOCH_JD: f64 = 2440587.5;

/// An astronomical epoch represented as Julian Date in scale `S`, stored at
/// precision tier `P`.
///
/// `S` defaults to [`Utc`] and `P` to [`Precise`], so that `Epoch` (without type
/// parameters) means `Epoch<Utc, Precise>` — the most common user-facing scale
/// at sub-nanosecond precision.
///
/// # 内部表現: canonical TAI
///
/// 内部は **canonical な TAI instant を精度 tier `P` の表現 ([`P::Repr`](Precision::Repr))
/// で保持**し、`S` は読み出しレンズである。`P = Precise` なら two-part [`TwoPartJd`]、
/// `P = Coarse` なら単一 [`f64`]。`jd()` は格納された TAI を scale `S` の JD に
/// 変換して返す: `Epoch<Utc>::from_jd(x).jd() == x` (UTC JD として round-trip)、
/// `Epoch<Tdb>::from_jd_tdb(x).jd() == x` (TDB JD として round-trip)。
///
/// scale 変換 (`to_tt()`, `to_tdb()` 等) は同じ TAI instant の **非破壊な再ラベル**で、
/// leap second / Fairhead 補正は構築時 (`from_*`) と読み出し時 (`jd()`) の lens
/// ([`TaiLens`]) でのみ適用される。同一物理 instant は同一内部値なので
/// `duration_since` は leap 跨ぎ・cross-scale でも厳密な SI 秒を返す。
///
/// # 精度 tier の選択
///
/// 豊富なコンストラクタ (`from_jd`, `from_gregorian`, …) は既定の [`Precise`] を
/// 作る。[`Coarse`] が欲しい場合は [`to_precision`](Self::to_precision) で
/// 変換する (例: `epoch.to_precision::<Coarse>()`)。tier は型の一部なので、
/// 別 tier の `Epoch` 同士の演算は明示的な変換を要する。
pub struct Epoch<S: TimeScale = Utc, P: Precision = Precise> {
    /// Canonical TAI instant, stored at precision tier `P`. Read out in scale
    /// `S` via the lens.
    tai: P::Repr,
    /// Scale tag (zero-sized).
    _scale: PhantomData<S>,
}

// Manual `Copy`/`Clone`/`PartialEq`/`Debug`: `#[derive]` would bound them on
// `P: Trait` rather than on the stored field `P::Repr`, which doesn't hold. The
// `P: Precision` bound gives `P::Repr: JdRepr`, hence `Copy + PartialEq + Debug`.

impl<S: TimeScale, P: Precision> Clone for Epoch<S, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: TimeScale, P: Precision> Copy for Epoch<S, P> {}

impl<S: TimeScale, P: Precision> PartialEq for Epoch<S, P> {
    fn eq(&self, other: &Self) -> bool {
        self.tai == other.tai
    }
}

impl<S: TimeScale, P: Precision> core::fmt::Debug for Epoch<S, P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Epoch")
            .field("scale", &S::NAME)
            .field("precision", &P::NAME)
            .field("tai", &self.tai)
            .finish()
    }
}

// Generic accessors (available on all scales and precision tiers)

impl<S: TimeScale, P: Precision> Epoch<S, P> {
    /// The human-readable scale name (e.g. "UTC", "TDB").
    pub fn scale_name() -> &'static str {
        S::NAME
    }

    /// The precision-tier name ("precise" / "coarse").
    pub fn precision_name() -> &'static str {
        P::NAME
    }

    /// Crate-internal constructor from the canonical TAI instant, tagging scale
    /// `S`. Scale conversions are exactly this (a re-tag of the same `tai`).
    pub(crate) fn from_tai_raw(tai: P::Repr) -> Self {
        Self {
            tai,
            _scale: PhantomData,
        }
    }

    /// Crate-internal accessor for the canonical TAI instant.
    pub(crate) fn tai_raw(&self) -> P::Repr {
        self.tai
    }

    /// Exact SI-second interval since `earlier`, measured on the shared TAI
    /// timeline — correct across leap seconds and across *any* scales (both
    /// epochs store canonical TAI, so `earlier` may be in a different scale).
    ///
    /// Both epochs must share the precision tier `P`; difference a mixed pair by
    /// first aligning tiers with [`to_precision`](Self::to_precision).
    pub fn duration_since<T: TimeScale>(&self, earlier: &Epoch<T, P>) -> Duration {
        Duration::from_si_seconds(self.tai.diff_days(earlier.tai_raw()) * 86400.0)
    }

    /// Re-express this epoch at precision tier `Q`, preserving the same scale
    /// `S` and canonical TAI instant.
    ///
    /// The instant is carried across via its `(hi, lo)` parts: [`Precise`] →
    /// [`Precise`] is exact, [`Coarse`] → [`Precise`] lifts the `f64` (residual
    /// zero), and [`Precise`] → [`Coarse`] collapses to `hi + lo` (dropping the
    /// sub-`f64` residual the coarse tier cannot hold).
    pub fn to_precision<Q: Precision>(&self) -> Epoch<S, Q> {
        let (hi, lo) = self.tai.parts();
        Epoch::<S, Q>::from_tai_raw(<Q::Repr as JdRepr>::from_parts(hi, lo))
    }
}

// Read-out accessors: lens the stored canonical TAI into scale `S`'s JD.
//
// Concrete per scale (not a generic `impl<S: TaiLens, P>`) so the crate-internal
// `TaiLens` trait stays off the public API — a public generic bounded by it
// would leak it (`private_interfaces`/`private_bounds`).
macro_rules! impl_jd_accessors {
    ($scale:ty) => {
        impl<P: Precision> Epoch<$scale, P> {
            /// Return the Julian Date, interpreted in this scale (lens read-out
            /// of the canonical TAI instant; single-`f64` collapse).
            pub fn jd(&self) -> f64 {
                <$scale as TaiLens>::scale_from_tai(self.tai).jd()
            }

            /// The Julian Date as a precise two-part `(hi, lo)` value — for
            /// feeding SOFA-style two-part consumers without the `jd()` collapse.
            ///
            /// On the [`Precise`] tier `lo` carries the conversion's sub-`f64`
            /// residual; since every constructor currently ingests a single
            /// `f64`, it reflects the lens arithmetic, not user-supplied sub-`f64`
            /// input precision (a two-part ingestion path is future work). On the
            /// [`Coarse`] tier `lo` is always `0.0`.
            pub fn jd_parts(&self) -> (f64, f64) {
                <$scale as TaiLens>::scale_from_tai(self.tai).parts()
            }

            /// Return the Modified Julian Date, interpreted in this scale.
            pub fn mjd(&self) -> f64 {
                self.jd() - MJD_OFFSET
            }
        }
    };
}
impl_jd_accessors!(Utc);
impl_jd_accessors!(Tai);
impl_jd_accessors!(Tt);
impl_jd_accessors!(Tdb);
impl_jd_accessors!(Gps);

// Epoch<Utc> constructors (main user-facing scale).
//
// Constructors build the default `Precise` tier. They stay on the concrete
// `impl Epoch<Utc>` (= `Epoch<Utc, Precise>`) rather than `impl<P> Epoch<Utc, P>`
// so that bare calls like `Epoch::from_jd(x)` infer the tier — a default type
// parameter is not a fallback for an unconstrained inference variable in
// expression position. For a `Coarse` epoch, build `Precise` then
// [`to_precision`](Self::to_precision).

impl Epoch<Utc> {
    /// Create a UTC epoch from a Julian Date (treated as UTC JD).
    ///
    /// `Epoch<Utc>::from_jd(x).jd() == x` (round-trip identity). The value is
    /// stored internally as the corresponding canonical TAI instant.
    pub fn from_jd(jd: f64) -> Self {
        convert::from_scale_jd::<Utc, Precise>(jd)
    }

    /// Create a UTC epoch from a Modified Julian Date value.
    pub fn from_mjd(mjd: f64) -> Self {
        Self::from_jd(mjd + MJD_OFFSET)
    }

    /// The J2000.0 reference epoch (JD 2451545.0).
    ///
    /// 歴史的には J2000.0 = 2000-01-01 12:00:00 TT だが、本実装では
    /// UTC scale で JD 2451545.0 を返す (後方互換のため)。厳密な TT J2000
    /// を得るには [`Epoch::<Tt>::from_jd_tt`] を使う。
    pub fn j2000() -> Self {
        Self::from_jd(J2000_JD)
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

        Self::from_jd(jd)
    }

    /// Parse a UTC epoch from ISO 8601 (CCSDS-compatible).
    ///
    /// Accepts both the calendar form `YYYY-MM-DDTHH:MM:SS[.fff]` and the
    /// ordinal / day-of-year form `YYYY-DDDTHH:MM:SS[.fff]` (used by CCSDS
    /// OMM). The `Z` suffix is optional — the timestamp is interpreted as UTC
    /// either way. Returns `None` if parsing fails.
    pub fn from_iso8601(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_suffix('Z').unwrap_or(s);
        let (date, time) = s.split_once('T')?;

        // Time of day, shared by both date forms.
        let (hour_s, rest) = time.split_once(':')?;
        let (min_s, sec_s) = rest.split_once(':')?;
        let hour: u32 = hour_s.parse().ok()?;
        let min: u32 = min_s.parse().ok()?;
        let sec: f64 = sec_s.parse().ok()?;
        if hour > 23 || min > 59 || sec >= 60.0 {
            return None;
        }

        let (year_s, rest) = date.split_once('-')?;
        let year: i32 = year_s.parse().ok()?;
        match rest.split_once('-') {
            // Calendar date: YYYY-MM-DD.
            Some((month_s, day_s)) => {
                let month: u32 = month_s.parse().ok()?;
                let day: u32 = day_s.parse().ok()?;
                if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
                    return None;
                }
                Some(Self::from_gregorian(year, month, day, hour, min, sec))
            }
            // Ordinal date: YYYY-DDD (zero-padded 3-digit day of year). The
            // 3-digit requirement disambiguates from a truncated calendar date
            // such as "2024-03", which is rejected rather than read as day 3.
            None => {
                if rest.len() != 3 {
                    return None;
                }
                let doy: u32 = rest.parse().ok()?;
                // Reject day 366 in a common year (it would roll into the next).
                let max_doy = if Self::is_leap_year(year) { 366 } else { 365 };
                if !(1..=max_doy).contains(&doy) {
                    return None;
                }
                let day_of_year =
                    doy as f64 + (hour as f64 * 3600.0 + min as f64 * 60.0 + sec) / 86400.0;
                Some(Self::from_year_day_of_year(year, day_of_year))
            }
        }
    }

    /// Create a UTC epoch from the current system time.
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        let unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_secs_f64();
        Self::from_jd(UNIX_EPOCH_JD + unix_secs / 86400.0)
    }

    /// Create a UTC epoch from a 4-digit year and a fractional day of year
    /// (`1.0` = Jan 1 00:00, `1.5` = Jan 1 12:00, …).
    pub fn from_year_day_of_year(year: i32, day_of_year: f64) -> Self {
        // JD of Jan 1 00:00 of that year, offset by the fractional day.
        let jan1_jd = Self::from_gregorian(year, 1, 1, 0, 0, 0.0).jd();
        Self::from_jd(jan1_jd + (day_of_year - 1.0))
    }

    /// Gregorian leap-year test.
    fn is_leap_year(year: i32) -> bool {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
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
        Self::from_year_day_of_year(year, day_of_year)
    }
}

// Epoch<Utc> methods (available on every precision tier; `P` is inferred from
// `self`, so these need no concrete-tier pinning).

impl<P: Precision> Epoch<Utc, P> {
    /// Julian centuries since J2000.0, computed directly from the UTC JD.
    ///
    /// **Note**: This treats the UTC JD as if it were a dynamical-time JD,
    /// which is strictly incorrect for high-precision ephemeris calculations.
    /// For Meeus/JPL DE usage, prefer `epoch.to_tdb().centuries_since_j2000()`.
    /// This method is kept for legacy bit-level compatibility where UTC
    /// centuries were used interchangeably with dynamical-time centuries.
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd() - J2000_JD) / JULIAN_CENTURY
    }

    /// Advance the epoch by `dt` seconds using naive UTC-JD arithmetic
    /// (`utc_jd + dt/86400`). Does NOT handle leap second boundaries.
    ///
    /// Legacy API. For leap-second-aware (SI-second) arithmetic use
    /// [`add_si_seconds`](Self::add_si_seconds) instead.
    pub fn add_seconds(&self, dt: f64) -> Self {
        convert::from_scale_jd::<Utc, P>(self.jd() + dt / 86400.0)
    }

    /// Advance the epoch by `dt` SI seconds, handling leap second boundaries.
    ///
    /// Adds `dt` directly on the stored canonical TAI instant (a uniform SI
    /// timeline), so leap seconds are handled automatically on read-out:
    /// 5 SI seconds from 2016-12-31T23:59:58 lands at 2017-01-01T00:00:02
    /// (not 00:00:03), because one SI second is "consumed" by the 2017-01-01
    /// leap.
    pub fn add_si_seconds(&self, dt: f64) -> Self {
        Self::from_tai_raw(self.tai_raw().add_days(dt / 86400.0))
    }

    /// Convert to Gregorian calendar date and time (UTC).
    pub fn to_datetime(&self) -> DateTime {
        to_datetime_from_jd(self.jd())
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
    /// canonical form use [`Ut1Epoch::era`] after an explicit UT1
    /// conversion via a proper EOP provider.
    ///
    /// Kept on `Epoch<Utc>` for bit-level compatibility with the pre-refactor
    /// `Epoch::gmst` method. Will be removed when downstream callers migrate
    /// to [`Ut1Epoch::era`].
    pub fn gmst(&self) -> f64 {
        era_formula(self.jd())
    }
}

#[cfg(test)]
mod tests;
