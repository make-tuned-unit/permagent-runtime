# FCC harness UX DAG

**Branch:** `feat/harness-cost-failover-ux` · **Date:** 2026-08-26
**Source review:** Free Claude Code (MIT, HEAD 6b3f16f) vs Permagent harness.

Do not adopt FCC as a runtime. Each node below is a Permagent seam. Telegram
`/stop` and an `ANTHROPIC_BASE_URL` worker gateway are skipped (iOS remote;
Claude/Codex already launch from Projects).

Verification lenses on every node: **completeness**, **bugs**, **wiring**,
**regressions**. A node is done only when all four CHECK lines pass.

```
N0 spec
 ├─ N1 ordered fallbacks ──────┐
 │                              ├─ N2 silent pre-commit failover
 ├─ N3 local session titles     │
 └─ N4 packs HTTP API ──────────┼─ N5 surface Apply routing (chat / Models / Build)
                                ├─ N6 searchable picker + validate-without-save
                                ├─ N7 teachable cost_optimizer
                                └─ N8 subscription-first on Projects
```

## N0 — spec (this file)

- Completeness: ranked moves from the FCC review are nodes, not a pile of todos.
- Bugs: worker gateway and Telegram are listed as skipped Later, not dropped.
- Wiring: each node names the file it lands in.
- Regression: no change to the “no vendor default” role-map rule.

CHECK: `rg -n "^## N" docs/orchestrator/fcc-harness-ux-dag.md`
EXPECT: N0 through N8 (and Later) are present.

## N1 — ordered fallback chain

`crates/goose/src/cost_router/fallback.rs`

`PERMAGENT_FALLBACKS` is `provider/model,provider/model`. Explicit
`PERMAGENT_FALLBACK_PROVIDER`/`_MODEL` still wins as the first hop.
Permanent (billing/auth) skips the same provider; pre-commit may use any
other pair.

- Completeness: parse, skip failed pair, skip same-provider on permanent.
- Bugs: half-set override still counts as unset.
- Wiring: `permanent_failure_fallback` consults the list.
- Regression: existing fallback tests still pass.

CHECK: `cargo test -p permagent --lib cost_router::fallback`
EXPECT: exit 0; `ordered_list_is_consulted_after_explicit_override` exists.

## N2 — silent pre-commit failover

`crates/goose/src/agents/agent.rs` + `may_silent_precommit_failover`

If the stream dies with a transient error **before any assistant response was
committed**, switch to the next fallback with no user-visible error and
retry the turn. Once tokens/tools have been shown, do not silent-switch.
CreditsExhausted / Authentication still announce.

- Completeness: NetworkError, ServerError, RateLimitExceeded.
- Bugs: committed streams never silent-switch; no raw error in the reply.
- Wiring: uses N1’s chain; `did_switch_provider_this_iteration` already retries.
- Regression: credits-exhausted announced path unchanged.

CHECK: `cargo test -p permagent --lib cost_router::fallback::tests::silent_precommit`
EXPECT: uncommitted+network → true; committed+network → false; credits → false.

## N3 — local session titles

`crates/goose/src/providers/base.rs`

Derive the session name from the first user lines. Call `complete_fast` only
when that yields nothing.

- Completeness: empty user text still falls through to the model.
- Bugs: titles stay ≤100 chars; XML stripped.
- Wiring: `generate_session_name` calls `local_session_title` first.
- Regression: `test_extract_short_title` still passes.

CHECK: `cargo test -p permagent --lib providers::base::tests::local_session_title`
EXPECT: “fix the login bug please and also” → a short local title, no provider.

## N4 — packs HTTP API

`crates/goose-server/src/routes/packs.rs`

`GET /api/packs` returns the recommendation, the configured map, and
`prompt: bool`. `POST /api/packs/apply` persists `mappings_to_persist`.

- Completeness: same triples as `permagent packs apply`.
- Bugs: empty considered → nothing written, `prompt: false`.
- Wiring: merged in `routes/mod.rs`; OpenAPI path listed.
- Regression: CLI `packs apply` still the source of truth for persistence.

CHECK: `cargo test -p permagent --lib cost_router::role_map::tests::should_prompt`
EXPECT: `should_prompt_when_unconfigured_and_two_distinct_models` passes.
(`packs.rs` wraps the same helper. `cargo test -p permagent-daemon packs` also
links every integration-test binary ~500MB each and ENOSPCs/OOMs on this machine.)

## N5 — surface Apply routing

The user must not have to know `permagent packs apply` exists.

- Chat composer (`RoleRoutingPrompt`) when `prompt` is true.
- Settings → Models as the first section, with Apply.
- Build `CostStatusline` compact Apply when `prompt` is true.

- Completeness: three surfaces, one API.
- Bugs: dismiss is local; Apply records `cost_optimizer` usage.
- Wiring: ModelsPanel test allowlist includes `getPacks` / `applyPacks`.
- Regression: SkillPromptBanner and cost meter copy unchanged.

CHECK: `cd ui/command-center && npx vitest run src/components/chat/RoleRoutingPrompt.test.tsx src/components/settings/SettingsView.models.test.tsx src/lib/costMeter.test.ts`
EXPECT: exit 0; Apply label present; cost meter still one `$`.

## N6 — searchable picker + validate-without-save

`ModelPicker.tsx`: typeahead across configured providers.
`ConfigureProviderModal`: Test sends a typed key as `api_key` and does not
persist first.

- Completeness: empty query shows all; query filters provider and model.
- Bugs: Test with empty field still checks stored config.
- Wiring: `CheckProviderRequest.api_key` optional.
- Regression: saving still requires Apply/Save.

CHECK: `cd ui/command-center && npx vitest run src/components/chat/ModelPicker.test.tsx`
EXPECT: filter “haiku” hides opus.

## N7 — teachable cost_optimizer

Add `cost_optimizer` to `TEACHABLE` (Settings → models). Teaching step opens
that pane and names the Apply button.

- Completeness: descriptor already exists; curriculum was the miss.
- Bugs: tab is `Settings`, section `models` (in NAV_CATALOG_TABS).
- Wiring: `learn_next` can offer it; agent lesson mentions Apply.
- Regression: `coding_harness_capabilities_are_discoverable_and_teachable`.

CHECK: `cargo test -p permagent --lib agents::self_knowledge::teachable`
EXPECT: `find_teachable("cost_optimizer")` is Some.

## Later (not this DAG)

- SSE byte holdback (0.75s) once we own the wire; N2’s commit flag is the
  harness equivalent.
- Telegram reply-scoped `/stop` — skipped: iOS remote control covers stop.
- FCC-style `ANTHROPIC_BASE_URL` gateway — skipped: Claude Code and Codex
  already run as workers from Build → Projects, not as a proxy in front of
  the daemon.

## N8 — subscription-first on Projects

`ui/command-center/src/components/grow/codingAgents.ts` + `ProjectChip.tsx`

The Projects dropdown already launches Claude / Codex / Cursor. Surface the
same ranking goal dispatch uses: subscription CLIs first ($0 at the margin),
Permagent for routed cheap models. Grow’s Send-to-agent select uses the same
list.

- Completeness: Claude, Codex, Cursor tooltips name subscription; Permagent
  says it is not cheaper than those CLIs if they are installed.
- Bugs: `onLaunch` commands stay string literals (desktop launcher guard).
- Wiring: Grow select labels `$0 extra` / `routed` from the same catalog.
- Regression: button order and commands unchanged.

CHECK: `cd ui/command-center && npx vitest run src/components/grow/codingAgents.test.ts src/components/build/ProjectChip.test.tsx`
EXPECT: subscription CLIs before permagent; Claude tooltip contains “subscription”.
