# orts / arika — AS-IS snapshot (frames & time scales)

> **This is a dated snapshot, not a spec.** orts' coordinate/time handling is
> 発展途上. The types below were read from the code on **2026-06-23**. Before
> relying on any detail, re-check the actual source and the live issues/PRs
> (`gh issue list`, `gh pr list`) — this area changes weekly. `arika/DESIGN.md`
> is the *intended* design and already drifts from the code in places (e.g. it
> documents `Rotation::<Gcrs,Cirs>::iau2006(tt)` while the code takes
> `(tt, utc, eop)`); trust the code, then the standards, then the doc.

## Time scales (`arika/src/epoch/`)

`Epoch<S>` where `S: TimeScale`. Internal representation is a **single `f64`
Julian Date interpreted in scale `S`** (`arika/src/epoch/mod.rs`). `Epoch`
without a parameter means `Epoch<Utc>`.

| Scale | Marker | Status |
|---|---|---|
| UTC | `Utc` (default) | implemented; main entry point (`from_gregorian`/`from_iso8601`/`from_datetime`/`now`/`from_tle_epoch`) |
| TAI | `Tai` | implemented (`from_jd_tai`) |
| TT | `Tt` | implemented (`from_jd_tt`); independent variable of IAU 2006/2000A series |
| UT1 | `Ut1` | implemented (`from_jd_ut1`, `era()`); EOP-dependent, kept off the TAI bridge |
| TDB | `Tdb` | implemented (`from_jd_tdb`); ephemeris / body-rotation independent variable |
| GPS | `Gps` | **in PR #192** (`FixedOffsetFromTai`, `GpsWeek`/`SecondsOfWeek`); not on `main` yet |

**Conversions** (actual method names; `arika/src/epoch/convert.rs`):

- `Epoch<Utc>`: `to_tai()`, `to_tt()`, `to_tdb()`, `to_ut1_naive()` (assumes
  ΔUT1 = 0, legacy), `to_ut1(eop)` (precise, needs a `Ut1Offset` provider)
- `Epoch<Tai>::to_tt()` (`+32.184 s`), `Epoch<Tt>::{to_tai(), to_tdb()}`,
  `Epoch<Tdb>::to_tt()` (Fairhead–Bretagnon periodic series, < 2 ms)
- Cross-scale subtraction is a **compile error** by design — convert first, then
  subtract within one scale.
- `Epoch<Utc>::add_si_seconds(dt)` crosses leap-second boundaries correctly;
  `to_datetime()` yields `23:59:60` at a leap instant, `to_datetime_normalized()`
  rolls to the next day for leap-unaware consumers.

**Constants:** `J2000_JD = 2451545.0`, `TT_MINUS_TAI_SEC = 32.184`.

## Frames (`arika/src/frame.rs`)

`Vec3<F>` (frame-tagged vector) and `Rotation<From, To>` (unit-quaternion
rotation). Category traits: `Eci`, `Ecef`, `LocalOrbital`. Markers also have a
runtime `FrameDescriptor` / `FrameCategory` for log/CLI boundaries.

| Marker | Category | Status / notes |
|---|---|---|
| `SimpleEci` | `Eci` | approximate ECI; parent of the ERA-only rotation. Visualization-grade. *No defined relation to `Gcrs`.* |
| `SimpleEcef` | `Ecef` | ERA-only rotation of `SimpleEci`; WGS-84 geodetic defined here |
| `Gcrs` | `Eci` | rigorous celestial side of the IAU 2006 CIO chain; **also the return type of the low-precision Meeus ephemerides**, so a `Vec3<Gcrs>` is not guaranteed rigorous |
| `Cirs` | `Eci` | CIO-chain intermediate (precession/nutation applied) |
| `Tirs` | `Ecef` | CIO-chain intermediate (ERA applied, polar motion not yet) |
| `Itrs` | `Ecef` | polar motion applied; rigorous Earth-fixed; geodetic conversions |
| `Teme` | `Eci` | **marker only** — TEME↔GCRS/SimpleEci rotation NOT implemented; parsers return mean elements (`omm::Omm`), not `Vec3<Teme>` |
| `Rsw` | `LocalOrbital` | Radial/Along-track/Cross-track, fixed axis order `[R̂, Ŝ, Ŵ]` (≠ the many LVLH variants) |
| `Body` | (none) | spacecraft body-fixed; no Eci/Ecef/LocalOrbital category |

**Rotations available:**

- Simple path (`frame.rs`): `Rotation::<SimpleEci, SimpleEcef>` (and reverse)
  via `from_ut1(Epoch<Ut1>)`, `from_utc_assuming_ut1_eq_utc(Epoch<Utc>)`
  (legacy ΔUT1 = 0), or `from_era(f64)`.
- **Full IAU 2006 CIO chain is implemented** (`arika/src/earth/iau2006/cio_chain.rs`):
  - `Rotation::<Gcrs, Cirs>::iau2006(tt, utc, eop)` — independent variable **TT**
  - `Rotation::<Cirs, Tirs>::from_era(ut1)` — independent variable **UT1**
  - `Rotation::<Tirs, Itrs>::polar_motion(tt, utc, eop)` — EOP indexed by **UTC**
  - `Rotation::<Gcrs, Itrs>::iau2006_full(tt, ut1, utc, eop)` and the convenience
    `iau2006_full_from_utc(utc, eop)`
  - The EOP `eop: &P` arg is gated by the relevant `Ut1Offset`/`PolarMotion`/
    `NutationCorrections` traits; `NullEop` won't compile here.
- **No `SimpleEci`→`Gcrs` or `SimpleEcef`→`Itrs` conversion** — deliberate, to
  block silent precision upgrades.
- **No `Teme`↔anything** rotation yet.

## Frame-aware propagation & forces (`orts/src/`)

- `OrbitalState<F: Eci = SimpleEci>`, `ExternalLoads<F: Eci = SimpleEci>`,
  `Model<S, F: Eci = SimpleEci>`, `HasOrbit::Frame: Eci` — all parameterized by
  the inertial frame, defaulting to `SimpleEci`.
- `EarthFrameBridge` (`orts/src/environment.rs`) bridges an ECI propagation
  frame to its Earth-fixed counterpart for geodetic/atmosphere/magnetic models:
  - `SimpleEci` → `Fixed = SimpleEcef`, `EopStorage = ()`, rotation from
    `to_ut1_naive().era()` (ERA only).
  - `Gcrs` → `Fixed = Itrs`, `EopStorage = GcrsEopStorage`, rotation from
    `iau2006_full_from_utc(utc, eop)` (full chain).
- `AtmosphericDrag<F: EarthFrameBridge>` is correctly frame-gated. PR #193
  narrows `ThirdBodyGravity`/`SolarRadiationPressure` from a blanket
  `impl<F: Eci>` to explicit `SimpleEci`+`Gcrs` impls.

## Known gaps & traps (with live issues)

- **#191 — force models ignore the integration frame's orientation (false
  type-safety).** The conservative forces (J2 zonal, third-body, SRP) compute
  the *same numbers* for `OrbitalSystem<Gcrs>` and `<SimpleEci>`: `ZonalHarmonics`
  hard-codes `position.z` as the Earth pole, and third-body/SRP use the Meeus
  `Vec3<Gcrs>` raw via `into_inner()`. The type system blocks `Vec3<A> + Vec3<B>`
  but does **not** catch "this force ignores frame orientation". Adding a
  GCRS-misaligned inertial frame (e.g. `Teme`) would silently produce wrong
  results. Root-cause plan: split `EarthFrameBridge` into capability traits
  (`EarthPoleBridge`/`EarthFixedBridge`/`EphemerisFrameBridge`). PR #190 pinned
  the limitation + honest docs; PR #193 is the first narrowing step.
- **#151 — frame-generic propagation** for rigorous GCRS; lifting the
  `SimpleEci`-fixed assumption across the model set.
- **TEME unimplemented** beyond the marker — see SKILL.md "ちゃんと扱いたい".
- **f64 JD precision floor** (~tens of µs). `Jd2`/`TaiInstant` redesign is
  deliberately deferred (see the time-scale redesign plan); don't assume
  sub-µs Epoch fidelity today.
- **Pre-1972 UTC** uses a constant 10 s TAI−UTC offset (no fractional-second
  rubber-second table) — Apollo-era dates carry ~km ephemeris error.
- **`to_ut1_naive()` / `from_utc_assuming_ut1_eq_utc`** ignore ΔUT1 (up to
  ±0.9 s ≈ ~0.46 km of LEO ground-track) — fine for visualization, wrong for
  precise Earth-fixed work; use the EOP-provided precise path there.

## Verifying currency

```sh
gh issue list --search "frame OR time OR coordinate OR epoch"
gh pr list --state all --search "arika OR frame OR scale"
# read the actual types, not this snapshot:
sed -n '1,120p' arika/src/frame.rs
sed -n '1,60p'  arika/src/epoch/scale.rs
```
