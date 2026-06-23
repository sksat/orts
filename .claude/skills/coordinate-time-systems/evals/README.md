# Evals for the coordinate-time skills

Configuration only — no results are checked in (they go stale). This records
*how* to evaluate the companion pair `coordinate-time-systems` and
`coordinate-time-implementation` so the checks are reproducible.

## Files

- `evals.json` — behavioural evals: three realistic tasks, each with
  `expectations` (objective statements a good answer must satisfy). They span
  both skills (a TEME→GCRS conversion, a GCRS→ITRS time-scale signature, adding
  GPS time).
- `trigger-eval.json` — description-triggering queries: `should_trigger=true`
  for substantive, skill-worthy tasks (implement / redesign / reconcile /
  **verify**); `should_trigger=false` for near-misses in adjacent domains.

## Running the behavioural evals

For each prompt, run an agent twice and grade its answer against `expectations`:

- **with-skill:** give the agent both `SKILL.md` files and tell it to apply them.
- **baseline:** same prompt, no skills.

The most informative variant **denies repository access** (let the agent only
write its answer, not read `arika/`). With repo access the baseline recovers the
answer from arika's own docs/code, so the comparison is uninformative; without
it you measure what the skill adds over the model's own knowledge.

## Running the trigger eval

The skill-creator's `run_eval.py` drives `claude -p` over `trigger-eval.json`.
Note: in some headless `claude` versions the trigger-detection harness reports
zero triggers for *every* query (a mechanism/version mismatch, not a description
problem), so treat its absolute numbers with care and fall back to judging the
descriptions against these queries by hand.

## What we learned (keep in mind when editing)

- A capable model already answers the textbook mechanics (per-step time scales,
  `GPS = TAI − 19 s`, the TAI hub, phantom-typed epochs) unaided — so the skill's
  value, and these evals' signal, live in the **non-obvious** parts: typing the
  *frame* of a vector (not just the scale), `UT1` as the sidereal independent
  variable, representation-vs-frame singularities, covariance/interpolation.
- The skills also serve as a **verification checklist** for work the model could
  write unaided — hence the `verify`/`review` trigger queries.
