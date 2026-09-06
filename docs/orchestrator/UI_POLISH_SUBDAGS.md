# Whole-suite polish sub-DAGs

Status: research and inventory active; production polish has not passed.
Order mandated by the operator: macOS acceptance -> iOS -> watchOS -> suite
acceptance. Reliability fixes already in progress remain independent. Research
and read-only inventory run alongside the harness/voice programs; shared source
edits wait for their owners. This document uses the existing program controller,
not a second scheduler. Runtime goal binding requires existing approval evidence.

After the shared-system and shell gates, macOS surface lanes may run in
parallel on disjoint files, subject to available cheap-worker slots. The listed
surface order is a prioritization order, not a reason to idle an unrelated
worker. Shared primitive changes have one owner and invalidate affected receipts.
All macOS lanes join at u16; iOS remains strictly after u16 and watchOS after
iOS, as requested. This preserves methodical per-screen steps without imposing
unnecessary whole-app serial execution.

## Common per-screen child DAG

For EVERY inventory row, instantiate these ordered steps before implementation:

1. **Audit:** record surface ID, exact route/opening action, source paths, current
   capability/API/command, data identity, applicable states and before evidence.
2. **Specify:** choose the smallest change using existing components; record
   primary/contextual actions, keyboard/focus behavior, authority, side effects,
   undo/recovery and expected durable result. Identify shared consumers.
3. **Regress:** add the failure/behavior test before or with the surgical edit.
   Pure visual changes still need token/layout/accessibility regression coverage.
4. **Implement:** edit assigned paths only, wire through existing handlers and
   stores, and preserve user edits. No new framework or decorative dead control.
5. **Verify:** focused tests/typecheck, relevant API/controller integration,
   before/after same-data captures and real pointer/keyboard journey. A mock is
   identified as a mock and cannot pass the native/real-backend gate.
6. **Review:** independent review of code patterns, semantics, visual hierarchy,
   accessibility, privacy, performance and adjacent consumers; fix defects with
   bounded changed-snapshot reruns. Two failed repair attempts trigger diagnosis,
   not blind repeated verification.
7. **Receipt:** source identity, files, commands, actual counts, screenshots,
   durable outcomes and rollback. Mark the inventory row passed only after all
   applicable steps pass. New surfaces expand the ledger and reopen coverage.

The next dependency-ready screen starts at receipt completion. Use worker
completion messages, not repeated status polling. A paused human release gate
blocks only that gate, not independent approved work. Root assigns cheaper
qualified workers to bounded implementation/tests and retains design decisions
and independent acceptance. No fixed vendor preference, automatic Council fanout
or paid inference is needed for ordinary layout and regression work.

## Coverage and verification contract

Each surface row needs: ID, parent, entry/deep link, platform, source, capability,
backend/command, shared components, authority, all applicable states, owner,
status, evidence and exception reason. Include content variants (empty, loading,
ready, long/large, selected/editing, saving, error, offline/stale, unauthorized),
interaction variants (pointer, keyboard, focus, touch where applicable), and
appearance/layout variants. Explicit N/A is reviewable; blank is incomplete.

Include long English strings, long provider/model names, currency/date formats,
and representative RTL/bidirectional text alongside Dynamic Type and compact
layouts. These are robustness fixtures, not a claim that translations exist.
Do not reverse identifiers, URLs or code while adapting layout direction.

For each screen receipt use the existing evidence artifact format with required
fields: surface_id, source/diff identity, build identity, device/OS/renderer,
viewport/scale, appearance/accessibility/locale settings, fixture identity,
commands and exit codes, per-state pass/fail/not-applicable with rationale,
artifact paths, measured metrics, reviewer and rollback. Omitted applicable
states fail the receipt review. No additional production telemetry store is
introduced by this evidence contract.

Browser and referral privacy: reuse the existing validated URL/navigation and
history policy. External chatter links accept only validated HTTP(S); reject
javascript/data/file execution from analytics content. Do not retain credentials,
secret-bearing query strings or fragments in analytics/history evidence. Keep
origin-versus-path provenance explicit; never reconstruct a hidden Reddit thread
from a domain alone. Verify history retention/clear controls and existing consent,
and retain only the minimally necessary safe source link. Do not silently change
ordinary signed URL navigation while sanitizing persisted analytics/history.

Capability coverage is a bidirectional join: UI controls -> real producer, and
supported producer -> discoverable UI home. Internal-only APIs need a reason;
not every backend primitive belongs on screen. Denominator is frozen from source
inventory and reconciled at each release. Acceptance requires 100% disposition
of discovered rows, zero unexplained omissions, zero dead controls and no
unaccepted high-severity functional/accessibility defects. Do not hide missing
features by reducing the denominator or removing a control without review.

Use existing Vitest/typecheck/build, native XCTest and bounded backend tests.
Reuse unaffected receipts. Add targeted coverage for changed behavior and perform
one full relevant-platform suite at its fan-in. Never rerun all Rust for a CSS
change. Native renderer/physical-device acceptance is not substituted with Chrome.

## u0-research

Use the linked Reddit thread for candidate discovery, and primary product/design documentation for claims. Keep community preference separate from measured usability. Review UI_POLISH_RESEARCH_2026-09-05.md; do not copy assets or import competitor frameworks.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u1-inventory

Reconcile sidebar/workspace definitions, router and panel switches, settings categories, modals, deep links, native menus, detached windows, onboarding and platform view registries. Reverse-map backend/tool capabilities to their intended UI homes. Every route and supported capability must have a row; internal-only exclusions require a reason and review, never silent omission.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u2-shared-system

Audit current tokens and primitives. Start with representative Home, dense Build, long Chat and Settings views. Reuse ViewHeader, Button, Chip, Toggle, StateBlock, JobProgress, AsOf and existing modal shells. Test typography, spacing, semantic colors, focus, materials and motion before migrating consumers. No new component framework or duplicate design system.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u3-shell

The complete first-run setup journey is an explicit nested DAG:
[Onboarding O1–O5](UI_POLISH_ONBOARDING_SUBDAG.md). It includes every wizard
moment, research-backed progressive discovery, original visual direction,
failure/restart recovery and a genuine first useful interaction. Do not reduce
this scope to polishing the welcome screen alone.

Review sidebar expanded/collapsed, workspace selection, split panes, detachment/rejoin, native menu commands, window restoration, launch/onboarding, hub connection and permissions. Preserve keyboard navigation and explicit focus ownership. Show where a dropped file will go before accepting it.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u4-home

Review Home overview, Decisions list/detail/history, plan approval, consequences, task progress and empty/loading/error cases. Preserve the operator's preference for the Decisions-style header hierarchy. Never style an unverified action as successful.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u5-chat

Review main and side chat, detached chat, history/search, composer, model/agent selection, voice state and transcripts, streamed text, citations, attachments/OCR, tools, stop/retry and reconnect. Preserve scroll when the operator reads history; provide an explicit follow-latest affordance. Real voice producer acceptance is supplied by the voice program, not a mocked transcript screenshot.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u6-build

Review terminal/provider menus and next-turn switching, editor/file tree, browser address suggestions and navigation, downloads, multi-tab media ownership, session/task prompts, Council plan details, DAG workers, spend and verification. Every close/back action must affect the actual owned resource. Existing harness fixes retain path ownership.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u7-projects

Review project list/detail, goals, DAG/roadmap, activity, artifacts, creation/edit/archive and every nested dialog. Connect the evidence and next action to the same project/task identity; no detached duplicate state.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u8-people

Review lists, filters, person detail/edit, relationship navigation, project links and all secondary panes. Preserve identity and scroll on return; expose supported actions without a wall of buttons.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u9-grow

Review every Grow subtab and nested panel, project switcher, analytics ranges/charts/trends, sources/referrals, result/action connections and external links. Test with the chat pane at minimum and maximum widths. Origin-only referrers must not masquerade as exact Reddit thread URLs.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u10-finance

Review every finance tab/tool, account/provider/key setup, charts, currencies, amounts, freshness and any action confirmation. Style risky actions distinctly and keep source/authority visible. Do not perform live financial mutations as design tests.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u11-automate

Review automation/recipe/schedule/skill creation, configuration, runs, failures, pause/cancel/retry and tool setup. Reuse existing automation controllers and approval semantics; a pleasant card must not conceal a disabled or unsupported capability.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u12-brain

Review Brain search, recall results, memory detail/provenance, recognition cues, sources and existing consent/correction/forget controls. Coordinate with Spectral S1-S6 for unavailable actions; do not invent local-only UI state that claims to forget or learn. Unimplemented dependencies remain blocked and visible.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u13-world

Polish World HUD, selection, actions, station navigation, help and all existing overlays without replacing the authored Blender environment or live behavior. Root owns art. Reuse World W3/W4 receipts when unchanged, rerun affected/native gates on changes, retain full-sized orb requirements in voice rather than conflating it with World.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u14-settings

Enumerate every category and nested control from source, including preferences/persona/agents/memory/autonomy, sessions/downloads/activity/spend, tools/models/keys/devices/search/data sources and system categories discovered by the inventory. Verify saved values after reload; surface capability availability and actionable errors. Never expose credentials in snapshots.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u15-secondary

Close the inventory remainder: notification tray, command/search, skills surfaces, shared menus/popovers, tooltips, dialogs, file previews, detached panes and native lifecycle states. A row cannot be omitted because its parent tab already passed. Reconcile newly added surfaces before macOS acceptance.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u16-macos-acceptance

Run exact-build macOS journeys with expanded/narrow chat, long content and populated/empty/error data; verify keyboard/VoiceOver/focus, light/dark/high-contrast/reduced-motion/transparency behavior, resource teardown and latency. Complete every macOS inventory row and capability disposition before iOS polish begins.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u17-ios

Only after macOS acceptance, adapt the approved identity through existing SwiftUI primitives to every iOS inventory row: Home, Decisions, Control, Chat/voice, agent/model sheets, memory, project/detail, settings, pairing, permissions and all discovered surfaces. Preserve touch targets, Dynamic Type, safe areas, keyboard, rotation and device-specific navigation. Functional voice fixes may proceed before this visual phase.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u18-watchos

Only after iOS acceptance, adapt identity to every watch view, complication/widget or handoff that actually exists. Favor glanceable status and brief actions, not compressed desktop screens. Verify crown/focus/touch, accessibility, offline/unpaired states and duplicate-safe phone handoff with real WatchConnectivity.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.

## u19-suite-acceptance

Reconcile all source-discovered surfaces and supported capability homes against executed evidence. Verify consistent vocabulary, model/task identity and state across macOS/iOS/watchOS. Record exact artifact freshness, rollback and known limitations for operator release approval; screenshots and valid YAML alone do not pass this gate.

Gate: the corresponding master exit plus the per-screen contract above. Use
`UI_POLISH_SURFACE_INVENTORY_2026-09-05.md` as the authoritative coverage ledger.
