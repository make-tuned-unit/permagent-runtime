# Proposal: native architecture map view for "Map codebase"

**Status:** backlog idea — parked by Jesse 2026-09-16, not approved, not scheduled
**Author:** Archify integration assessment, 2026-09-16 (read-only sweep; nothing else in the repo changed)
**Related:** `docs/architecture/PRIME_RLM_AND_SKILLS.md`, `docs/benchmarks/HARNESS_BENCHMARKS.md`, `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`, #1118

## Constraints set by Jesse

No added runtime cost (tokens) and no added dependencies. That rules out vendoring Archify (a Node package) and any model authoring or refinement pass. Priority order: this sits behind #1118 (the ~17k-token preamble) while harness cost is the priority.

## The idea

When "Map codebase" is pressed (`ui/command-center/src/components/projects/CodeIndexPanel.tsx` → `POST /api/projects/{id}/index-code`, `crates/goose-server/src/routes/projects.rs`), additionally derive a deterministic architecture layer from data the tree-sitter analyzer already computes (`crates/goose/src/agents/platform_extensions/analyze/{repo_map,graph,parser}.rs`):

- ≤12 components from crates, packages and top-level dirs ranked by the existing PageRank; edges from aggregated imports and calls; boundaries from the top-level split; `sources[{path,line}]` pinned to the HEAD revision.
- Stored as typed JSON beside the existing Brain memory `code:{project_id}:map` (for example `code:{project_id}:arch`).
- Rendered natively as SVG in React in the project panel (reuse `graphPalette.ts`); grid layout; edge semantics in cards rather than edge labels; click a node to open its source at the pinned line; focus and upstream/downstream reach computed only from authored edges.
- Delta (added, removed, changed, moved per component, connection and boundary) computed in Rust on re-index. The post-landing refresh hook (`refresh_code_map_after_landing`) already fires. No architecture-drift check exists anywhere in CI today.
- Keep the JSON a compatible subset of Archify's `architecture.schema.json` (MIT) so Archify can optionally be run on it externally. A bridge, never a dependency.

## Benefit, ranked

1. The Projects surface becomes legible for the ~45 repos under the dev directory.
2. Structural change is visible after every landed goal at zero tokens.
3. Grounded "how does X reach Y" answers from authored, revision-pinned edges.
4. Permagent's own architecture docs (all 3 weeks to 5 months stale; no `ARCHITECTURE.md`) stop rotting.
5. Maybe a 1–2k-token coordinator orientation block. Hypothesis only: the 2026-08-10 A/B showed injected maps barely change worker navigation. Must be A/B'd in `permagent-eval`.

Not a token saver, not better retrieval, not a replacement for the tree-sitter repo map.

## Appendix: the Archify assessment this rests on

Source: tt-a1i/archify @ 72c750b (v2.17.0-dev.1, MIT, 64.6k stars, 119 tests, last stable 2.16.0 on 2026-08-30). Cloned to scratchpad; CLI exercised locally (doctor/validate/deliver/compare/inspect/guide all pass on node 24).

## 1. What Archify actually is
- An agent SKILL (SKILL.md + Node CLI). The agent authors typed JSON IR (components, boundaries, connections, cards, optional git-pinned `sources[]`), and `archify.mjs validate|deliver` compiles it deterministically into one self-contained HTML (viewer inlined, ~810 KB). Validation returns machine-readable diagnostics with `supportedFixes`; deliver writes SHA-256 receipts. `compare` diffs two IR snapshots (added/removed/changed/moved/rerouted + boundary/evidence changes) with a JSON receipt.
- Hand-placed layout by the model is the product. Auto-layout, general graph IR, Mermaid parsing, and diff-UI-in-template are explicitly declined in ROADMAP "Not planned". Showcase profile targets ≤12 primary nodes.
- Source evidence: `meta.repository.{url,revision}` + `components[].sources[{path,line,end_line}]`, verified locally via `git cat-file` at the pinned SHA against `--repo-root`; requires an `origin` remote matching the URL (`link_mode: local-only` for private repos). No remote requests.
- Network: optional update check (disable with ARCHIFY_UPDATE_CHECK_DISABLED=1) and `brands capture <url>` fetches. Nothing else.
- Cost of using it: SKILL.md ≈4.1k tokens + one schema ≈1.7k + common schema ≈0.8k + one example ≈1k ⇒ ≈7.5k tokens before authoring; authoring-contract adds ≈3.5k when repairs are needed. A real-repo map (mco, 13 components/12 connections/13 source refs) is ≈2.2k tokens of IR. validate/deliver run in ~0.15 s.
- Model floor (their own benchmark, 2026-07-26): MiniMax/DeepSeek/Qwen 8/15 first-pass usable; failures cluster in dataflow/lifecycle routing. Architecture type is the easiest.

## 2. What Permagent already has (from the sweep)
- Tree-sitter analyzer + call graph + Aider-port ranked repo map (PageRank, token-budgeted, DEFAULT_CODING_REPO_MAP_TOKENS=4096) injected into coding-harness sessions (builder.rs:1000). Stale header comment in repo_map.rs:31 says it is not wired — it is.
- Persisted per-project Brain memory `code:{project_id}:map` (tree + symbols text), goal-aware slice CODE_MAP_INJECT_MAX_CHARS=4000, `map_query` tool, refresh after landing. No component model, no boundaries, no git revision, no file:line in the stored map, no structural diff.
- Token reality (docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md, docs/benchmarks/POLYGLOT_PUBLIC.md): input:output ≈69:1; ~24.6k-token fixed preamble re-billed per call; ~17k-token capability inventory = 76% of the prompt (#1118). Code-map A/B (2026-08-10): −18.9% billed input on one lookup task, tool calls unchanged, follow-up cell retracted as contaminated.
- Render surfaces: Tauri CSP `script-src 'self' 'unsafe-inline'`, frame-src 'self' http: https:; DocumentViewer has no HTML branch (PDF iframe sandbox=""); in-app browser refuses file:/data:. autovisualiser already emits self-contained HTML (mermaid etc.) that NO UI renders; mcp_ui_proxy templates exist unwired.
- Node: bundled mcp-runtime/node v22.22.1 in the shipped .app; `node` is NOT in the default command allowlist (AgentTrust tier); npm/npx are. No subprocess egress policy.
- Skills: SKILL.md discovery in ~/.permagent/skills, ~/.claude/skills, .agents/skills etc.; `load_skill` exposes body + supporting files. `run_executable_skill` has no approval gate (documented hole, PRIME_RLM_AND_SKILLS.md:163). No installer/marketplace client.
- Roster: knowledge.rs snapshot 2026-07-15; no strict-JSON conformance measurement; qwen38 split context_limit 4096 (< skill load); Apple FM cannot take schemas; no MiniCPM row.
- Docs: no ARCHITECTURE.md; docs/architecture/* 3 weeks–5 months stale; one mermaid diagram in the whole repo.

## 3. Verdicts
Token usage: Archify is token-NEGATIVE per invocation and does not touch the lever that matters (preamble, #1118). The only plausible token-positive use is a cached ≈2k-token architecture IR as an orientation block, and the 2026-08-10 A/B says injected maps barely change navigation. Treat as hypothesis to A/B, not a savings plan.
Codebase mapping: Archify is not an indexer and is strictly weaker than the tree-sitter map at symbol level. It adds the layer Permagent lacks: component/boundary model, git-pinned evidence, deterministic validation, snapshot delta, and a shareable render. Value is comprehension + review receipts, not retrieval.

## 4. Integration points (ranked)
A. Zero-code trial: drop the packaged skill in ~/.permagent/skills; ask a cloud coordinator to map Permagent itself with evidence (local-only). Measures real token cost, yields the first ARCHITECTURE artifact. Needs `node <skill>/bin/archify.mjs` allowlisted or wrapped as an npm-script (npx is allowlisted) and ARCHIFY_UPDATE_CHECK_DISABLED=1 in the env seam (extension_manager.rs merge_environments).
B. Artifact viewer (shared with autovisualiser — one concept, one place): serve project artifacts from permagentd and render in a sandboxed iframe (allow-scripts, no allow-same-origin); frame-src already permits http localhost. Unblocks both Archify and the existing unrendered HTML tools.
C. Brain `code:{project_id}:arch` beside `:map`; refresh via the existing refresh_code_map_after_landing hook; expose to workers via `map_query`-style tool rather than injection (per the A/B finding).
D. Architecture Delta as a review receipt: `compare --json` between pre-goal and post-goal IR attached to goal `metadata_json.verification` for goals touching >1 crate/module. Cross-family reviewer gets a structural diff instead of raw diff. Costs tokens; gate to large goals only.
E. Self-documentation: replace stale docs/architecture prose with an evidence-pinned map + delta per release.
Do NOT: use Archify as a repo indexer or wire tree-sitter → IR auto-layout (declined upstream, 12-node cap); route IR authoring to local models (ctx 4096, 8/15 floor); install via run_executable_skill until skill_run gate exists.

## 5. Risks / gates
- Unguarded run_executable_skill path — use load_skill + shell allowlist instead.
- Update-check + brand-capture egress; no subprocess network policy exists — env var + forbid `brands capture`.
- Evidence requires origin remote; worktrees fine, remote-less projects fall back to source-free maps.
- 810 KB per artifact; dev-line version — pin to 2.16.0 stable; single maintainer.
- Skill overhead ≈7.5–11k tokens per authoring run; not per-goal.

## 6. Recommended sequence
1. Trial A (1 session, no code). 2. Viewer B (small daemon route + iframe). 3. C then D behind permagent-eval A/B with cost_ledger deltas. Token-reduction work stays on #1118.
