---
name: coordinate-time-typing
description: >-
  Express coordinate reference frames, time scales, and state representations in
  the type system, and test them with property-based tests — durable design
  guidance for orts/arika that holds whether you extend the current types or
  redesign them. Use when implementing, refactoring, or rethinking frame/scale
  code: adding a frame, scale, or transform; typing vectors, epochs, covariances,
  or rotations; deciding which time scale a transform takes; isolating EOP
  dependencies; choosing an interpolation space; or writing tests for rotation
  and scale-conversion invariants. Companion to `coordinate-time-systems`, which
  covers the domain concerns (which frame/scale/representation, the traps).
---

# Typing & testing coordinate frames and time scales

The domain concerns — which scale a transform requires, which system/realization
a name means, representation-vs-frame, covariance and interpolation traps — live
in the companion skill **`coordinate-time-systems`**. This skill is the *how*:
two durable techniques that turn those concerns into code that cannot silently
go wrong. They apply equally when you extend the existing types and when you
redesign them, so treat what follows as design principles, not a description of
the current API. A capable model already reaches for scale-tagged epochs on its
own; the load-bearing, easily-missed parts are below — keep them prominent.

## Technique 1: put the frame, scale, *and representation* in the type

A frame, a time scale, and a state representation should each be a type
parameter, not a convention you remember. The wrong combination then fails to
compile instead of silently producing the wrong physics.

- **Tag vectors with their frame** — mixing two frames is a compile error; you
  cross frames *only* through a typed rotation. (In orts, `Vec3<F>` crossed by
  `Rotation<From, To>`.) **This is the bit most often skipped** — a strong
  implementer will tag the time scale but still pass a bare position vector whose
  frame is only a comment. Tag it.
- **Tag instants with their scale** — subtracting two different scales doesn't
  compile; convert explicitly. (In orts, `Epoch<S>`.)
- **Tag the representation too** — Cartesian vs Keplerian vs equinoctial is an
  independent type axis from the frame, and the elements carry singularities
  (e→0, i→0). Make a representation conversion an explicit, fallible step, not an
  implicit reinterpret.
- **Type the covariance, not just the mean.** A covariance carries the same
  frame/representation tags as the state. Typing it stops the two classic bugs:
  rotating it as `R·P` instead of **`R·P·Rᵀ`**, and changing representation by a
  rotation when the nonlinear map needs a **Jacobian**.
- **Encode a transform's required scale in its signature** — the precession step
  takes a TT epoch, the Earth-rotation step a UT1 epoch, an ephemeris call a TDB
  epoch; a method like `era()` exists only on the UT1 epoch.

The bar, whatever the shape of the types: **make the physically-meaningful
operations expressible and the meaningless ones fail to compile.** Sharper rules,
most useful when you are *changing* the design:

- **Don't launder a bare vector / JD across a boundary** where its frame, scale,
  or representation is ambiguous. Raw escape hatches are sometimes needed — keep
  them local and named so the unchecked nature is obvious (`_raw_unchecked`),
  never on a public path.
- **Type the invariant, not the vibe.** Add a type for a real, easily-confused
  invariant (a frame; a scale; a *continuous* week vs a 10-bit *broadcast* GPS
  week) — not for philosophical taxonomy (proper- vs coordinate-time) that
  changes nothing about what compiles and only misleads about precision.
- **No silent upgrade between precision tiers.** A visualization-grade
  (rotation-only) value must not masquerade as a rigorous one; don't provide
  approximate→rigorous conversions — let a naming convention (a `Simple` prefix)
  warn in the type.
- **A blanket `impl<F: Frame>` over a precision-aware operation is a trap.** It
  lets an approximation written for one frame propagate, with no compile error,
  to a future frame where it's wrong. Use explicit per-frame impls so adding a
  frame *forces* you to write — or refuse — its handling.

## Technique 2: property-based testing for the invariants

Frames, scales, and rotations are full of algebraic invariants, and generated
inputs catch the worst-case float hand-picked cases miss. (In Rust: `proptest`.)
Assert with an explicit tolerance — never `==` on a transformed `f64`:

- **Rotation round-trip & isometry:** `inverse(r)∘r ≈ identity`; `|r·v| ≈ |v|`.
- **Composition:** `(a∘b)∘c ≈ a∘(b∘c)`; `r ∘ inverse(r) ≈ identity`.
- **Scale round-trip:** `utc → tai → utc ≈ utc`, including a generator that lands
  *on a leap-second boundary* (where `(epoch + d) − epoch ≈ d` is easy to break).
- **Covariance:** stays symmetric positive-semidefinite under transform; a
  representation round-trip preserves it within tolerance.
- **Quaternion / angle hygiene:** results stay unit-norm (renormalize in
  propagation loops — products drift); angle differences handle wrap-around.
- **Generators must include the nasty inputs:** near-zero and parallel position/
  velocity (local-orbital-frame degeneracy), the element singularities (e→0,
  i→0), huge magnitudes, sub-µs time deltas, and `NaN`/`±∞` where the function
  claims total behavior.

Frame *mixing* is caught at compile time, not as a runtime property — that is
Technique 1 doing the property test's job for free.

## Designing the conversions and operations

- **Hub the time scales** (a one-liner the model already knows): route data-free
  conversions through a single continuous hub (TAI) so a fixed-offset scale is
  "one constant + one impl"; keep UT1 *off* that hub so its EOP dependency shows
  in the type.
- **Make precision-aware rotations frame-explicit and scale-correct.** A rotation
  that depends on the frame's orientation names its concrete frames (no blanket
  impl) and takes the right independent-variable scale. TEME→GCRS is the worked
  example: GMST + equation of the equinoxes (+ precession/nutation), a real
  conversion taking the correct scale — never a relabel — cross-validated against
  ERFA/Orekit.
- **Interpolate in the space where the path is linear**, and make the type reflect
  it: interpolate position in an inertial frame then rotate to ECEF (lerping ECEF
  loses the sagitta of Earth rotation); SLERP quaternions, don't lerp Euler
  angles; don't interpolate osculating elements through perigee.
- **Mind float precision at the representation boundary.** A single `f64` Julian
  Date floors resolution at tens of µs and *cancels catastrophically* differencing
  epochs near J2000 — carry time as a two-part/offset value (as SOFA does) when
  precision matters, and keep the precision tier visible in the type.

## Workflow: changing coordinate/time code

1. **Anchor on the standard first** (resolve the domain concerns via the companion
   skill against the primary sources) before writing.
2. **Keep it expressible-or-uncompilable** (Technique 1): tag frame/scale/
   representation; encode the required scale in signatures; refuse silent
   precision upgrades and blanket precision-aware impls.
3. **Pin behavior before refactoring** (project rule): characterization tests with
   boundary *and* non-finite inputs (`NaN`, `±∞`); bit-for-bit where required.
4. **Add property-based tests** (Technique 2) for the invariants above.
5. **Cross-validate against references** — ERFA for the IAU 2006/2000A chain,
   Orekit/GMAT for propagation and body rotations (see the `orekit-fixtures`
   skill and the cross-validation tests under `arika/tests/`).
6. **Sanity-check non-trivial designs with `smart-friend`/Codex** before coding,
   and get an external review before merge.

The frame and time types currently live under `arika/src/frame.rs` and
`arika/src/epoch/`, with the precession/nutation chain under `arika/src/earth/`
— read the code as it is now before changing it.
