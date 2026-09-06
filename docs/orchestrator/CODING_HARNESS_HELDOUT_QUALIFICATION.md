# Held-out qualification contract

`permagent-eval::qualification` is the no-provider, deterministic promotion
check for E7. It consumes retained JSON evidence from the evaluator and emits
a derived scorecard; callers cannot promote an area by supplying an
`Excellent` label.

The validator enforces:

- a non-empty, versioned held-out set;
- unique optimizer IDs, held-out task IDs, and run IDs, so duplicated records
  cannot inflate either the benchmark size or the three-run streak;
- no overlap between optimizer and held-out task IDs;
- complete area evidence for every retained run;
- three consecutive passing runs at the end of the retained history;
- no unresolved P1 evidence; and
- a passing held-out gate for overall promotion.

An area with missing evidence or fewer than three runs is `Unrated`. A failed
gate is `Poor`; partial evidence is not silently converted into a pass. The
three-run streak is trailing: a failure resets it, so an earlier green run
cannot mask a later regression.

This is qualification machinery, not a benchmark runner. Provider calls,
dataset checkout, artifact storage, and raw trace retention remain explicit
outer DAG nodes. A release must retain the input evidence and the derived
report so the result can be independently reproduced.

Example use from Rust:

```rust
let report = permagent_eval::qualify(&evidence)?;
assert_eq!(report.overall, permagent_eval::AreaRating::Excellent);
```

The same contract is reachable without a provider through the evaluator CLI.
An outer run/retention job writes the `QualificationInput` JSON (the shape is
the Rust type above), then the deterministic command validates it and emits
only the derived report:

```bash
cargo run -p permagent-eval -- qualify \
  --input retained-qualification.json \
  --out qualification-scorecard.json
```

Use `--input -` to read JSON from stdin. The output is machine-readable JSON;
an invalid benchmark version, overlapping optimizer/held-out IDs, or malformed
run evidence exits non-zero. No provider, model, network, or benchmark fetch
is involved. The command does not accept a caller-supplied rating, so missing
evidence remains `Unrated` and a one-run smoke test cannot become `Excellent`.

The current repository scorecard remains `Hold`: it has not supplied the
required held-out evidence, and the retained local smoke is not a qualification
sample.
