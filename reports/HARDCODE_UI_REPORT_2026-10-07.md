# Hardcode sweep, UI + identity + guard: report (2026-10-07)

Branch `wq/hardcode-ui`. Lanes L1, L11 (UI), L12.

## Done
- L1: on-chain identity is opt-in. Config keys `identity_chitin_slug` and `identity_chain` (empty by default) are read through `api.readConfig`. With neither set `hydrate()` makes no request and returns `linked: false`; the passport (`ForumPassport`, `HenryIdentityTab`) renders "No on-chain identity linked" with a button to Settings, Persona. The developer's DID, wallet, SBT/passport ids, avatar and chitin slug are gone from `services/identity.ts` and `config/chain.ts`. Persisted identity state is cleared when unlinked.
- Settings, Persona: new "On-chain identity (optional)" section (`OnChainIdentitySection.tsx`), validated, tokens only.
- `Hank` literals, `HankConversation`, `sitWithHank`, `HANK_TABLE_SEAT`, `.hank-*` CSS renamed to orchestrator names; fallback label is "Your orchestrator".
- CSP kept (the feature works when enabled); rationale in `ui/desktop/src-tauri/CSP_NOTES.md`.
- L11 UI: dev harness views (`cyborg-test`, `stargate-test`) only mount in dev builds; personal paths, names, hosts, project and place names in UI fixtures made neutral.
- L12: guard widened (roots, `.toml/.json/.html`, `skipFiles`, rule `exclude`), 8 new rules, `LEGACY_AGENT_NAMES = [Henry, Hank]`, Rust test `shipped_content_policy_holds`.

## Guard status
`node scripts/shipped-content-check.mjs` exits 1: 291 hits, ALL under `crates/**` (Rust lane). Zero in `ui/**`. Full rule file:line list: `reports/HARDCODE_GUARD_HITS_2026-10-07.txt`; per file counts, biggest: cost_router/fallback.rs 36, providers/ollama.rs 29, voice/proper_noun_corrector.rs 21, session_wing.rs 17, mesh/pool.rs 16, session/builder.rs 13, analytics_drain.rs 11.
Single-hit items for the Rust lane: telemetry key `posthog.rs:16`; `henry-malcolm` in `identity/canonical.rs`, `session_wing.rs:1093`, `activity/ingestion.rs:824`; regional merchants in `finance_statements.rs:241-253`.

## Decisions to review
- Rules `developer-places-projects` and `personal-hosts-ids` EXCLUDE `ui/desktop/src-tauri/tauri.conf.json` and `Cargo.toml`: they hold the publisher's signing identity and `authors`, which are release metadata, not user preferences. Remove the exclude if you want those parameterised.
- One exception (`terminal-reattach-label-example`, `illustrativeExample`, reviewBy 2027-01-31): a doc comment in `terminalReattach.ts`, whose bytes are hash-pinned by `terminalChrome.test.ts`; editing it would force a re-freeze.
- Rust test skips (with a stderr line) when `node` is not installed.
- `src/shippedContentCheck.test.ts` already runs this guard inside vitest, so the full vitest suite stays red until the Rust lane clears its hits.
