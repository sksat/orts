# Evals for coordinate-time-implementation

Two layers, config only (no results checked in — they go stale):

## Implementation-specific (`evals.json`, here)

Cases where *this* skill (typing discipline + property-based testing), not the
companion concerns skill, is the primary driver:

- **`review-typed-api`** — a verification task: review a draft Rust frame/time
  API for type-safety (bare `Vector3` instead of `Vec3<F>`, a blanket
  `impl<F: Eci>` over a precision-aware op, an EOP-free `Utc→Ut1`, a silent
  `SimpleEci→Gcrs` upgrade, `era()` on the wrong scale).
- **`proptest-rotation-scale`** — write a property-based test plan for a
  `Rotation<From,To>` and `Epoch<S>`: the invariants and the generators
  (degenerate + non-finite) it must cover.

## Shared with the companion skill

The cross-skill behavioural cases (a realistic task needs both the concerns and
the implementation) and the description-triggering queries live once under the
companion:

- [`../../coordinate-time-systems/evals/`](../../coordinate-time-systems/evals/)
  — `evals.json`, `trigger-eval.json`, `README.md`.

Run any of these with-skill vs baseline, ideally repository-denied (so the
baseline can't recover the answer from arika's own code); grade against each
case's `expectations`. See the companion README for the method and findings.
