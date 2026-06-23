# Evals — shared with the companion skill

`coordinate-time-implementation` and `coordinate-time-systems` are a companion
pair and are evaluated **together** (a realistic task needs both: the domain
concerns *and* how to encode them). The shared eval config therefore lives in
one place, under the companion skill:

- [`../../coordinate-time-systems/evals/`](../../coordinate-time-systems/evals/)
  — `evals.json` (behavioural cases), `trigger-eval.json` (triggering queries),
  and `README.md` (how to run them).

The three behavioural cases exercise *this* skill directly — typing a TEME→GCRS
conversion, designing a GCRS→ITRS signature that takes the right scale-tagged
epochs, and modelling GPS time via the TAI hub with property-based tests — so
its `skills_under_test` lists both skills. Run them as described in that README
(with-skill vs baseline, ideally repository-denied).
