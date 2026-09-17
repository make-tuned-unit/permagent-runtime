# Lane N2: vendor-neutral, cost-proportional routing

Branch `lane/N2` (worktree `permagent-worktrees/laneN2`), based on `e03e6d46`. I only edited files. No cargo command ran, so nothing below has been compiled or tested yet. I ran `rustfmt --edition 2021 --check` on every touched `.rs` file and it is clean.

| # | Commit | What |
|---|---|---|
| 1 | `e251b9f2` | EDIT and REVIEW (and the recommender's ORCHESTRATE) get the planner's cost band. One shared helper. (N1 finding 1) |
| 2 | `f9ec9266` | The HARD pack is derived from the configured models, not the `claude-opus-4-8` constant. (N1 finding 2) |
| 3 | `38630b24` | The roadmap planner is gated on a deterministic planning-complexity verdict. (ruling 2026-09-17) |
| 4 | `f65059b9` | The goal-level reviewer is picked from a review-complexity floor plus a proportional price rule. (ruling 2026-09-17 07:55, the scope extension) |

## 1. Band for EDIT / REVIEW / ORCHESTRATE (N1 finding 1)

- `recommend.rs:122` `cheapest_within_band` is the one helper. It finds the strongest candidate, then picks the cheapest one within `band` of it. Exact cost ties go to the stronger model, then to the id. Since commit 4 it is a thin wrapper over `cheapest_clearing_floor` (see item 4), so the bands and the review rule run the same code.
- `recommend.rs:92` `EDIT_RELIABILITY_BAND = 0.01`. It is tighter than the planner's 0.03 because EDIT's floor (0.97) is only 3 points below 1.0. A 0.03 band would treat every floor-clearer as equal and throw away the metric.
- `recommend.rs:98` `REVIEW_STRENGTH_BAND = PLANNER_STRENGTH_BAND` (0.03).
- `recommend.rs:663` `pick_banded` applies the band among floor-clearers. When nothing clears the floor, the strict `edit_rank`/`orchestrate_rank` fallback is unchanged: weak picks get no cost discount.
- `recommend.rs:702` ORCHESTRATE and `:736` EDIT use `pick_banded`. Their reason text says "within X of the strongest … at lower cost" when the band chose a cheaper model.
- `recommend.rs:805` REVIEW: among floor-clearers, take the capable different-family partition if it has any members, otherwise all floor-clearers, then apply the band. `review_rank` keeps its signature (R63c asked for that) and now only serves the below-floor fallback. The cross-family rule is unchanged.
- `capability_directory.rs:578` `planner()` now calls the same helper. `card_id_order` is at `:841`.
- Tests are in the `recommend.rs` tests module:
  - `edit_band_picks_the_cheaper_model_within_the_band` covers Fable 98.5% vs Sonnet 98.0%, synthetic and with real KB rows. Sonnet wins.
  - `edit_band_keeps_the_more_reliable_model_outside_the_band`
  - `edit_band_admits_a_free_local_model_that_meets_the_band_and_floor`
  - `review_band_picks_the_cheaper_verifier_within_the_band`
  - `review_band_keeps_the_cross_family_rule`
  - `orchestrate_band_matches_the_planner_rule`
- I updated `crates/goose/tests/delegate_model_pins.rs`. Its guard test asserted the derived EDIT pick was `claude-fable-5`; it now asserts `claude-sonnet-5` and "not the priciest". The escalation receipt test now expects `claude-sonnet-5`.

## 2. HARD pack derived (N1 finding 2)

- `packs.rs:312` `derive_hard_pack(available, provider_keyed)` returns the recommender's ORCHESTRATE pick (`recommend_from_available`, which the Apply-recommended-routing flow also uses and which now has the band), matched back to the id the user actually has. If that pick is below the floor, it falls back to:
- `packs.rs:284` `cheapest_keyed_capable_hard_pack`: the cheapest non-local KB model that clears the ORCHESTRATE floor, among providers that have a real key (`provider_has_key`, `:351`). Keyless and unknown providers don't count.
- `packs.rs:252` `load_packs()` derives HARD whenever neither `PERMAGENT_PACK_HARD_*` key is set.
- `packs.rs:170` `ModelPacks::default()` is config-free and can't see keys. Its `hard` is the cheapest KB model that clears the floor, treating every provider as reachable. Today that is `minimax/MiniMax-M2.5`. It comes from the data, not a named vendor.
- `DEFAULT_HARD_MODEL` is deleted.
- Consumers of the old constant:
  - `mod.rs` tests: `non_cheap_tiers_resolve_from_packs_unchanged` and `route_pack_composes_…` now compare against the derived pack.
  - `packs.rs` tests are rewritten.
  - `escalation.rs:532`, `derived.rs:571` and `summon.rs:4467` only assert "not equal to the pack default". They still hold.
  - **`summon.rs:4404` (`unset_role_falls_through_to_single_model_not_a_baked_default`) asserts the literal `"claude-opus-4-8"` and WILL FAIL.** R63c owns summon.rs, so I left it. At integration, delete that one `assert_eq!(pack_default.as_str(), "claude-opus-4-8")` line; the `assert_ne!` after it still covers the property.
  - `role_map.rs:16` doc is updated.
  - `permagent-eval/src/tier.rs`'s `"frontier"` bench tier also names Opus. It is an eval preset, not a router default, so I didn't touch it.
- Tests:
  - `default_hard_is_the_cheapest_kb_model_that_clears_the_planner_floor`
  - `derived_hard_pack_never_names_a_vendor_the_user_does_not_have`: OpenAI + Z.AI yields `zai/glm-5.2`, the fallback is non-Anthropic, and with nothing keyed the result is `None`.
  - `derived_hard_pack_follows_the_objective_pick_when_anthropic_is_available`: Fable wins on merit.

## 3. Complexity-gated planner (ruling 2026-09-17)

### How planning complexity is decided

This is `planner_complexity.rs:207` `assess_planning_complexity`. It is pure and reads facts only. The plan is COMPLEX if **any** of these hold:

| Signal | Threshold | Source |
|---|---|---|
| A roadmap proposal is still pending on the project when decompose starts (a redecompose: the last plan was rejected or revised) | present | `project.metadata_json.roadmap_proposal`. `create_roadmap` removes it on success. |
| Parked goals | ≥ 1 live (non-archived) goal card in Failed or with `needs_human_attention` | SQL in `orchestrator.rs:1013` `prior_plan_history` |
| Held goals | ≥ 1 live goal card with `hold_done` metadata that is not Complete | same query |
| Distinct file paths named in the objective | ≥ `PLANNING_MULTI_FILE_THRESHOLD` = 3 (`:50`) | Whitespace tokens with a known source/config extension. URLs, versions and `Node.js`-style framework names are excluded. |
| Programming languages among the named files | ≥ `PLANNING_CROSS_LANGUAGE_THRESHOLD` = 2 (`:54`) | Extension → language map. Docs and config count as files, not as languages. |
| Enumerated requirement lines (`- `, `* `, `• `, `1. `, `2) `) | > `assess::MANY_CRITERIA` (5) | objective lines |
| No file named and a large repo | `RepoFacts.file_count` ≥ `LARGE_REPO_FILE_THRESHOLD` = 500 (`:59`) | Mechanical vocabulary (`assess::MECHANICAL_SIGNALS`: typo, rename, bump, …) can clear this one signal. Prose can only route a plan down. |

Everything else is ROUTINE. The verdict always lists its reasons, including for ROUTINE. Difficulty prose ("extremely complex, use the best model", "architectural refactor") is never read as a promoting signal; that is `assess.rs:14-30`'s rule.

### Routing

- `capability_directory.rs:969` `resolve_planner_for_complexity(decomposition_pin, role_pin, directory, &PlannerGate)` (`PlannerGate` is at `:945`):
  1. An explicit `ORCHESTRATOR_DECOMPOSITION_*` or `PERMAGENT_ROLE_ORCHESTRATE_*` pin **wins for ROUTINE and COMPLEX** and is never silently overridden. It still goes through the existing floor check.
  2. COMPLEX with spend not capped goes to `planner()`, the expert pick: the strongest configured planner, band 0.03. That is Fable today.
  3. ROUTINE, or COMPLEX when the session budget band is Gate or Hard, goes to `routine_planner()` (`:640`): the cheapest capable planner (`ROUTINE_PLANNER_STRENGTH_BAND = ∞`, `:90`; every floor-clearer or graduated candidate counts as fit), through the same helper. If the session coordinator clears the floor and costs ≤ that pick, the route is `None` and the session coordinator plans (`PlannerReason::RoutineCoordinator`). The frontier model plans routine work only when it is itself the cheapest capable planner.
  4. If nothing is capable, the result is the existing `SessionFallback`.
- `planner_candidates()` (`:613`) is shared by the expert and routine picks.
- New `PlannerReason` variants `RoutineCheapestCapable` and `RoutineCoordinator` both carry `spend_capped`.
- Wiring in `orchestrator.rs`:
  - `:149` `resolve_planner_route_gated(Option<&PlannerGate>)` is now the single body. `resolve_planner_route_free()` calls it with `None`, so there are still exactly 2 production `resolve_planner_route_free()` call sites and the source-grep test holds.
  - `:4901` `resolve_planner_route(&self, gate)`.
  - `:4931` `resolve_decomposition_provider` takes the pre-resolved route.
  - `:10935` assesses complexity and emits the `planner_complexity` tracing line with its reasons.
  - `:11103` builds the gate: the coordinator is the session's provider/model; `spend_capped` means the Gate or Hard band on `accumulated_cost_usd`. The route is resolved once and reused for both the planner call and the "roadmap created" receipt, whose log line gains `planner_complexity`.
  - `:11289` records on `roadmap_proposal`: `planner_complexity` (routine|complex), `planner_complexity_reasons`, `planner_complexity_signals`, `planner`, `planner_reason`.
  - The `receipt cost_routing planner=` line format is unchanged. `set_planner` already records the provider that actually answered, and with a `None` route that is the session provider/model. I didn't touch `routing_receipt.rs`.
- Tests:
  - `planner_complexity.rs`: one-file fix → ROUTINE; multi-file refactor, cross-language, prior failed/rejected plan (3 shapes), >5 enumerated requirements → COMPLEX; prose asserting difficulty stays ROUTINE; unscoped in a large repo → COMPLEX unless mechanical; framework names, URLs and versions are not files.
  - `capability_directory.rs`:
    - `routine_plan_goes_to_the_cheapest_capable_planner_complex_to_the_expert`: glm-5.2 vs Fable.
    - `routine_planner_respects_the_floor_and_uses_the_frontier_only_when_cheapest`
    - `routine_plan_uses_the_coordinator_when_it_is_capable_and_no_dearer`
    - `complex_plan_under_a_spend_cap_gets_the_routine_planner`
    - `a_user_orchestrate_pin_wins_for_routine_and_complex`: both pin levels × both verdicts × both spend states.
  - `orchestrator.rs:39599` `prior_plan_history_reads_parked_held_and_pending_proposal` runs against a real DB and checks that a held-then-completed goal doesn't count.

## 4. Review complexity + proportional reviewer assignment (scope extension)

All of this is pure and lives in `recommend.rs`.

- **Facts** (`:1373` `ReviewFacts::from_diff(diff, call_sites)`), measured on the diff the reviewer sees. On the gate that is the R22d trunk-relative `root HEAD..goal HEAD`, not the goal's own `HEAD`, so auto-landed work still measures. The facts are:
  - changed lines and files
  - code files vs test files (the existing `tool_monitor::is_test_path` classifier)
  - languages (same extension map as item 3)
  - modules (parent dirs of code files)
  - signature changes (`call_site_completeness::detect_signature_changes`)
  - the caller's count of enumerated call sites (the R62e `git grep` data)
- **Verdict** (`:1474` `assess_review_complexity`):
  - COMPLEX, floor **0.80** (the ORCHESTRATE floor): ≥ 2 languages, OR a signature change with ≥ 1 enumerated call site, OR > 600 changed lines, OR ≥ 8 code files (`assess::MANY_FILES`).
  - MODERATE, floor **0.75**: > 200 lines (`reviewer_pick::SMALL_DIFF_LINES`), OR ≥ 3 code files, OR ≥ 2 modules, OR a signature change with no call sites found, OR code changed with no test file and > 3 lines (`review_gate::TRIVIAL_MAX_LINES`).
  - ROUTINE, floor **0.60**: everything else.
  - The ROUTINE and MODERATE floors are exactly `reviewer_pick`'s `REVIEWER_MIN_ORCHESTRATION` and `REVIEWER_STRONG_ORCHESTRATION`.
  - Risk-area vocabulary (security, money) is not read. The repo has no objective classifier for those, and a keyword list would just be prose.
- **Selection rule** (`:212` `cheapest_clearing_floor`, which is also what the bands now run):
  1. Candidates more than `MATERIAL_FLOOR_SHORTFALL` = 0.02 (`:156`) below the floor are ineligible.
  2. Walk the rest cheapest first and take the first that clears the floor.
  3. A cheaper candidate that is only within the margin is displaced only by a floor-clearer whose gain ≥ `PROPORTIONALITY_K` × ln(price ratio), with K = 0.05 (`:167`, `upgrade_is_proportionate` `:176`). The price ratio uses a $0.10/Mtok minimum, so $0 local models give a finite ratio.
  4. Why logarithmic, and why 0.05: a 2× price then needs a gain of 0.035, just past the 0.03 near-tie band; 3× needs 0.055; 10× needs 0.115. A 0.005 edge at 3× falls short by 11×.
- **Assignment** (`:1630` `assign_reviewer(facts, worker, available)`): candidates are KB-rated, non-retired models from a different family than the author. The result is a `ReviewerAssignment` (`:1579`) holding the verdict, floor, reasons, facts, `worker_family`, every candidate (provider, model, family, orchestration, $/Mtok, status: picked / clears_floor / near_floor / materially_short / same_family / family_unproven / retired), the pick, `pick_clears_floor` and `why`. If the author is unknown, there is no pick.
- **Goal-level gate** (`orchestrator.rs:22859`, inside `dispatch_cross_family_reviewer_for_salvage`):
  - The R62e call-site context is now gathered before selection and reused for the prompt.
  - The assignment's pick goes into `select_reviewer`'s derived slot, replacing the derived REVIEW map entry. A configured `PERMAGENT_ROLE_REVIEW_*` pin still wins as step 1.
  - With no pick, `select_reviewer` gets an empty pool, so it can't fall back to a model the floor ruled out.
  - For a derived-source pick, `pick.why` is the assignment's `why`.
  - `record_reviewer_assignment` (`:23025`) writes `reviewer_assignment` to the goal card, plus `selected`, `selected_source` and `recorded_at`, and emits a `reviewer assignment` tracing line.
- Tests (`recommend.rs` tests):
  - `half_a_percent_edge_at_three_times_the_price_never_wins`: both-clear and just-under-floor variants, plus the 18→60 $/Mtok EDIT case.
  - `a_pricier_model_wins_only_when_cheaper_ones_fall_short_proportionately`
  - `review_facts_measure_the_diff`
  - `routine_small_diff_gets_the_cheapest_clearing_reviewer` → `zai/glm-4.5-air`, not Fable or Gemini.
  - `complex_signature_change_uses_a_stronger_model_only_when_cheaper_ones_miss_the_floor` → `google/gemini-3-pro`, or `zai/glm-5.2` when it's available.
  - `the_recorded_reviewer_decision_contains_the_evidence`
  - `unknown_author_gets_no_reviewer_pick`
  - `review_complexity_ladder`
  - `orchestrator.rs:32084` `reviewer_assignment_is_recorded_on_the_goal_card` (DB).

### How this fits with R63c's `cheapest_cross_family_reviewer`

- R63c's helper runs `select_reviewer` step 3: the cheapest cross-family model clearing 0.60 (≤ 200 lines) or 0.75 (> 200 lines). For line counts alone that matches my ROUTINE and MODERATE floors, and among floor-clearers it matches my rule (cheapest wins).
- What it lacks:
  1. The COMPLEX tier (0.80) and the non-size signals: languages, modules, signatures with call sites, untested code.
  2. The near-floor margin and the proportional displacement.
  3. The recorded decision.
- **Integration wiring**: in `summon.rs` `resolve_review_dispatch` (around `:2712` on lane/R63c) and `after_turn.rs:589`, replace

  ```rust
  cheapest_cross_family_reviewer(author, &available, lines)
  ```

  with

  ```rust
  cost_router::recommend::assign_reviewer(
      ReviewFacts::from_diff(&trunk_relative_diff, call_sites),
      author,
      &available,
  ).pick_role_model()
  ```

  and trace or record the assignment.
  - If a delegate call site only has a line count and no diff, `ReviewFacts { changed_lines: lines, ..Default::default() }` reproduces R63c's floors exactly: ROUTINE 0.60 up to 200 lines, MODERATE 0.75 above.
  - R63c's note that the delegate path can measure 0 after auto-land means that path should compute the diff against the fork point or trunk, the same way the goal gate does (`resolve_trusted_landing_root` → `rev-parse HEAD` → `git diff <root>..HEAD`).

## Gate filters

Run these on the coordinator's build slot. The crate is `-p permagent` (goose).

```
cargo test -p permagent --lib cost_router::recommend
cargo test -p permagent --lib cost_router::capability_directory
cargo test -p permagent --lib cost_router::packs
cargo test -p permagent --lib cost_router::planner_complexity
cargo test -p permagent --lib cost_router::tests            # mod.rs route_pack / concrete_pack tests
cargo test -p permagent --lib cost_router::derived
cargo test -p permagent --lib cost_router::escalation
cargo test -p permagent --lib cost_router::assess
cargo test -p permagent --lib orchestrator::tests::prior_plan_history_reads_parked_held_and_pending_proposal
cargo test -p permagent --lib orchestrator::tests::reviewer_assignment_is_recorded_on_the_goal_card
cargo test -p permagent --lib orchestrator::tests::decomposition_default_is_no_longer_qwen
cargo test -p permagent --lib orchestrator::tests::orchestrate_role_refusal_reason_is_shared_not_duplicated
cargo test -p permagent --lib orchestrator::tests::resolve_decomposition_provider_never_returns_a_circuit_open_provider
cargo test -p permagent --lib orchestrator::tests::dispatch_cross_family_reviewer
cargo test -p permagent --lib orchestrator::tests::resolve_planner_route_free_refuses
cargo test -p permagent --test delegate_model_pins
cargo test -p permagent --lib summon::tests::unset_role_falls_through_to_single_model_not_a_baked_default   # EXPECTED FAIL until the one-line integration fix above
cargo clippy -p permagent --all-targets -- -D warnings
```

I didn't touch `routes/packs.rs` in goose-server. Its recommendations do change, though, so optionally also run `cargo test -p permagent-server packs`.

## Risks / what changes for the operator

- **Apply recommended routing, EDIT:**
  - Anthropic-only surface: it will now suggest `anthropic/claude-sonnet-5` instead of `claude-fable-5`.
  - With `google/gemini-3-pro` keyed: `gemini-3-pro` (99.0%, $14/Mtok, since Sonnet's 98.0% is inside the band at a higher price).
  - The operator's existing `PERMAGENT_ROLE_EDIT_*: claude-fable-5` pin in `~/.permagent/config.yaml` is untouched and still wins until he re-applies or unpins it. I didn't modify the config.
- **ORCHESTRATE:** the Apply flow still recommends Fable 5. Its 0.95 is 0.064 ahead of Opus, outside the band. Applying pins ORCHESTRATE, and **a pin disables the complexity gate** because a pin is the user's decision. The operator already pins ORCHESTRATE to Fable, so for him the gate does nothing until that pin is removed. Consider having the Apply flow skip ORCHESTRATE (a ruling question, not implemented).
- **REVIEW in the recommender:** on an Anthropic-only surface, the derived REVIEW entry is still Fable. Fable is alone in its 0.03 band, and no capable different-family model exists. The goal gate (item 4) and R63c's delegate path no longer read that entry, but Apply would still pin REVIEW=Fable. Pointing `recommend()`'s REVIEW row at `assign_reviewer` with a MODERATE default is a possible follow-up.
- **Planner thresholds are judgment calls:** 3 named files, 2 languages, more than 5 list lines, 500-file unscoped. The Permagent repo itself is over 500 files, so an unscoped objective like "Add offline sync" is COMPLEX and gets Fable, while "fix the typo in …" stays ROUTINE. Plan history counts every live parked or held goal on the project with no recency window, so a stale Failed card keeps that project COMPLEX until it is archived.
- **ROUTINE planner = cheapest capable.** With MiniMax-M2.5 (0.80, $1.50) or glm-5.2 keyed, routine plans go there. The planner floor (0.80) was set as "plans rarely need rescue", but routine plans on sub-frontier models are unmeasured. That is worth an eval slice before the operator unpins ORCHESTRATE.
- **Review rule:** a candidate up to 0.02 under the floor can be picked over a pricier floor-clearer when the price step isn't justified; the decision records it as `near_floor`. A COMPLEX review on a surface where only frontier models clear 0.80 will pick one of them. Per the ruling, that is proven at selection time and recorded.
- **Band semantics:** planner cost ties now prefer the stronger model before the id. That is only visible on an exact price tie.
- **Cross-lane files:** `crates/goose/tests/delegate_model_pins.rs` changed (2 assertions), and it may overlap R63c. Also `summon.rs:4404`, above.
- **Not compiled.** The generic helpers (`cheapest_within_band` → `cheapest_clearing_floor` with `&impl Fn` forwarding), `PlannerReason`'s new variants (serde-tagged) and the reordered async code in `dispatch_cross_family_reviewer_for_salvage` are the most likely places for a compile nit.
