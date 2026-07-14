# Build brief — Librarian consolidation atoms (Spectral layered store)

You are building the Permagent (Librarian) side of an integration whose Spectral
side already shipped. The spectral pin is ALREADY bumped (Cargo.toml → rev
108a12e) and Cargo.lock updated — do not touch the pin. This machine is an Intel
Mac that CANNOT build the Rust workspace; CI is the gate. Author carefully, run
`cargo fmt --all` (rustfmt works locally at ~/.cargo/bin/cargo), and rely on CI
for compile/clippy. Work ONLY in this worktree (~/dev/permagent-worktrees/librarian-atoms).

## Why (the load-bearing evidence — do not violate)
On the real eval, retrieval recall is near-ceiling; the gap is the ACTOR
mis-counting the same real-world item across sessions. A cheap READ-TIME LLM
consolidation pre-pass REGRESSED −9.2pp (a weak, lossy intermediate the strong
actor over-trusts). So consolidation MUST be: (a) write-time, (b) strong-model,
(c) durable, (d) gated to recurring clusters, and (e) consumed as a HINT the
actor verifies against raw sources — never authoritative. Getting atom quality
wrong reintroduces the −9pp regression. This is the whole ballgame.

## Spectral APIs (already shipped at the pinned rev; on `spectral::Brain`)
- `consolidation_candidates(min_co_count: u64, scan_limit: usize) -> Result<Vec<ConsolidationCandidate>>`
  - `ConsolidationCandidate { member_keys: Vec<String>, cohesion: f64, signal: &'static str }` (member_keys always ≥2)
- `get_memory(id: &str) -> Result<Option<spectral::ingest::Memory>>` — key→id is `blake3(key)[..8]` hex. Prefer the existing `SafeBrain::get_memory_by_key(key)` which already does the hashing (brain_handle.rs:198).
- `consolidate_as(source_keys: &[String], target_key: &str, tier: spectral_ingest::CompactionTier, content: &str) -> Result<RememberResult>` — stores the pre-computed atom + provenance edges.
- `consolidate_extractive(source_keys, target_key, tier)` — deterministic $0 fallback (longest source). Use when the LLM/provider is unavailable so layered recall still works.
- `recall_with_provenance(query: &str, config: &RecallTopKConfig, visibility: Visibility, max_sources_per_hit: usize) -> Result<Vec<LayeredHit>>`
  - `LayeredHit { hit: MemoryHit, sources: Vec<MemoryHit> }`; `MemoryHit { id, key, content, signal_score, visibility, source, ... }`
- `CompactionTier { Raw, HourlyRollup, DailyRollup, WeeklyRollup }` — use `WeeklyRollup` for cross-session entity atoms (highest abstraction).

## Permagent access pattern (MANDATORY — enforced by SafeBrain)
- `crate::agents::platform_extensions::get_global_brain() -> Option<SafeBrain>`.
- `SafeBrain` (brain_handle.rs) wraps `Arc<spectral::Brain>`. It ALREADY has async wrappers (`recall`, `get_memory`, `get_memory_by_key`, `consolidate_into`) that call the sync spectral Brain inside `tokio::task::spawn_blocking` via `raw_blocking_handle()`. 
- ADD three async wrapper methods to `SafeBrain`, mirroring the existing ones EXACTLY (clone the Arc, `spawn_blocking(move || brain.raw_blocking_handle().<fn>(...))`, map join errors):
  - `pub async fn consolidation_candidates(&self, min_co_count: u64, scan_limit: usize) -> anyhow::Result<Vec<spectral::...::ConsolidationCandidate>>`
  - `pub async fn consolidate_as(&self, source_keys: Vec<String>, target_key: String, tier: CompactionTier, content: String) -> anyhow::Result<...>`
  - `pub async fn recall_with_provenance(&self, query: String, visibility: Visibility, max_sources: usize) -> anyhow::Result<Vec<LayeredHit>>` (build a default `RecallTopKConfig` inside, matching how `recall`/`recall_cascade` construct configs).
  - Verify the exact type paths (`spectral::Brain` re-exports vs `spectral_graph::brain::`) against the pinned crate source under `~/.cargo/git/checkouts/spectral-*/108a12ef/` — do NOT guess; grep the actual re-exports in `crates/spectral/src/lib.rs`.

## Strong-model provider (the atom generator's LLM)
Mirror `crates/goose-server/src/proactive.rs` `resolve_provider()` (~line 255):
`permagent::providers::create_with_named_model(&provider_name, &model_name, Vec::new()) -> Option<Arc<dyn Provider>>`. But use the ACTOR/lead model tier, not the fast model — the quality contract demands actor-tier or better. Call `provider.complete(...)` (not `complete_fast`). Read the lead provider/model from config the same way proactive.rs reads its provider/model. If no provider resolves → fall back to `consolidate_extractive` so layered recall still exists at $0 (log it).

## What to build

### 1. Atom generator (write-side) — new module `crates/goose/src/agents/platform_extensions/librarian_atoms.rs`
Async fn, e.g. `pub async fn run_atom_consolidation(brain: &SafeBrain, max_atoms: usize, provider: Option<Arc<dyn Provider>>) -> anyhow::Result<usize>`:
1. `brain.consolidation_candidates(min_co_count, scan_limit)` — pick recurring clusters (start min_co_count=2, scan_limit reasonable e.g. 200; cap to `max_atoms`).
2. For each cluster: fetch member contents via `get_memory_by_key` for each `member_key` (skip clusters where members can't be fetched).
3. Build the strong-model prompt encoding the QUALITY CONTRACT verbatim (below). ONE call per cluster → one atom.
4. Parse the atom text (plain text lines; keep it robust — if the model returns prose, store as-is; the contract asks the atom to name its source keys inline).
5. `target_key` = a stable derived key, e.g. `atom:{blake3-or-hash-of-sorted-member-keys}` (deterministic so re-runs are idempotent — if the target already exists, skip). `brain.consolidate_as(member_keys, target_key, CompactionTier::WeeklyRollup, atom)`.
6. Best-effort per cluster: a failure on one cluster logs + continues; never abort the sweep.
Return the count of atoms written.

### THE ATOM QUALITY CONTRACT (encode this into the system prompt VERBATIM in spirit):
1. Entity-keyed, dedup-correct: one line per distinct real-world item, keyed by its most distinctive identifier; MERGE cross-session mentions of the SAME item; never split one item across sessions.
2. Inclusion-strict: only items the user actually did/attended/owns — exclude hypotheticals, planned-not-done, assistant suggestions.
3. Cite sources: each line references the source session/turn keys it came from (name them in the atom text).
4. Omission-safe: if unsure two mentions are the same item, SAY SO in the atom rather than silently merging/dropping — the actor has the raw sources to adjudicate.
5. Strong model, entity-keyed, additive — the atom is a CANDIDATE SET, not an authoritative replacement.

### 2. Scheduling — piggyback the existing warm sweep
In `crates/goose-server/src/routes/librarian/scheduling.rs` `warm_and_run()`, AFTER the existing describe/annotate/entity passes (`run_batch` + `describe_entities_batch`), add an atom-consolidation stage that calls `librarian_atoms::run_atom_consolidation(...)` with a per-sweep cap (e.g. 10 atoms). GATE THE ENTIRE STAGE behind a config flag defaulting to OFF (e.g. `LIBRARIAN_ATOMS_ENABLED`, read via `Config::global()`), because default-ON is gated on the mini's paired eval (Arm B > Arm A). Log atoms written. Do NOT add a separate scheduler.

### 3. Read-side — actor consumes atoms as HINTS
Find where the actor's memory recall is assembled for its context (grep `search_memory`, `MemoryRecallable`, the memory-recall tool/formatter the agent uses). Behind the SAME flag, add a path that uses `brain.recall_with_provenance(...)` and formats each `LayeredHit` as: the atom, then its raw source turns, with framing text stating: "The CONSOLIDATED atoms are a candidate set to VERIFY, not ground truth; confirm each against the raw sessions before counting; add items the atoms missed." This framing is the load-bearing difference from the regressed read-time version — get it exactly right. If this integration point is large/unclear, STOP and report it as a follow-up rather than guessing — the write-side + wrappers are the priority.

### 4. Self-knowledge
Update the Librarian worker descriptor (crates/goose/src/agents/platform_extensions/librarian.rs — it's a Worker, Queryable, in KNOWN_WORKER_IDS) so Henry can describe that the Librarian now writes durable cross-session consolidation atoms. This WILL change the 4 `permagent__*.snap` prompt-manager snapshots — hand-edit all 4 to the new rendered text (the render is `**Name** — {what_it_does}. {why_it_matters}`; match it precisely). Keep the descriptor change minimal (one clause) to keep the snapshot edit tractable. Do NOT trip `workers_are_queryable_surfaces_are_static` / `every_known_worker_has_a_descriptor`.

## Gates
- `cargo fmt --all` (run it — a fmt miss cost a prior PR a CI round).
- No local Rust build possible; CI runs `--workspace --all-targets`. Self-review every new `use`, the exact spectral type paths (grep the pinned checkout), the SafeBrain wrapper signatures, and the provider call.
- Add a focused unit test where cheap (e.g. deterministic target-key derivation is idempotent).
- Everything additive + flag-gated default-OFF: this PR must NOT change actor behavior until the mini eval flips the flag.

## Report back
1. Files created/changed + the SafeBrain wrappers (exact signatures) + the flag name.
2. The atom system prompt you authored (paste it) — this is the crux; I will review it against the quality contract.
3. Whether you completed the read-side or stopped and flagged it.
4. Self-knowledge descriptor + whether you hand-edited the 4 snapshots.
5. `cargo fmt` output; any spectral type-path uncertainty CI must confirm.
6. Explicit note that #4 (5–10 sample atoms on real data) + the paired Arm A/B eval must run on the mac mini — this PR ships the flagged mechanism, not the eval result.
