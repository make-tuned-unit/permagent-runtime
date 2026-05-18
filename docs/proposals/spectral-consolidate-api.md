# Brain.consolidate_into() API for memory-graph consolidation

> Filed as [Spectral #131](https://github.com/make-tuned-unit/spectral/issues/131).
> This file is a local copy for reference.

## Summary

Permagent's Librarian performs consolidation -- detecting redundant memory clusters and merging them into representative summaries. Today the merge is recorded by a Permagent-side `ALTER TABLE memories ADD COLUMN _pm_consolidated_into TEXT`, with direct UPDATE writes from `ollama.rs` (now `librarian/consolidation.rs` after the in-flight module split).

This bypasses the Spectral abstraction layer. A Spectral pin bump that adds its own schema migration or constraint to the `memories` table could silently break the Permagent write path. There's no Spectral-side awareness that some memories are consolidation targets vs. sources, so recall surfaces both unless Permagent filters client-side.

## Proposed API

```rust
trait Brain {
    fn consolidate_into(
        &self,
        source_keys: &[String],
        target_key: String,
        opts: ConsolidateOpts,
    ) -> Result<ConsolidationResult>;

    fn list_consolidated(
        &self,
        target_key: Option<&str>,
    ) -> Result<Vec<ConsolidationEdge>>;
}

pub struct ConsolidateOpts {
    pub exclude_sources_from_recall: bool,  // default true
    pub preserve_signal_scores: bool,       // default true
}
```

## Semantics

- `source_keys` marked as consolidated into `target_key`
- If `exclude_sources_from_recall`, `recall()` omits source memories
- If `preserve_signal_scores`, `target_key` inherits summed signal_score from sources
- Consolidation chain preserved (A->B->C queryable)
- Idempotent: re-consolidating same source->target is a no-op

## Migration path

Once this lands, Permagent migrates:
1. Bump Spectral pin past the consolidate_into commit
2. Replace direct SQL in librarian/consolidation.rs with trait calls
3. Remove the Permagent-side `_pm_consolidated_into` column (drop or leave as no-op alias for one release)

Spectral owns the column, the migration, the constraint.

## Open questions

- Should `recall()` default to filtering consolidated sources, or always opt-in via `RecallOpts`?
- Behavior when `target_key` itself gets consolidated later (chain traversal vs. flatten on write)?
- Should `consolidate_into` accept a freshly-written summary memory (target_key doesn't exist yet) or require target to exist first?

## Context

This is part of Permagent's pre-Phase-2 hardening sequence. Related: the ambient-context-block extraction issue (in permagent-runtime), the auth-as-middleware work (Phase 1.5 gate), and the `librarian/` module split (in flight).
