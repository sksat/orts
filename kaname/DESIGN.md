# kaname — 測地学・天文ライブラリ

軌道力学シミュレータから独立して利用可能な測地学・天文計算ライブラリ。
ワークスペース内の他クレートに依存しない。

## 責務と範囲

- 座標系の定義と変換（ECI/ECEF の各種バリエーション、WGS-84 Geodetic）
- 時刻系（Epoch / Julian Date / ERA / scale 間変換）
- 天体暦（太陽・月・惑星の位置）
- 天体の body-fixed 座標系 (IAU rotation model)

**スコープ外**: ケプラー要素（離心率, 軌道傾斜角等）は軌道力学の概念であり orts-orbits に所属する。
kaname は「地球と天球の幾何学」を扱い、「軌道の力学」は扱わない。

## 設計判断

### 座標系の型安全性

`Frame` trait と phantom 型 `Vec3<F>` / `Rotation<From, To>` で座標系をコンパイル時に区別する。
生の `Vector3<f64>` を使い回さない。

#### Frame category

sealed trait として以下のカテゴリを定義する:

- **`Eci`** — Earth-centered inertial の structural category。実装者: `SimpleEci`、`Gcrs`、`Cirs`
- **`Ecef`** — Earth-centered Earth-fixed の structural category。実装者: `SimpleEcef`、`Itrs`、`Tirs`
- **`LocalOrbital`** — 軌道ローカル系の structural category。実装者: `Rsw` (Radial / Along-track / Cross-track)
- **`BodyFixed<Body>`** — 天体固定系

category trait は structural operation (magnitude / dot / cross / 非依存 math) のための共通 interface として
使う。precision-aware な変換 (例: IAU 2006 CIO chain) は concrete 型 `Vec3<Gcrs>` / `Vec3<Itrs>` を
引数型に直接書き、近似と厳密の silent 混同は Rotation constructor の concrete typing で防ぐ。

#### 精度別 frame の役割

- `SimpleEci` / `SimpleEcef` — 歳差・章動・極運動・frame bias を全て無視した「ERA-only Z 回転の親/先」。
  可視化グレードの用途。名前の `Simple` prefix が精度警告として機能する
- `Gcrs` (Geocentric Celestial Reference System) — IAU 2006 CIO chain の celestial side
- `Cirs` (Celestial Intermediate Reference System) — IAU 2006 CIO chain 中間
- `Tirs` (Terrestrial Intermediate Reference System) — polar motion 未適用の Earth-fixed
- `Itrs` (International Terrestrial Reference System) — polar motion 適用済み、Geodetic 変換はこの frame に紐づく

`Rotation<SimpleEci, Gcrs>` / `Rotation<SimpleEcef, Itrs>` のような「簡易 path から高精度 path への
upgrade 変換」は提供しない。silent な degradation 経路を作らないため。

### 時刻系と座標系・測地系の定義レベルの結合

時刻系は暦 (Gregorian/ISO 8601) の表示だけに閉じる問題ではなく、**特定の reference frame や地球回転と
一次資料レベルで結合している**。設計の前提として以下を置く (IAU 2000 Resolutions B1.3/B1.5/B1.8/B1.9 と
IERS Conventions 2010):

#### proper time vs coordinate time

- **Proper time**: 特定の世界線上の物理時計が測る時間 (物理概念)
- **Coordinate time**: 4 次元時空座標系の時間パラメータ (数学概念)

IAU 2000 B1.9 以降、**TT は純然たる proper time ではなく、TCG との linear scale で defined された
"coordinate-derived time scale"** として再定義されている (`dTT/dTCG = 1 - L_G`, L_G = 6.969290134×10⁻¹⁰
は defining constant)。同様に **TDB は IAU 2006 B3 により**:

```
TDB = TCB - L_B × (JD_TCB - T0) × 86400 + TDB0
L_B  = 1.550519768×10⁻⁸
T0   = 2443144.5003725 (JD)
TDB0 = -6.55×10⁻⁵ s
```

#### Time scale ↔ reference frame 対応

| Time scale | 種別 | 紐づく frame | kaname での扱い |
|---|---|---|---|
| **TCB** | coordinate time | BCRS | out of scope |
| **TDB** | coordinate-derived time | BCRS | `Epoch<Tdb>`。Meeus / JPL DE (Teph ≈ TDB) の入力、IAU 2009 body rotation の formally な独立変数 |
| **TCG** | coordinate time | GCRS | out of scope |
| **TT** | coordinate-derived time (TCG linear scale) | — (TAI で実現) | `Epoch<Tt>`。IAU 2006 precession / IAU 2000A/B nutation の独立変数。TAI + 32.184 s |
| **TAI** | proper time (実装) | global clock ensemble | `Epoch<Tai>`。atomic clock ensemble の合成時刻 |
| **UT1** | Earth rotation angle time scale | CIRS ↔ TIRS 境界 | `Epoch<Ut1>`。defining observable は ERA = 2π × (0.7790572732640 + 1.00273781191135448 × (JD_UT1 − 2451545.0)) (IAU 2000 B1.8 / SOFA `iauEra00`)。atomic clock が刻む時刻ではなく Earth の瞬間的な向きを時間単位で表現したもの |
| **UTC** | hybrid (operational) | — | `Epoch<Utc>`。TAI + leap seconds。rate は TAI を継承、leap second で UT1 に 0.9 秒以内同期。`from_iso8601` / `from_gregorian` 等の default 入口 |

#### Frame rotation の time scale は definitional (conventional ではない)

- `Rotation<Gcrs, Cirs>::iau2006(tt)` — `Epoch<Tt>` 必須 (precession/nutation は TT centuries で定義)
- `Rotation<Cirs, Tirs>::from_era(ut1)` — `Epoch<Ut1>` 必須 (ERA は UT1 の definitional な関数)
- `Rotation<Tirs, Itrs>::polar_motion(utc, eop)` — `Epoch<Utc>` (IERS EOP が UTC MJD でインデックスされる conventional)
- `sun_position(tdb)` / `moon_position(tdb)` / `planet_position(tdb)` — `Epoch<Tdb>` 必須 (Meeus / JPL DE は dynamical time 系を入力とする)
- IAU 2009 body rotation (Earth/Moon/Mars 等) — `Epoch<Tdb>` 必須 (Archinal et al. 2011 の W/α/δ 多項式は "interval in Julian days from J2000 in TDB")

### 時刻の primitive

- `Epoch<S>` — scale `S` で解釈される瞬間
- `Duration` — SI 秒で計測した scale 非依存の時間間隔
- `DateTime` — Gregorian calendar 表示。`Epoch<Utc>` からのみ生成可能
- `TimeScale` trait — sealed category trait
- `TimeInterval<S>` / `Rate<S>` / `Epoch<Tcg>` / `Epoch<Tcb>` / `Epoch<Gps>` / `CivilDuration` 等は out of scope

### Scale-specific constructors と methods

scale 間の silent 混同を防ぐため、API 入口を scale 固有に分ける:

- `Epoch<Utc>::from_gregorian` / `from_iso8601` / `from_datetime` / `now` / `from_tle_epoch`
- `Epoch<Tt>::from_jd_tt`, `Epoch<Tdb>::from_jd_tdb`, `Epoch<Ut1>::from_jd_ut1`
- `Epoch<Ut1>::era()` — 他の scale には `era()` / `gmst()` を生やさない。`Epoch<Tdb>::era()` はコンパイルエラー
- `Epoch<S>::centuries_since_j2000()` は scale ごとに別実装 (同じ f64 式だが TT centuries / TDB centuries 等と意味が異なる)

### Scale 変換

```
Epoch<Utc>.to_tai()     → leap second table
Epoch<Utc>.to_ut1(eop)  → UT1 = UTC + dUT1 (EopProvider 依存)
Epoch<Tai>.to_tt()      → const 32.184 s
Epoch<Tt>.to_tdb()      → Fairhead-Bretagnon periodic series (< 2 ms)
Epoch<Utc>.to_tdb()     → UTC → TAI → TT → TDB の chain
```

cross-scale subtraction (`Epoch<Utc> - Epoch<Tt>`) は型エラー。明示的な scale 変換後に同一 scale での
`Epoch<S> - Epoch<S> → Duration` を要求する。

### Duration と leap second 境界

`Duration` は SI 秒単位 (TAI 時間で計測)。`Epoch<Utc>` への加算は leap second 境界を正しく扱う:

```rust
let e  = Epoch::<Utc>::from_iso8601("2016-12-31T23:59:58Z")?;
let e2 = e.add_si_seconds(5.0);  // 2017-01-01T00:00:01Z
```

`Epoch<Utc>.to_datetime()` は leap second instant で `23:59:60` を返す。leap second 非対応の外部連携用に
`to_datetime_normalized()` で `00:00:00` 側に繰り上げる別 API を提供する。

UTC calendar arithmetic (「翌日同時刻」「翌月同日」) は kaname の scope 外。必要になれば `CivilDuration`
相当の API を別途追加する。

### leap second は geodetic 現象

Earth rotation の secular slowdown (潮汐摩擦) が原因。leap second table を kaname 内 compiled-in で持つ。
外部 EOP provider とは完全に別体系 (更新 cadence / 意味論が異なる)。

### EOP (Earth Orientation Parameters) はパラメータごとの trait に分割する

`Ut1Offset` / `PolarMotion` / `NutationCorrections` / `LengthOfDay` の 4 trait に分割し、`NullEop` placeholder
はこれらの trait を一つも実装しない。precise な rotation API (`iau2006_full` 等) は対応する trait bound で
gate され、`NullEop` を渡すと compile error になる。

実装は外部 crate (tobari または新 crate) が IERS Bulletin A/B / CSSI 等から読み込んで trait を提供する。
kaname 自体は data file を持たない (WASM compatibility)。

### Reference ellipsoid と geodetic 変換

座標 frame と reference ellipsoid は直交する二つの設計軸。frame は座標 origin と axis を、ellipsoid は
地球形状の数学的近似を定義する。Cartesian 座標には ellipsoid は影響せず、geodetic (lat/lon/height) 変換
時にのみ必要になる。

`ReferenceEllipsoid` trait + `Wgs84` / `Grs80` / `Iers2010` / `MoonSphere` / `MarsSpheroid` の concrete を
定義する。実装は WGS84 のみ (他は定数定義のみの scaffolding)。ITRS の formal ellipsoid は GRS80 だが、
WGS84 との差は polar radius で ~0.1 mm しかなく、satellite dynamics の精度要求では無視できる。

`Geodetic<E: ReferenceEllipsoid>` を parameterize し、default alias `pub type Geodetic = Geodetic<Wgs84>`
で ergonomics を維持する。`Vec3<Itrs>::to_geodetic()` は default (WGS84) を返し、
`Vec3<Itrs>::to_geodetic_with::<E>()` で明示指定可能。

`GeoidModel` trait / `AstronomicalLatitude` struct (vertical deflection marker) / `Datum` trait は
scaffolding のみ提供し、実装は out of scope:

- geoid model (EGM2008 等) による orthometric height
- Moon/Mars body-fixed での selenographic/areographic 変換
- vertical deflection の実数値計算
- datum 変換 (WGS84 ↔ NAD83 等)
- ITRF realization 間の epoch propagation

### `geodetic_altitude()` の設計

ECI 位置ベクトルから WGS-84 測地高度を直接計算するユーティリティ。
`p = sqrt(x² + y²)` と `z` は Z 軸回転不変なので GMST 不要。
大気抵抗モデル（tobari）での利用を想定した設計。

### 天体暦の精度レベル

Meeus "Astronomical Algorithms" に基づく低精度解析解を採用。
太陽位置で ~0.35°（vs DE405）、月位置で ~1% 距離誤差がある。

高精度暦（JPL DE430 等）への拡張は将来の選択肢。現状の軌道力学シミュレーションでは十分な精度。

## 単位規約

- 位置: km
- 速度: km/s
- 角度: rad
- 時刻: Julian Date（日）
- scale-specific JD は `Epoch<S>::jd()` 経由で取得 (どの scale で解釈するかは型で明示)
- SI 秒 (TAI 秒) は `Duration` で表現
