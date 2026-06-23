---
name: coordinate-time-systems
description: >-
  Handle coordinate reference frames and time scales correctly in orts/arika.
  Use when working with frames (ECI/ECEF, GCRS/ITRS/CIRS/TIRS, J2000/EME2000,
  TEME, LVLH/RTN/RSW, body-fixed), time scales (TAI/UTC/UT1/TT/TDB/GPS),
  `Epoch<S>`, `Vec3<F>`, `Rotation<From,To>`, frame or scale conversions, leap
  seconds, ERA/GMST/EOP, ephemeris/SGP4/TLE frames — or whenever you need to
  type, convert, or reason about the coordinate-or-time semantics of a value.
---

# Coordinate frames & time scales

Getting frames and time scales right is genuinely hard — for humans and for
LLMs alike. This skill has two halves: the **general cautions** that make the
domain treacherous, and the **how** — the two techniques that actually defend
against those traps: **expressing frame and scale in the type system**, and
**property-based testing**.

Verify domain facts against the authoritative standards rather than from memory
or from one tool's conventions: IERS Conventions (2010, TN36), IAU resolutions
(2000 B1.x, 2006 B3), SOFA/ERFA, Vallado *Fundamentals of Astrodynamics and
Applications*, CCSDS ODM. Full catalogs with citations and cross-tool alias
tables: [references/frames.md](references/frames.md),
[references/time-scales.md](references/time-scales.md).

---

## Part 1 — General cautions (what makes this hard)

### 1. A time scale is usually the *independent variable* of a transform, not a label

This is the deepest trap. Frames and time scales are coupled at the
**definitional** level: a transform is *defined* as a function of one *specific*
time scale, and feeding it the wrong scale is not a units error you can rescale
away — it is the wrong physics.

- Earth Rotation Angle (and sidereal time) is *defined* as a function of **UT1**.
- Precession/nutation series are *defined* in **TT** Julian centuries.
- Planetary/lunar ephemerides and body-rotation models take **TDB**.
- EOP tables (polar motion, ΔUT1) are *indexed* by **UTC** by convention.

So "what time is it" is never enough; you need "in which scale, for which
purpose". The exact couplings are in [references/time-scales.md](references/time-scales.md).

### 2. Names are overloaded — and the everyday ones are ambiguous by themselves

The casual labels you get handed — "ECI", "ECEF", "inertial", "GMT", "epoch",
"GPS time", "Zulu" — under-specify the actual system, its realization, and (for
time) the scale. "ECI" might mean J2000/EME2000, GCRS, TEME, MOD, or TOD;
"ECEF" might mean ITRF, WGS84, or PEF; "GMT" conflates UT1 and UTC. Even the
precise names collide: "J2000", "GCRS", and "ICRF" are *not* identical (frame
bias ~23 mas); "RTN", "RSW", and "RIC" name one idea but communities disagree on
axis order/sign; TEME *looks* like an ECI but is its own quasi-inertial frame.

**A name alone never pins a value — always resolve *which system*, *which
realization*, and *which scale*.**

### 3. Reference System ≠ Reference Frame

A distinction worth internalizing (IERS/IAU usage), because it is easy to lose:

- A **Reference System** is the *conceptual definition* — the conventions,
  models, and constants that define how the axes are oriented (e.g. **ICRS**,
  **GCRS**, **ITRS** are *systems*). Abstract; not directly measurable.
- A **Reference Frame** is a *realization* of a system — a concrete catalog of
  fiducial coordinates / EOP that materializes those axes from data (e.g.
  **ICRF3** realizes ICRS; **ITRF2014 / ITRF2020** realize ITRS; the WGS84
  G-realizations are aligned to ITRF at the cm level).

"Which realization" is an independent axis from "which system". For Earth-
satellite dynamics the realization difference is often sub-cm and ignored
deliberately — but the *concept* matters the moment you ingest external data
(which ITRF does this GNSS product use? which EOP series? which ICRF realizes
the star catalog?). A system name does not answer the realization question; that
question usually just hasn't been asked yet.

### Recurring concrete traps

Each is catalogued with its error budget and source in
[references/](references/): leap seconds and arithmetic across their boundaries;
ignoring ΔUT1 (up to ~0.9 s → ~0.46 km of LEO ground track) and using UTC where
UT1 is required; omitting polar motion; conflating J2000/GCRS (frame bias);
treating a TEME state as J2000/GCRS; GPS week rollover; TDB-vs-TT for ephemeris
lookups.

---

## Part 2 — The how

### How #1: Express frame and scale in the type system

This is the single best defense against every trap in Part 1: **a frame or a
time scale should be a type parameter, not a convention you have to remember.**
The wrong combination then fails to compile instead of silently producing the
wrong physics.

- Vectors carry their frame: `Vec3<F>` where `F` is a frame marker (`Gcrs`,
  `Itrs`, `Teme`, `Rsw`, …). Mixing frames is a compile error; you cross frames
  only through a typed `Rotation<From, To>`.
- Instants carry their scale: `Epoch<S>` where `S` is a scale marker (`Utc`,
  `Tai`, `Tt`, `Ut1`, `Tdb`, …). Subtracting two different scales does not
  compile; convert explicitly first.
- A transform's *required* scale (Part 1 trap #1) is encoded in its signature so
  the wrong scale cannot be passed: the GCRS→CIRS rotation takes an `Epoch<Tt>`,
  the CIRS→TIRS rotation takes an `Epoch<Ut1>`, a Sun-position call takes an
  `Epoch<Tdb>`. `Epoch<Tdb>::era()` should not exist.

The bar when you touch this code: **make the physically-meaningful operations
expressible and the meaningless ones fail to compile.** Sharper rules that
follow:

- **Don't launder a bare `Vector3<f64>` / `f64` JD across a boundary** where its
  frame or scale is ambiguous. Raw-access escape hatches (`into_inner`,
  `from_raw`) are sometimes necessary — keep them local and name them so the
  unchecked nature is obvious, never on a public path.
- **Type the invariant, not the vibe.** Add a type when a value carries a real,
  easily-confused invariant or semantic (a frame; a scale; a *continuous* week
  count vs a 10-bit *broadcast* GPS week). Don't add trait bounds for
  philosophical taxonomy (proper-time vs coordinate-time) that change nothing
  about what compiles — that just misleads about precision.
- **No silent upgrade between precision tiers.** A visualization-grade
  (e.g. ERA-only) value must not silently masquerade as a rigorous (full
  IAU 2006) one. Don't provide approximate→rigorous conversions; let a naming
  convention (e.g. a `Simple` prefix) carry the precision warning in the type.
- **A blanket `impl<F: Frame>` over a precision-aware operation is a trap.** It
  lets an approximation written for one frame propagate, with no compile error,
  to a future frame where it is wrong. Prefer explicit per-frame impls so adding
  a frame *forces* you to write — or deliberately refuse — its handling.

### How #2: Property-based testing for the invariants

Frames and time scales are *full* of algebraic invariants, and generated inputs
catch the worst-case float that hand-picked cases miss. (The technique is
*property-based testing*; in Rust the usual crate is `proptest`.) Assert with an
explicit tolerance — never `==` on a transformed `f64`:

- **Rotation round-trip:** `r.inverse().transform(r.transform(v)) ≈ v`.
- **Magnitude preservation:** `|r.transform(v)| ≈ |v|` (rotations are isometries).
- **Composition / associativity & identity:** `a.then(b).then(c)` ≈
  `a.then(b.then(c))`; `r.then(r.inverse()) ≈ identity`.
- **Scale round-trip:** `utc → tai → utc ≈ utc`; chained `tt → tdb` and back
  within the model's stated tolerance.
- **Duration across discontinuities:** `(epoch + d) − epoch ≈ d` *even across a
  leap-second boundary* — a hand-written test rarely lands on one; a generator
  will, and shrinking hands you the minimal failing case.
- **Generators must include the nasty inputs:** near-zero and parallel position/
  velocity (local-orbital-frame degeneracy), huge magnitudes, sub-µs JD deltas,
  and `NaN`/`±∞` where the function claims total behavior. (A float-predicate
  rewrite can agree on finite values yet diverge at `NaN`.)

Frame *mixing* itself is enforced at compile time, not as a runtime property —
that is How #1 doing the property test's job for free.

---

## TAI and TEME — two worth getting right

**TAI** is the continuous atomic timeline and the natural **pivot** for every
data-free scale conversion. Model the scales that are a fixed SI offset from TAI
(`TAI`, `TT = TAI + 32.184 s`, `GPS = TAI − 19 s`) as sharing one capability, and
route data-free conversions through a single TAI pivot. UTC adds leap seconds;
TDB adds a periodic series; **UT1 must stay off that pivot** because it is an
Earth-rotation observable reachable only through an EOP (ΔUT1) provider — keeping
it off makes the EOP dependency visible in the type (there is no scale-free
`UT1 → TAI`). Adding a new affine scale should then be "one constant + one trait
impl", nothing more. Don't conflate TAI with UTC: `GPS − UTC` *steps* at each
leap second even though `GPS − TAI` is constant.

**TEME** (True Equator, Mean Equinox) is the quasi-inertial frame SGP4
propagates in. It is **not** J2000/EME2000 and **not** GCRS — treating a TEME
state as ECI is a classic, silent error (hundreds of metres to kilometres).
Converting TEME→GCRS needs GMST + the equation of the equinoxes (plus
precession/nutation), *not* a relabel. When you implement such a conversion:
make the rotation frame-explicit (no blanket impl), take the correct scale as
its independent variable, and cross-validate against a reference (ERFA/Orekit).

---

## Workflow: changing coordinate/time code

1. **Anchor on the standard.** Confirm the definition, the independent-variable
   scale, and the error budget against the external sources in
   [references/](references/) before you write anything.
2. **Keep it expressible-or-uncompilable** (How #1): add the marker/scale; encode
   the required scale in transform signatures; refuse silent precision upgrades
   and blanket precision-aware impls.
3. **Pin behavior before refactoring** (project rule): characterization tests
   including boundary *and* non-finite inputs (`NaN`, `±∞`); pin bit-for-bit
   where behavior must be preserved.
4. **Add property-based tests** (How #2) for the round-trip / isometry /
   composition / leap-second invariants, with degenerate and non-finite
   generators.
5. **Cross-validate against references.** ERFA for the IAU 2006/2000A chain,
   Orekit/GMAT for propagation and body rotations — see the `orekit-fixtures`
   skill and `arika/tests/` (`iau2006_vs_erfa.rs`, `iau_rotation_orekit.rs`).
6. **Sanity-check non-trivial designs with `smart-friend`/Codex** before coding,
   and get an external review before merge (project methodology).

## References & where the types live

- [references/frames.md](references/frames.md) — coordinate frame catalog
  (celestial → terrestrial → local → body-fixed), with definitions, the
  transformation chains, the System-vs-Frame realizations, alias tables, and
  sources.
- [references/time-scales.md](references/time-scales.md) — time scale catalog
  (TAI/UTC/UT1/TT/TDB/TCB/TCG/GPS + GMST/GAST), the conversion graph, which
  conversions need EOP vs leap seconds vs constants, and sources.
- In this repo the frame/time types live in `arika/src/frame.rs` (`Vec3<F>`,
  `Rotation<From,To>`, frame markers, category traits) and `arika/src/epoch/`
  (`Epoch<S>`, scale markers, conversions, leap seconds); the IAU 2006 chain is
  under `arika/src/earth/`. `arika/DESIGN.md` records the design rationale.
