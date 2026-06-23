---
name: coordinate-time-typing
description: >-
  Express coordinate reference frames and time scales in the type system, and
  test them with property-based tests — durable design guidance for orts/arika
  that holds whether you are extending the current types or redesigning them.
  Use when implementing, refactoring, or rethinking frame/scale code: adding a
  frame, scale, or transform; choosing typed API signatures (frame-tagged
  vectors, scale-tagged epochs, typed rotations); deciding which time scale a
  transform must take; isolating EOP dependencies; or writing tests for rotation
  and scale-conversion invariants. Companion to `coordinate-time-systems`, which
  covers the domain concerns (which frame/scale, which realization, the traps).
---

# Typing & testing coordinate frames and time scales

The domain concerns — which time scale a transform requires, which system /
realization / scale a name actually means, and the traps that bite — live in the
companion skill **`coordinate-time-systems`**. This skill is the *how*: two
durable techniques that turn those concerns into code that cannot silently go
wrong. They apply equally when you extend the existing types and when you
redesign them, so treat what follows as design principles, not a description of
the current API.

## Technique 1: express frame and scale in the type system

The single best defense against every domain concern: **a frame or a time scale
is a type parameter, not a convention you have to remember.** The wrong
combination then fails to compile instead of silently producing the wrong
physics. Three moves carry most of the value:

- **Tag vectors with their frame.** A position/velocity vector is parameterized
  by a frame marker, mixing two frames is a compile error, and you cross between
  frames *only* through a typed rotation. (In orts this is `Vec3<F>` crossed by
  `Rotation<From, To>` — but the principle is what matters when you change it.)
- **Tag instants with their scale.** An epoch is parameterized by a time-scale
  marker, and subtracting two different scales does not compile — you convert
  explicitly first. (In orts, `Epoch<S>`.)
- **Encode a transform's required scale in its signature.** Because the scale is
  usually the transform's independent variable, make the signature demand the
  right one: the precession/nutation step takes a TT epoch, the Earth-rotation
  step takes a UT1 epoch, an ephemeris call takes a TDB epoch. A method like
  `era()` should exist only on the UT1 epoch, so `tdb_epoch.era()` cannot be
  written.

The bar, whatever the shape of the types: **make the physically-meaningful
operations expressible and the meaningless ones fail to compile.** Sharper rules
that follow — and that are most useful precisely when you are *changing* the
design:

- **Don't launder a bare numeric vector / JD across a boundary** where its frame
  or scale is ambiguous. Raw-access escape hatches are sometimes necessary —
  keep them local and name them so the unchecked nature is obvious (e.g. a
  `_raw_unchecked` suffix), never on a public path.
- **Type the invariant, not the vibe.** Add a type when a value carries a real,
  easily-confused invariant or semantic (a frame; a scale; a *continuous* week
  count vs a 10-bit *broadcast* GPS week). Don't add trait bounds for
  philosophical taxonomy (proper-time vs coordinate-time) that change nothing
  about what compiles — that just misleads about precision.
- **No silent upgrade between precision tiers.** A visualization-grade (e.g.
  rotation-only) value must not silently masquerade as a rigorous one. Don't
  provide approximate→rigorous conversions; let a naming convention (e.g. a
  `Simple` prefix) carry the precision warning in the type.
- **A blanket `impl<F: Frame>` over a precision-aware operation is a trap.** It
  lets an approximation written for one frame propagate, with no compile error,
  to a future frame where it is wrong. Prefer explicit per-frame impls so that
  adding a frame *forces* you to write — or deliberately refuse — its handling.
  This is what keeps the type safety honest as the set of frames grows.

## Technique 2: property-based testing for the invariants

Frames and time scales are *full* of algebraic invariants, and generated inputs
catch the worst-case float that hand-picked cases miss. (The technique is
*property-based testing*; in Rust the usual crate is `proptest`.) Assert with an
explicit tolerance — never `==` on a transformed `f64`:

- **Rotation round-trip:** `inverse(r) ∘ r` applied to `v` ≈ `v`.
- **Magnitude preservation:** a rotation is an isometry — `|r·v| ≈ |v|`.
- **Composition / associativity & identity:** `(a∘b)∘c ≈ a∘(b∘c)`;
  `r ∘ inverse(r) ≈ identity`.
- **Scale round-trip:** `utc → tai → utc ≈ utc`; chained `tt → tdb` and back
  within the model's stated tolerance.
- **Duration across discontinuities:** `(epoch + d) − epoch ≈ d` *even across a
  leap-second boundary* — a hand-written test rarely lands on one; a generator
  will, and shrinking hands you the minimal failing case.
- **Generators must include the nasty inputs:** near-zero and parallel position/
  velocity (local-orbital-frame degeneracy), huge magnitudes, sub-µs time deltas,
  and `NaN`/`±∞` where the function claims total behavior. A float-predicate
  rewrite can agree on finite values yet diverge at `NaN`.

Frame *mixing* itself is enforced at compile time, not as a runtime property —
that is Technique 1 doing the property test's job for free.

## Designing the conversions

The two techniques above shape how to design the conversion surface itself.
Durable patterns (worth applying when you redesign, not just extend):

- **Model time scales by how they bridge to a hub.** Pick one continuous hub
  (TAI is the natural choice). Scales that differ from it by a constant share one
  small capability and route through a single pivot, so adding such a scale is
  "one constant + one impl", nothing more. Scales that need *data* declare it in
  their type: a leap-second table (UTC), a periodic series (TDB). And a scale
  that needs a *measurement* — UT1, via an EOP/ΔUT1 provider — must stay **off**
  the data-free pivot, so the type makes the dependency unavoidable (there is no
  argument-free `UT1 → TAI`). This keeps "which conversions need EOP vs a table
  vs a constant" visible in the signatures.
- **Make precision-aware frame rotations frame-explicit and scale-correct.** A
  rotation that depends on the frame's orientation must name its concrete frames
  (no blanket impl) and take the correct independent-variable scale. TEME is the
  worked example: a `TEME→GCRS` rotation is GMST + equation of the equinoxes
  (plus precession/nutation), so it must be a real, frame-explicit conversion
  taking the right scale — never a relabel — and it should be cross-validated
  against a reference implementation.

## Workflow: changing coordinate/time code

1. **Anchor on the standard first.** Resolve the domain concerns (companion skill
   `coordinate-time-systems`) against the primary sources before writing — the
   definition, the independent-variable scale, the data dependency, the
   conventions, the error budget.
2. **Keep it expressible-or-uncompilable** (Technique 1): add the marker/scale;
   encode the required scale in transform signatures; refuse silent precision
   upgrades and blanket precision-aware impls.
3. **Pin behavior before refactoring** (project rule): characterization tests
   including boundary *and* non-finite inputs (`NaN`, `±∞`); pin bit-for-bit
   where behavior must be preserved.
4. **Add property-based tests** (Technique 2) for the round-trip / isometry /
   composition / leap-second invariants, with degenerate and non-finite
   generators.
5. **Cross-validate against references.** ERFA for the IAU 2006/2000A chain,
   Orekit/GMAT for propagation and body rotations — see the `orekit-fixtures`
   skill and the cross-validation tests under `arika/tests/`.
6. **Sanity-check non-trivial designs with `smart-friend`/Codex** before coding,
   and get an external review before merge (project methodology).

The frame and time types currently live under `arika/src/frame.rs` (frame-tagged
vectors, typed rotations, frame markers) and `arika/src/epoch/` (scale-tagged
epochs, scale markers, conversions, leap seconds), with the precession/nutation
chain under `arika/src/earth/` — read the code as it is now before changing it.
