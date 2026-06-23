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
LLMs alike. This skill does **not** bake in a catalog of every frame and scale:
the landscape is large and it shifts (new frames, successive ITRF/ICRF
realizations, the planned end of leap seconds around 2035), so a frozen table
goes stale and a vague memory of one is worse than useless. Instead it gives you
three durable things:

1. the **concerns to resolve** for any frame, scale, or transform;
2. **what to look up, and where** — fresh each time, because the current
   authoritative text is simpler and more accurate than recall;
3. the **how** — the two techniques that defend against the traps: express
   frame and scale in the **type system**, and **property-based testing**.

---

## Part 1 — The concerns to resolve

For every frame, scale, or transform you touch, resolve the following. Look them
up (next section) rather than trusting memory or one tool's defaults.

### A time scale is usually the *independent variable* of a transform, not a label

This is the deepest trap. Frames and time scales are coupled at the
**definitional** level: a transform is *defined* as a function of one *specific*
scale, and feeding it the wrong scale is not a units error you can rescale away —
it is the wrong physics. So always ask *which scale* a transform or model
requires:

- Earth Rotation Angle (and sidereal time) is *defined* from **UT1**.
- Precession/nutation series are *defined* in **TT** Julian centuries.
- Planetary/lunar ephemerides and body-rotation models take **TDB**.
- EOP tables (polar motion, ΔUT1) are *indexed* by **UTC** by convention.

"What time is it" is never enough; you need "in which scale, for which purpose".

### A name never pins a value — resolve *which system*, *which realization*, *which scale*

The casual labels you get handed — "ECI", "ECEF", "inertial", "GMT", "epoch",
"GPS time", "Zulu" — under-specify the system, its realization, and (for time)
the scale. "ECI" might mean J2000/EME2000, GCRS, TEME, MOD, or TOD; "ECEF" might
mean ITRF, WGS84, or PEF; "GMT" conflates UT1 and UTC. Even precise names
collide: "J2000", "GCRS", and "ICRF" are *not* identical (frame bias ~23 mas);
"RTN", "RSW", and "RIC" name one idea but communities disagree on axis
order/sign; TEME *looks* like an ECI but is its own quasi-inertial frame. Pin the
exact meaning before you trust the value.

### Reference System ≠ Reference Frame

A distinction worth internalizing (IERS/IAU usage), because it is easy to lose:

- A **Reference System** is the *conceptual definition* — the conventions,
  models, and constants defining how the axes are oriented (**ICRS**, **GCRS**,
  **ITRS** are *systems*). Abstract; not directly measurable.
- A **Reference Frame** is a *realization* of a system — a concrete catalog of
  fiducial coordinates / EOP materializing those axes from data (**ICRF3**
  realizes ICRS; **ITRF2014 / ITRF2020** realize ITRS).

"Which realization" is an independent axis from "which system". For Earth-
satellite dynamics the realization difference is often sub-cm and ignored
deliberately — but it matters the moment you ingest external data, and a system
name does not answer the realization question; that question usually just hasn't
been asked yet.

### Pin it down as a tuple, not a name

A frame/scale reference is really a *tuple*, and a name fixes at most one or two
of its slots. Resolve every slot:

- **Orientation model + version.** Which precession/nutation/sidereal-time model
  realizes it (IAU-76/FK5 vs 2000A vs 2006), and which path — the CIO-based
  (X, Y, s, ERA) or the classical equinox-based (precession/nutation/GAST). Both
  are valid and agree to ~tens of µas; mixing pieces from the two is a bug.
- **Origin / center.** Orientation does not fix the point the coordinates are
  about — geocentric vs barycentric vs topocentric. Light-time, parallax, and
  aberration hinge on the origin, not the axes.
- **Frozen vs of-date.** Is the frame inertial-at-an-epoch (frozen) or does it
  rotate with the model (of-date)? TEME-of-epoch ≠ TEME-of-date; CCSDS encodes
  this as the `_INERTIAL` vs `_ROTATING` suffix. Distinct from the
  independent-variable scale.
- **Independent-variable time scale** (the first concern above).
- **Realization, and its currency.** Which ITRF/ICRF realization (concern above)
  — *and* whether the data materializing it is current: EOP tables, the
  leap-second table, the ITRF/ICRF realization, the DE ephemeris, and body-
  rotation reports all expire. Keep provenance/version with the value.
- **What the conversion requires:** EOP (ΔUT1, polar motion, dX/dY) vs a
  leap-second table vs a constant offset vs a periodic series vs nothing.
- **Handedness & active vs passive.** Rotation-*of-frame* (passive/alias — what a
  coordinate transform is) vs rotation-*of-vector* (active/alibi); plus the
  quaternion convention (Hamilton vs JPL, scalar-first vs scalar-last).
- **Ellipsoid / datum — orthogonal to the rotation frame.** WGS84 is both an
  ellipsoid (for geodetic lat/lon/height) and a TRF realization; don't fuse "the
  frame" with "the shape".
- **Position vs velocity.** A rotating↔inertial transform that is right for
  position is wrong for velocity without the **ω×r** term.
- **Error budget** vs the rigorous model (acceptable here?) and **units**
  (km vs m, rad vs deg, SI seconds vs days).

Each slot in that tuple is exactly what How #1 turns into a type or a
typed-conversion parameter.

### The traps that bite most

Durable, high-cost mistakes (rough magnitudes — verify specifics against the
sources below):

- **TEME treated as J2000/GCRS** — full accumulated precession+nutation since
  J2000.0, growing to tens–thousands of km over years. SGP4/TLE output is TEME
  only; convert explicitly.
- **Ignoring ΔUT1 / feeding UTC into sidereal time** — up to ~419 m of LEO ground
  track (ΔUT1 bound ±0.9 s × ~465 m/s). GMST/ERA are functions of UT1, not UTC.
- **Dropping polar motion** (TIRS→ITRF) — up to ~9 m of Earth-fixed surface error.
- **Leap-second arithmetic in UTC** — ~1 s (~465 m) error when differencing across
  a boundary; `23:59:60` is legal. Do duration math in TAI/TT.
- **TDB vs TT for ephemerides** — the ~1.7 ms annual term → ~50 m for Earth.
- **GPS 10-bit week rollover** — ~19.6 yr jumps; needs a pivot epoch.
- **J2000/EME2000 vs GCRS frame bias** — only ~0.8 m at LEO, but the *naming* trap
  is worse than the magnitude: SPICE `J2000` ≈ ICRF (bias *not* applied) while
  GMAT/Orekit `EME2000` *is* the biased frame — same string, different frame.
- **"LVLH" axis-convention split** — Vallado/STK (X=radial, Z=orbit-normal) vs
  CCSDS/Wertz (Z=nadir, Y=−orbit-normal): incompatible rotations under one name.

### Where to look it up — fresh, each time

Verify against the primary standard, not from memory and not from a single
tool's conventions:

- **IERS Conventions (2010, TN36)** — *the* source for the GCRS↔ITRS chain, the
  ERA formula, frame-bias constants, and CIP/CIO/TIO definitions.
- **IERS EOP products** (e.g. the `finals2000A` readme) — the exact column
  *units* (xp,yp in arcsec, ΔUT1 in s, dX/dY in **mas** — a classic 1000× trap),
  and Bulletin A (rapid+predicted) vs B (final).
- **IAU resolutions** (2000 B1.x, 2006 B3) — definitional authority for
  ICRS/GCRS/BCRS, TT/TCG, and the exact TDB transform; ICRF realizations.
- **SOFA / ERFA** — the canonical algorithms *and* reference values to test
  against (and which routine takes TT vs UT1).
- **Vallado, *Fundamentals of Astrodynamics and Applications*** — the astrodynamics
  community's conventions and worked reductions (RSW/NTW/SEZ, the equinox chain).
- **CCSDS ODM (502.0-B) + SANA registries** — the exact `REF_FRAME` /
  `TIME_SYSTEM` / orbit-relative tokens when exchanging data.
- **NASA SPICE / NAIF** — body-fixed and lunar frame authority (`IAU_*`,
  `ITRF93`, `MOON_PA`/`MOON_ME`); note SPICE `J2000` ≈ ICRF.
- A tool's own docs (Orekit / GMAT / STK / Skyfield) — use these to learn *that
  tool's* naming, never as the definition itself.

Prefer fetching the current authoritative text: the numbers and realizations
change, and a tool's defaults are not the standard.

---

## Part 2 — The how

### How #1: Express frame and scale in the type system

This is the single best defense against every concern in Part 1: **a frame or a
time scale should be a type parameter, not a convention you have to remember.**
The wrong combination then fails to compile instead of silently producing the
wrong physics.

- Vectors carry their frame: `Vec3<F>` where `F` is a frame marker (`Gcrs`,
  `Itrs`, `Teme`, `Rsw`, …). Mixing frames is a compile error; you cross frames
  only through a typed `Rotation<From, To>`.
- Instants carry their scale: `Epoch<S>` where `S` is a scale marker (`Utc`,
  `Tai`, `Tt`, `Ut1`, `Tdb`, …). Subtracting two different scales does not
  compile; convert explicitly first.
- A transform's *required* scale (Part 1) is encoded in its signature so the
  wrong scale cannot be passed: the GCRS→CIRS rotation takes an `Epoch<Tt>`, the
  CIRS→TIRS rotation takes an `Epoch<Ut1>`, a Sun-position call takes an
  `Epoch<Tdb>`. `Epoch<Tdb>::era()` should not exist.

The bar when you touch this code: **make the physically-meaningful operations
expressible and the meaningless ones fail to compile.** Sharper rules:

- **Don't launder a bare `Vector3<f64>` / `f64` JD across a boundary** where its
  frame or scale is ambiguous. Raw-access escape hatches (`into_inner`,
  `from_raw`) are sometimes necessary — keep them local and name them so the
  unchecked nature is obvious, never on a public path.
- **Type the invariant, not the vibe.** Add a type when a value carries a real,
  easily-confused invariant or semantic (a frame; a scale; a *continuous* week
  count vs a 10-bit *broadcast* GPS week). Don't add trait bounds for
  philosophical taxonomy (proper-time vs coordinate-time) that change nothing
  about what compiles — that just misleads about precision.
- **No silent upgrade between precision tiers.** A visualization-grade (e.g.
  ERA-only) value must not silently masquerade as a rigorous (full IAU 2006)
  one. Don't provide approximate→rigorous conversions; let a naming convention
  (e.g. a `Simple` prefix) carry the precision warning in the type.
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

1. **Anchor on the standard.** Resolve the Part 1 concerns by looking up the
   primary sources before you write anything — definition, independent-variable
   scale, error budget, conventions.
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

## Where the types live (in this repo)

- `arika/src/frame.rs` — `Vec3<F>`, `Rotation<From,To>`, frame markers, category
  traits.
- `arika/src/epoch/` — `Epoch<S>`, scale markers, conversions, leap seconds.
- `arika/src/earth/` — the IAU 2006 precession/nutation / CIO chain and EOP traits.
- `arika/DESIGN.md` — the design rationale (read the code alongside it).
