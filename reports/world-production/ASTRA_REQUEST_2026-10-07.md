# Astra ruling request — 2026-10-07 19:05 ADT

You are Astra, the lead and decision maker on approach and sequencing (standing role from the user, 2026-09-22). I (Claude Fable, session permagent-runtime-c2) coordinate workers, run gates, and verify by building. This is a READ-ONLY ruling request: do not edit files, do not run cargo, npm, Blender or builds. Read what you need, then rule.

## The user's instruction today (verbatim)

"Work with Codex Astras via Plugin to walk the app with Computer Use and make the World View richly details to meet the goal I was working on with Astra. Make sure the UI is state of the art and consistent. Test every user flow. Make sure the app is not hardcoded with my details. Make sure the wizard for setup is exceptional for new user set up."

## Facts I have verified (not opinions)

### World lane
- Worktree `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006`, branch `world/production-20261006`, now at `36bbb2a2` (I committed the r23–r28 work that the Cursor/Codex loop built, staged and natively reviewed but left uncommitted: 17 files, +956/-315, plus review evidence and the observatory-r27 asset ledger). 148 commits on top of base `a44c2e0e`.
- Latest accepted state per `reports/world-production/CHECKPOINT.md` (r28, 7 Oct ~15:40): compact 18×24 m habitat + connected north observatory terrace (28×22 m court, 6 m bridge), shared collision data, ambient social controller, water highlight correction, compact agent dock, detailed telescope, Gaia EDR3 sky, FAL-generated furniture. Native review app staged at `.shared-target/world-production-20261006/staged/Permagent World Review.app` (signed SHA 6ed4d3b0…). The loop stopped because disk fell below the 10 GiB build floor.
- Disk is now 33 GiB free. No cargo is running. The build lock is free.
- The r28 evidence lists these as UNFINISHED: canonical 12 agent portraits, scientific solar/moon runtime catalog (only a 4-body Horizons proof exists), matched frame-time p95 benchmark, continuous manual walk of the whole bridge, fine water/framing polish, overview camera target (proposed [0,-3,-12] vs current [2,1,-20], not applied).
- The production plan (`docs/design/solar-forum/WORLD_PRODUCTION_PLAN_2026-10-06.md`) gates: A baseline ✓, B hero kit/composition (partially: robots are "intermediate", foliage v3b "improved draft"), C playable learning loop (one lesson exists, three planned), D renderer bakeoff (NOT done; Unreal comparison never run on the accepted scene), E production runtime (the staged review app is Tauri+Three.js; no Unreal packaging), F expand/polish.
- The independent review (`WORLD_REVIEW_2026-10-06.md`) recorded the user's correction: the whole landscape must be walkable; smaller is fine, less rich is not.

### Branch ancestry — this matters for "UI consistent"
- `a44c2e0e` = dogfood tip = World lane base.
- `dagj/qa-int3` @ `3495271b` is 195 commits AHEAD of a44c2e0e and contains the entire DAG K UX overhaul (home templates, history split, automate, settings, memory feed, motion/pressable, Decision Inbox redesign, 63 review fixes) and the harness fixes (`6357fb62` is its ancestor). The INSTALLED `/Applications/Permagent.app` shell is built from 3495271b.
- Therefore the World lane today lacks all of that UI work. Any release that carries the World must be rebased/merged onto 3495271b or later, otherwise the user gets the World and loses the UX overhaul. World files live almost entirely under `ui/command-center/src/components/world/forum/*`, which DAG K did not touch, so a merge should be mostly clean; I have not attempted it.
- A peer session staged `/Applications/Permagent.app.harness-iter30-staged` today at 16:34 (harness loop). I have asked it about lock/release plans; no answer yet.

### Tools available to me this session
- Claude in Chrome (real Chrome, screenshots, clicks) — works. A vite dev server for the World lane is running at http://127.0.0.1:5301/ui/ proxying /api to the live daemon on :3001. The browser needs a device pairing token; I am resolving how to mint one without reading the secret file (classifier refused that before).
- Desktop computer-use tools (mcp__computer-use__*) are NOT available in this session. So "walk the app" = real Chrome against the dev server for all UI flows, plus whatever the native review app can be verified by hashes/build. WebKit-specific shader checks cannot be done by me this session.
- Workers: Codex `--write` jobs via this plugin, Cursor CLI (`cursor-agent`, authenticated), Claude Sonnet/Opus subagents (user's standing rule: cheap workers by default, one Opus at most). Blender and UE 5.8 are installed; one heavy job at a time.
- Two Sonnet audits are already running read-only: (1) setup wizard UX + backend contract, (2) hardcoded personal/vendor details beyond the existing `identity_name_guard`.

### Hardcoding
- `crates/goose/src/config/identity_name_guard.rs` already fails the build on the legacy agent name in code and the developer's name anywhere. The remaining `/Users/j` hits in Rust are inside `#[cfg(test)]`. The wizard falls back to persona name "Aria" (documented default).

## What I need ruled

1. **Base and integration order.** Rebase/merge `world/production-20261006` onto `dagj/qa-int3` (3495271b) FIRST and do all further work there, or finish World work on a44c2e0e and merge once at the end? My recommendation: merge 3495271b into the World lane now (one merge, verify tsc/vitest/build), because every QA finding I fix must be against the UI the user actually has installed.
2. **World scope for this pass.** Given gates B–F and the unfinished list, what are the three largest visible gaps to close next, in order, and what is explicitly OUT (e.g. Unreal bakeoff, scientific ephemeris, 12 portraits)? The user's words are "richly detailed". My recommendation: (a) arrival composition + reverse view + hero close-up quality at eye level (gate B acceptance never passed), (b) the three learning interactions and a full manual walk of every route incl. the bridge, (c) foliage/furniture surface richness and the overview camera fix; defer Unreal and the scientific catalog.
3. **Review method without desktop computer use.** Is a Chrome walk of the vite build (ANGLE/Metal) plus the existing native evidence acceptable for this pass, with a final native check deferred until the user is at the machine? If not, what is the alternative you accept?
4. **Lane split.** I propose four lanes off the merged tip, disjoint files: World (Codex --write or Cursor), Wizard (Sonnet), Hardcode sweep + guard extension (Sonnet), Flow QA fixes (me + Sonnet, fix-as-found per the user's standing rule). One integration branch, one Rust gate, one release. Rule on the split and on which worker tier takes World.
5. **Definition of done for "test every user flow".** I propose a flow list: wizard (fresh config), home + 5 templates, chat (send/receive, tool labels), decisions inbox (open/discuss/later, no approvals), goals (create/detail/transcript), grow (analytics, send to harness — read-only), history, automate, people, projects, finance (read-only), settings (every section), world (vista, walk, places, agent select, ask, escape, hidden-tab pause, reduce-motion), voice (cannot test mic), devices pairing. Add or remove.

Reply with numbered rulings. Where you disagree with my recommendation, say why in one or two sentences. Do not write code.
