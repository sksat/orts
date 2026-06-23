---
name: coordinate-time-systems
description: >-
  Reason about coordinate reference frames and time scales correctly (any
  astrodynamics work) — which frame/scale/representation a value is actually in,
  the concerns to resolve before trusting it, and what to look up. Use when
  interpreting or choosing frames (ECI/ECEF, GCRS/ITRS/CIRS/TIRS, J2000/EME2000,
  TEME, ICRF, LVLH/RTN/RSW, body-fixed), time scales (TAI/UTC/UT1/TT/TDB/GPS),
  or state representations (Cartesian/Keplerian/equinoctial); or when reasoning
  about frame/scale conversions, covariance transforms, interpolation, leap
  seconds, ERA/GMST/GAST, EOP/ΔUT1, polar motion, frame bias, or realizations —
  and also when reviewing or verifying frame/time/coordinate work (use it as a
  checklist of the traps, even for code you could write unaided). The companion
  skill `coordinate-time-typing` covers expressing and testing these in
  orts/arika code.
---

# Coordinate frames & time scales — the concerns

Getting frames and time scales right is genuinely hard — for humans and for LLMs
alike. A capable model already knows the textbook mechanics cold (that
precession takes TT, ERA takes UT1, `GPS = TAI − 19 s`, a phantom-typed epoch is
a good idea). It does *not* reliably catch the structural traps below on its
own. So this skill spends its words on the **non-obvious concerns** and on
**what to look up**, not on a catalog you'd anyway re-derive — the frame/scale
landscape shifts (new realizations, the ~2035 end of leap seconds) and a frozen
table goes stale.

For *how* to encode all of this in code, see the companion skill
**`coordinate-time-typing`**.

## The concerns to resolve

### A time scale is usually the *independent variable* of a transform, not a label

A transform is *defined* as a function of one *specific* scale; the wrong scale
is the wrong physics, not a rescalable units error. ERA/sidereal time is defined
from **UT1**; precession/nutation series in **TT**; ephemerides/body rotation in
**TDB**; EOP tables are indexed by **UTC**. (The UT1 one is the bit most often
missed — "I have a UTC timestamp" is *not* enough for a sidereal angle.) A
related piece-level trap: **GMST + the equation of the equinoxes = GAST**, and
mixing a CIO-path rotation with an equinox-path sidereal term is subtly wrong —
pick one paradigm and stay in it.

### A name never pins a value — resolve *which system*, *which realization*, *which scale*

Everyday labels under-specify: "ECI" might mean J2000/EME2000, GCRS, TEME, MOD,
or TOD; "ECEF" might mean ITRF, WGS84, or PEF; "GMT" conflates UT1 and UTC. Even
precise names collide — "J2000", "GCRS", "ICRF" are not identical (frame bias
~23 mas), and "RTN"/"RSW"/"RIC" name one idea with communities disagreeing on
axis order/sign. Pin the exact meaning before you trust the value.

### Reference System ≠ Reference Frame

A **System** is the conceptual definition (ICRS, GCRS, ITRS — the conventions
and models). A **Frame** is a *realization* of it from data (ICRF3 realizes
ICRS; ITRF2014/ITRF2020 realize ITRS). "Which realization" is an independent
axis a system name does not answer — and it matters the moment you ingest
external data.

### Representation ≠ frame (and its singularities)

A separate axis from "which frame": *how* the state is parameterized. Cartesian,
Keplerian, equinoctial, spherical are different representations of the same
physical state in the same frame. **Keplerian is singular at e→0 and i→0,
spherical at the poles** — the canonical "my sun-sync (i≈98°, e≈0) test NaNs"
bug. Equinoctial/modified-equinoctial elements exist to remove those
singularities. Treat frame and representation as two independent choices.

### It's rarely just one position vector

The frame question repeats for every quantity, and some carry *extra* traps:

- **Velocity:** a rotating↔inertial transform that is right for position is wrong
  for velocity without the **ω×r** term.
- **Covariance:** a covariance rotates as **`R·P·Rᵀ`** (both sides), *never*
  `R·P`; and a Cartesian↔Keplerian/RTN covariance change needs the **Jacobian**
  of that nonlinear map, not a rotation. A covariance also has its own frame,
  representation, and epoch — track them like you track a vector's.
- **Measurements:** an observation lives in a sensor/topocentric frame at a
  *specific* time tag, and you must distinguish transmit vs receive epoch
  (radar/GNSS: the satellite state at signal emission, not reception); apparent
  direction ≠ geometric (aberration ~20.5″, deflection near the Sun).

### Resolve the rest as a tuple, not a name

A frame/scale reference is really a tuple; a name fixes at most one or two slots:
orientation **model + version** (IAU-76/FK5 vs 2000A vs 2006; CIO X/Y/s/ERA path
vs equinox precession/nutation/GAST path — both valid, mixing pieces is a bug);
**origin/center** (geocentric/barycentric/topocentric — light-time and parallax
hinge on this, not the axes); **frozen vs of-date** (TEME-of-epoch ≠ of-date;
CCSDS `_INERTIAL` vs `_ROTATING`); **realization + data currency** (EOP, leap,
ITRF/ICRF, DE ephemeris all expire — keep provenance with the value); **what the
conversion needs** (EOP vs leap table vs constant vs periodic series); **handedness
& active-vs-passive** (rotation-of-frame vs rotation-of-vector; quaternion
Hamilton vs JPL, scalar-first vs -last); **ellipsoid/datum** (orthogonal to the
rotation frame — WGS84 is both an ellipsoid and a TRF); **units** (km vs m, rad
vs deg, SI s vs days).

### The traps that bite most

Durable, high-cost mistakes (rough magnitudes — verify against the sources below):

- **TEME treated as J2000/GCRS** — accumulated precession+nutation since J2000.0,
  growing to tens–thousands of km. SGP4/TLE output is TEME only; convert.
- **Ignoring ΔUT1 / feeding UTC into sidereal time** — up to ~419 m of LEO ground
  track. **Dropping polar motion** — up to ~9 m of Earth-fixed error.
- **Interpolating in the wrong space** — lerp an ECEF position across a span and
  you lose the km of sagitta from Earth rotation (interpolate in ECI, then
  rotate); Euler-angle lerp ≠ quaternion SLERP; interpolating osculating elements
  through perigee is garbage.
- **Covariance done as `R·P`** instead of `R·P·Rᵀ`, or rotated when a Jacobian was
  needed.
- **Leap-second arithmetic in UTC** — do duration math in TAI/TT (`23:59:60` is
  legal). **TDB vs TT for ephemerides** — ~50 m for Earth.
- **J2000/EME2000 vs GCRS frame bias** — ~0.8 m at LEO, but the *naming* trap is
  worse: SPICE `J2000` ≈ ICRF (bias not applied) while GMAT/Orekit `EME2000` *is*
  the biased frame — same string, different frame.
- **"LVLH" axis-convention split** — Vallado/STK (X=radial, Z=normal) vs
  CCSDS/Wertz (Z=nadir, Y=−normal): incompatible rotations under one name.

## TEME and onboard clocks — two underestimated cases

**TEME** (the SGP4/TLE frame) is *not* J2000/GCRS — treating a TEME state as a
generic ECI is a silent km-scale error. A real TEME→GCRS conversion is GMST +
equation of the equinoxes (+ precession/nutation), model-version dependent —
never a relabel. (TAI is the natural hub for the *atomic* scales, but **UT1 is
reachable only through an EOP/ΔUT1 measurement**, never from TAI alone.)

**Onboard clocks** differ from a ground scale by a *rate*, not a constant: a GPS
SV clock runs ~38 µs/day fast (plus an eccentricity periodic term), and TCG/TCB
differ from TT by rate. Model the rate, don't subtract an offset.

## Where to look it up — fresh, each time

Verify against the primary standard, not from memory or one tool's conventions:

- **IERS Conventions (2010, TN36)** — the GCRS↔ITRS chain, ERA formula, frame-bias
  constants, CIP/CIO/TIO definitions.
- **IERS EOP products** (`finals2000A` readme) — exact column *units* (xp,yp
  arcsec; ΔUT1 s; dX/dY in **mas** — a 1000× trap), Bulletin A vs B.
- **IAU resolutions** (2000 B1.x, 2006 B3) — ICRS/GCRS/TT/TDB definitions; ICRF.
- **SOFA / ERFA** — canonical algorithms and reference values (and which routine
  takes TT vs UT1; note its two-part date arguments exist for precision).
- **Vallado** — astrodynamics conventions and worked reductions (RSW/NTW/SEZ,
  the equinox chain, the TEME reduction).
- **CCSDS ODM (502.0-B) + SANA registries** — exact `REF_FRAME`/`TIME_SYSTEM`
  tokens when exchanging data.
- **NASA SPICE / NAIF** — body-fixed and lunar frames (note SPICE `J2000` ≈ ICRF).
- Tool docs (Orekit/GMAT/STK/Skyfield) — for *that tool's* naming, never as the
  definition itself.
