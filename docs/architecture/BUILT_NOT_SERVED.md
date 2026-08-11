# Built but not served — the wiring inventory (2026-08-11 sweep)

Three systematic sweeps (orphan wiring, agent parity + teachability, dead data
lanes) answering Jesse's question: *how much of what we have built is not being
properly served?* Answer: a lot — ~15 dead data lanes, ~20 orphaned
events/routes clusters, an agent that can read everything and see nothing, and
a teaching curriculum that retires after 14 of ~60 capabilities.

North star (the agents-home principle): every feature ships with a UI surface,
an agent tool, a teachable entry, and an honest quiet-state. Anything missing a
leg is unfinished. No approve button without a real effect arm.

## The ranked wiring plan

### Wave 1 — correctness of promises already made (small, load-bearing)

1. **`needs_human_attention` is INVERTED** — the flag meaning "raise this to
   the user" is read only as a filter that HIDES the goal (decisions.rs:1366,
   cards.rs:1103). Add a needs-attention bucket to the Inbox instead. (XS)
2. **Incidents never resolve** — no `UPDATE incidents SET status` exists;
   every worker prompt accrues stale open incidents forever, silently
   degrading quality. Add resolve path + a triage surface. (S)
3. **Dead-tab teaching lesson** — the dashboard lesson navigates to
   nonexistent tab "Home" (platform_extensions/mod.rs:256); add a guard test
   walking EVERY descriptor's teaching steps against the nav catalog. (XS)
4. **Notification policy bypassed by its own client** — the router computes
   routing and emits `notification_routed`; the UI ignores it and re-derives
   from raw facts, so thresholds/digest silently do nothing. Switch
   notifications.ts to the routed event. (S)
5. **ActionRequired briefings are un-acknowledgeable** — no ack route, no
   button; the unacked list grows forever and never toasts. (S)
6. **`connectionStatus` written, never rendered** — the app goes silently
   stale on daemon disconnect. One dot in the sidebar. (XS)

### Wave 2 — give the agent eyes and hands (the autopilot spine)

7. **`observe_app surface:"view"`** — Henry cannot see which tab/project/
   modal the user is on; the signal already flows into the activity journal
   unread. Unlocks idempotent navigation, teach-in-place, contextual offers.
   (M — the single highest-leverage item)
8. **ITEM_CATALOG last-mile four** — card / note / memory / person deep-links;
   the store actions exist and are human-used (openCardOnBoard,
   focusProjectNote, focusBrainMemory, openPersonDetail); the exclusion
   comment is stale now that list tools return ids. (S)
9. **`card_update` + due-date + column rename tools** — Henry can create and
   destroy cards but not edit one; and the card `auto-approve` toggle — the
   autopilot dial itself — has no tool. (M)
10. **`app_action` dashboard/browser/voice** — add/remove dashboard cards,
    browser tabs/bookmarks, voice start/stop: three customization surfaces,
    zero new plumbing (the #254 catalog pattern). (S–M)
11. **`set_setting` tool gated through the Decision Inbox** — Henry can
    describe every guardrail and flip none; a tier-1 decision per change
    reuses answer_decisions verbatim. Needs a careful agent-writable key
    allowlist. (M, biggest single parity jump)

### Wave 3 — dead features with both halves built

12. **Gmail OAuth cluster** — routes + UI listeners exist; the Rust emitters
    were never written (same listener-outlives-emitter class as the meeting-
    link bug). (S)
13. **Supervised-terminal gate events** — emitters exist, the promised inbox
    bridge was never built (#399/#400 S3). (M)
14. **ExecutionReceipt liveness strip** — heartbeats captured every 30s and
    thrown away; render alive/first-output/attempt-N-of-cap on the goal card.
    (S)
15. **Verifier panel + rubric in the evidence digest** — per-panelist
    verdicts (shipped 2026-08-11) render nowhere. (S)
16. **verifier.json Settings pane** — model, panel_models, auto-approve
    types: real dials, hand-edit-JSON-only today. (S)
17. **Session search/fork/export routes** — five handlers, no UI entry
    points. (M)
18. **Pronunciations editor** — the agent learns them; the user can never see
    or correct them (write-only feature). (S)
19. **Curriculum tier-2** — TEACHABLE covers 14 of ~60 capabilities and
    permanently exhausts; add missing surfaces + workers/guards tier,
    fix the two dead usage signals. (M)

### Cleanup (delete, don't wire)

Legacy Goose route surface (~55 routes), threads/thread_manager (orphaned
model), `integrations` table, dead store stubs (loadEvents, addChatMessage),
`browser_current_url` command, dropped World zone components,
`recognition_events.injected_memory_ids_source` (reader-only column).

### Guard to add (prevents the whole class)

A build-time assertion that every `api.listen('<name>')` in command-center has
a matching Rust emit site — would have caught the meeting-link regression, the
Gmail cluster, and the message-mirroring events. Plus its inverse as a report
(emitted-never-listened) run in CI as advisory.

Full sweep details (file:line for every item) live in the 2026-08-11 session
transcripts; this doc is the ranked index. Update it as waves land.
