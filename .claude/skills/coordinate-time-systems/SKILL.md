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
LLMs. The two recurring traps:

1. **Frames and time scales are coupled at the *definitional* level.** A time
   scale is often the **independent variable** of a transform, not a decorative
   label. ERA is *defined* as a function of UT1; precession/nutation series are
   *defined* in TT centuries; ephemerides take TDB. Feeding the wrong scale into
   a rotation isn't a units bug you can scale away — it's the wrong physics.
2. **Names are overloaded — and the everyday ones are ambiguous by
   themselves.** The casual labels you get handed — "ECI", "ECEF", "inertial",
   "GMT", "epoch", "GPS time", "Zulu" — under-specify the actual system, its
   realization, and (for time) the scale. "ECI" might mean J2000/EME2000, GCRS,
   TEME, MOD, or TOD; "ECEF" might mean ITRF, WGS84, or PEF; "GMT" conflates UT1
   and UTC. Even the precise names collide: "J2000", "GCRS", and "ICRF" are
   *not* identical (frame bias ~23 mas); "RTN", "RSW", and "RIC" name one idea
   but tools disagree on axis order/sign; TEME looks like an ECI but is its own
   quasi-inertial frame. **A name alone never pins a value — always resolve
   *which system*, *which realization*, and *which scale*.** This is exactly why
   orts bakes the answer into the type: `SimpleEci` names its approximation in
   the type, and `Epoch<S>` forces a scale to be chosen.

## Reference System ≠ Reference Frame

A distinction worth internalizing (IERS/IAU usage), because the type names hide it:

- A **Reference System** is the *conceptual definition* — the conventions,
  models, and constants that define how the axes are oriented (e.g. **ICRS**,
  **GCRS**, **ITRS** are *systems*). Abstract; you cannot measure against it
  directly.
- A **Reference Frame** is a *realization* of a system — a concrete catalog of
  fiducial coordinates/EOP that materializes those axes from data (e.g.
  **ICRF3** realizes ICRS; **ITRF2014/ITRF2020** realize ITRS; the **WGS84**
  G-realizations are aligned to ITRF at the cm level).

So "realization" is an independent axis from "which system". orts' markers
(`Gcrs`, `Itrs`, `Cirs`, `Tirs`) are named after **systems**, yet the phantom
parameter is called `Frame` and a `Vec3<F>` is treated as living in a frame.
For Earth-satellite dynamics the realization difference is usually sub-cm and
ignored deliberately — but the *concept* matters the moment you ingest external
data (which ITRF does this GNSS product use? which EOP series? what ICRF
realizes the catalog?). Don't let the type name lull you into thinking the
realization question has been answered; it usually just hasn't been asked.

## What is authoritative here, and what is not

- **Authoritative = external standards.** IERS Conventions (2010, TN36), IAU
  resolutions (2000 B1.x, 2006 B3), SOFA/ERFA, Vallado *Fundamentals of
  Astrodynamics and Applications*, CCSDS ODM. The domain *truth* lives there.
  Catalogs with citations: [references/frames.md](references/frames.md) and
  [references/time-scales.md](references/time-scales.md).
- **NOT authoritative = orts/arika's own design.** `arika/DESIGN.md` and the
  current type definitions describe the **AS-IS** design. orts' handling of
  coordinate and time systems is still **発展途上 (a work in progress)** — the
  design is one team's current snapshot, it has known gaps, and it changes.
  Treat in-repo docs as *"how orts does it today"*, never as ground truth for
  *"how it should be done"*. When the two disagree, the external standard wins
  and the orts side is a bug or a not-yet-done. Current snapshot + live gaps:
  [references/orts-current-state.md](references/orts-current-state.md).

## The core principle: express frame and scale in the type system

This is the design value orts commits to and the single best defense against
both traps above: **a frame or a time scale should be a type parameter, not a
convention you remember.**

- Position/velocity vectors carry their frame: `Vec3<F>` where `F: Frame`
  (`Gcrs`, `Itrs`, `Teme`, `Rsw`, …). Mixing frames is a compile error; you
  cross frames only through a `Rotation<From, To>`.
- Instants carry their scale: `Epoch<S>` where `S: TimeScale` (`Utc`, `Tai`,
  `Tt`, `Ut1`, `Tdb`, `Gps`). `Epoch<Utc> − Epoch<Tt>` does not compile; you
  convert explicitly first.
- A transform's *required* time scale is encoded in its signature so the wrong
  scale cannot be passed: `Rotation::<Gcrs,Cirs>::iau2006` takes `Epoch<Tt>`;
  `Rotation::<Cirs,Tirs>::from_era` takes `Epoch<Ut1>`; `sun_position` takes
  `Epoch<Tdb>`. `Epoch<Tdb>::era()` should not exist.

When you touch this code, the bar is: **make the physically-meaningful
operations expressible and the meaningless ones fail to compile.** A few
sharper rules that follow from it:

- **Don't launder a bare `Vector3<f64>` / `f64` JD across a boundary** where its
  frame or scale is ambiguous. `into_inner()` / `from_raw` are escape hatches —
  name them `_raw_unchecked`-style and keep them local, never on a public path.
- **Type the invariant, not the vibe.** Add a type when a value carries a real,
  easily-confused invariant or semantic (a frame, a scale, a continuous GPS week
  vs a 10-bit broadcast week). Do *not* add trait bounds for philosophical
  taxonomy (proper-time vs coordinate-time) that don't change what compiles —
  that misleads about precision. (This is exactly the line PR #192 draws.)
- **No silent upgrade between precision tiers.** `SimpleEci`→`Gcrs` or
  `SimpleEcef`→`Itrs` conversions are deliberately *not* provided: a simple
  (ERA-only, visualization-grade) value must not silently masquerade as a
  rigorous (full IAU 2006) one. The `Simple` prefix is the precision warning.
- **A blanket `impl<F: Frame>` over a precision-aware operation is a trap.** It
  lets an approximation written for one frame propagate, with no compile error,
  to a future frame where it's wrong. Prefer explicit per-frame impls so adding
  a frame *forces* you to write (or refuse) its handling — see #191/#193.

## "ちゃんと扱いたい": TAI and TEME

These two are the canonical examples of the hard cases, and both are *in flux*.

### TAI (and the TAI-bridge taxonomy)

TAI is the continuous atomic timeline; it is the natural **pivot** for every
data-free scale conversion. The clean model (PR #192, `worktree-time-scale-...`)
is: scales that are a fixed SI offset from TAI (`TAI`, `TT = TAI+32.184s`,
`GPS = TAI−19s`) implement one capability (`FixedOffsetFromTai`), and a single
internal `TaiConvertible` pivot routes conversions through TAI. UTC adds leap
seconds; TDB adds a periodic series; **UT1 deliberately stays off the bridge**
because it needs an EOP (ΔUT1) provider — so there is no `Epoch<Ut1>::to_tai()`
and the EOP dependency is visible in the type.

Practical guidance:
- New affine scale (e.g. GPS) ⇒ "one constant + one trait impl", nothing else.
  If you find yourself special-casing conversions, you're fighting the pivot.
- Don't conflate TAI with UTC: `GPS − UTC` *steps* at each leap second
  (17 s before 2017-01-01, 18 s after) even though `GPS − TAI` is constant.
- The internal representation is still a single `f64` JD (~tens-of-µs floor).
  Sub-µs work (`Jd2`/`TaiInstant`) is deliberately deferred — don't assume
  nanosecond fidelity from `Epoch` today.

### TEME (and SGP4/TLE/OMM)

TEME (True Equator, Mean Equinox) is the quasi-inertial frame SGP4 propagates
in. It is **not** J2000/EME2000 and **not** GCRS — treating a TEME state as ECI
is a classic, silent error (hundreds of metres to kilometres). Converting
TEME→GCRS needs GMST + the equation of the equinoxes (+ precession/nutation),
*not* a relabel.

In orts today (AS-IS): `Teme` exists as a `Frame` marker in the `Eci` category,
but **the TEME↔GCRS/SimpleEci rotation is not implemented**, and the element-set
parsers return *mean elements* (`omm::Omm`), not `Vec3<Teme>` state vectors. So
the type system currently prevents you from *accidentally* treating TEME as
GCRS (no rotation exists to misuse), but it also can't yet *do* the conversion.
If you implement it: the rotation must be frame-explicit (no blanket impl), take
the correct scale as its independent variable, and be cross-validated against a
reference (ERFA/Orekit) — see the workflow below.

## Before you change coordinate/time code

1. **Anchor on the standard, not on DESIGN.md.** Confirm the definition,
   independent variable, and error budget against the external sources in
   [references/](references/). The in-repo design is AS-IS and may be wrong.
2. **Check the live design state — this area moves weekly.** Open issues/PRs
   reshape the types here. As of 2026-06-23: **#191** (force models ignore the
   integration frame — false type-safety), **#151** (frame-generic propagation
   for rigorous GCRS), **#192** (TAI taxonomy + GPS Time), **#193** (frame-
   explicit third-body/SRP). Re-check with `gh issue list` / `gh pr list` and
   read the actual types before assuming the snapshot in
   [references/orts-current-state.md](references/orts-current-state.md) is current.
3. **Keep it expressible-or-uncompilable.** Add the marker/scale; encode the
   required scale in transform signatures; refuse silent precision upgrades and
   blanket precision-aware impls.
4. **Pin behavior before refactoring** (project rule). Characterization tests
   including boundary *and* non-finite inputs (`NaN`, `±∞`) — float predicate
   rewrites can agree on finite values yet diverge at `NaN`. For scale/JD
   arithmetic, pin bit-for-bit where behavior must be preserved.
5. **Use property-based testing for the invariants — frames and time scales are
   full of them.** (The technique is *property-based testing*; the workspace's
   implementation crate is `proptest = "1.10"`, already used in `orts` and
   `utsuroi` — `arika` doesn't use it yet and is the prime candidate.) Generated
   inputs catch the worst-case float that hand-picked cases miss. Good
   properties to assert (with an explicit tolerance — never `==` on transformed
   `f64`):
   - **Rotation round-trip:** `r.inverse().transform(r.transform(v)) ≈ v`.
   - **Magnitude preservation:** `|r.transform(v)| ≈ |v|` (rotations are
     isometries).
   - **Composition / associativity & identity:** `a.then(b).then(c)` ≈
     `a.then(b.then(c))`; `r.then(r.inverse()) ≈ identity`.
   - **Scale round-trip:** `utc.to_tai().to_utc() ≈ utc`; chained
     `to_tt().to_tdb()` and back within the model's stated tolerance.
   - **Duration across discontinuities:** `e.add_si_seconds(d) − e ≈ d` *even
     across a leap-second boundary* — a hand-written test rarely lands on one;
     proptest will.
   - **Generators must include the nasty inputs:** near-zero and parallel `r`,`v`
     (RSW/LVLH degeneracy → `None`), huge magnitudes, sub-µs JD deltas, and
     `NaN`/`±∞` where the function claims total behavior. Shrinking will hand
     you the minimal failing case.
   Frame *mixing* is enforced at compile time (not a runtime property) — that's
   the type system doing the proptest's job for free.
6. **Cross-validate against references.** ERFA for the IAU 2006/2000A chain,
   Orekit/GMAT for propagation and body rotations. See the `orekit-fixtures`
   skill and `arika/tests/` (`iau2006_vs_erfa.rs`, `iau_rotation_orekit.rs`).
7. **Sanity-check non-trivial designs with `smart-friend`/Codex** before coding,
   and get an external review before merge (project methodology).

## Key in-repo entry points (AS-IS references, verify against code)

| Concern | File |
|---|---|
| Frame markers, `Vec3<F>`, `Rotation<From,To>`, category traits | `arika/src/frame.rs` |
| Time scales (`Utc`/`Tai`/`Tt`/`Ut1`/`Tdb`), `Epoch<S>` | `arika/src/epoch/scale.rs`, `arika/src/epoch/mod.rs` |
| Scale conversions, ERA formula | `arika/src/epoch/convert.rs` |
| Leap second table | `arika/src/epoch/leap.rs` |
| IAU 2006 precession/nutation, CIO chain | `arika/src/earth/iau2006/` |
| EOP traits (`Ut1Offset`/`PolarMotion`/…) | `arika/src/earth/eop/` |
| Frame-aware environment bridge | `orts/src/environment.rs` (`EarthFrameBridge`) |
| Frame-parameterized state/forces | `orts/src/orbital/state.rs`, `orts/src/model.rs` |
| AS-IS design narrative (reference only) | `arika/DESIGN.md` |
