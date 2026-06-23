---
name: coordinate-time-systems
description: >-
  Reason about coordinate reference frames and time scales correctly (any
  astrodynamics work) — which frame/scale a value is actually in, the concerns
  to resolve before trusting it, and what to look up. Use when interpreting or
  choosing frames (ECI/ECEF, GCRS/ITRS/CIRS/TIRS, J2000/EME2000, TEME, ICRF,
  LVLH/RTN/RSW, body-fixed) or time scales (TAI/UTC/UT1/TT/TDB/GPS), or when
  reasoning about conversions, leap seconds, ERA/GMST, EOP/ΔUT1, polar motion,
  frame bias, or realizations. The companion skill `coordinate-time-typing`
  covers expressing and testing these in orts/arika code.
---

# Coordinate frames & time scales — the concerns

Getting frames and time scales right is genuinely hard — for humans and for LLMs
alike. This skill does **not** bake in a catalog of every frame and scale: the
landscape is large and it shifts (new frames, successive ITRF/ICRF realizations,
the planned end of leap seconds around 2035), so a frozen table goes stale and a
half-remembered one is worse than looking up the primary source on demand.
Instead it gives you the durable parts: the **concerns to resolve** for any
frame, scale, or transform, and **what to look up and where**.

For *how* to encode all of this in code — expressing frame and scale in the type
system, and property-based testing — see the companion skill
**`coordinate-time-typing`**.

## The concerns to resolve

For every frame, scale, or transform you touch, resolve the following. Look them
up (last section) rather than trusting memory or one tool's defaults.

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

## TAI and TEME — two worth understanding

**TAI** is the continuous atomic timeline — the one scale that never jumps. It is
the natural hub the others are defined against: the fixed-offset scales
(`TT = TAI + 32.184 s`, `GPS = TAI − 19 s`) differ from it by a constant; UTC
differs by an integer number of leap seconds (so `GPS − UTC` *steps* at each leap
second even though `GPS − TAI` is constant); TDB differs by a sub-millisecond
periodic series. **UT1 is the odd one out** — it is an Earth-rotation observable,
not an atomic scale, reachable only through an EOP (ΔUT1) measurement, so it
cannot be derived from TAI alone. Do duration arithmetic in TAI/TT, not UTC.

**TEME** (True Equator, Mean Equinox) is the quasi-inertial frame SGP4 propagates
in, and the frame TLE/OMM mean elements are expressed against. It is **not**
J2000/EME2000 and **not** GCRS — treating a TEME state as a generic ECI is a
classic, silent error that grows to kilometres. A correct TEME→GCRS conversion
applies GMST + the equation of the equinoxes (plus precession/nutation), and
depends on the model version you pick — it is never a relabel.

## Where to look it up — fresh, each time

Verify against the primary standard, not from memory and not from a single tool's
conventions:

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
