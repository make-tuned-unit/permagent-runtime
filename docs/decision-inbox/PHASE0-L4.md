# Decision Inbox — Lane L4 (UI: Morning Briefing) — Phase 0

Status: PHASE 0 — audit + UX design + static mockup. No feature code. Phase 1 gated on
mockup approval (`docs/decision-inbox/mockup-l4.html`).

## 1. Environment proof

```
pwd: /Users/jessesharratt/dev/permagent-worktrees/di-ui
branch: inbox/home-card
HEAD: 77288cb45 fix(ci): stage sherpa-onnx prebuilt libs outside target ... (#284)
      985fde8a3 refactor(brain): SafeBrain newtype ... (#277)
      6409ea09a fix(world): stop per-render geometry churn ... (#290)

git worktree list (relevant rows):
/Users/jessesharratt/dev/permagent-runtime              [main]
/Users/jessesharratt/dev/permagent-worktrees/di-ui      [inbox/home-card]   <- this lane
/Users/jessesharratt/dev/permagent-worktrees/di-daemon  [inbox/daemon-core]
/Users/jessesharratt/dev/permagent-worktrees/di-henry   [inbox/henry-loop]
/Users/jessesharratt/dev/permagent-worktrees/di-verify  [inbox/verification]
```

Note: branch base 77288cb45 is a few commits behind main HEAD 0522cd2ab (missing #289,
#274, #275). The design-system commit c0c4582d8 (#273) IS in ancestry — `tokens.ts` in
this worktree contains the full theme/`_syncCssVars` system. Low rebase risk; tracked in
Risks.

All paths below are relative to `ui/command-center/` in this worktree.

## 2. Dashboard card-registry audit

### Registration
- `src/components/dashboard/cards/registry.ts:14-19` — `CARD_REGISTRY:
  Record<string, CardRegistryEntry>` with 4 entries (`hero`, `stats`, `in_flight`,
  `recent`). Entry shape (`registry.ts:7-12`): `{ component, name, description,
  defaultSize: {w,h} }` on a 12-column grid.
- `src/components/dashboard/useLayout.ts:17-24` — `DEFAULT_LAYOUT` hardcodes the four
  card placements. Layout is persisted server-side via `GET/PUT /api/dashboard/layout`
  (`useLayout.ts:30,37`).
- `src/components/dashboard/AddCardPicker.tsx:14-16` — picker offers
  `CARD_REGISTRY` entries not already placed; adding a new card type to the registry
  automatically makes it addable.
- Mounting: `src/components/workspaces/WorkspaceRenderer.tsx:25` registers
  `dashboard: Dashboard` in `TOOL_COMPONENTS` (ToolType union at `src/lib/store.ts:127`).
  **workspaces/ is another swarm's territory — we will NOT add a new ToolType.** The
  inbox detail view must live inside `dashboard/` (see UX spec §4.2).

### Data flow (exemplar: InFlightCard, end-to-end)
1. `src/components/dashboard/useDashboard.ts:15-21` — single fetch
   `apiFetch<DashboardData>('/api/dashboard')`; **polls every 15s**
   (`useDashboard.ts:25`). Errors swallowed (`catch { /* ignore */ }`).
2. `src/components/dashboard/Dashboard.tsx:53` consumes the hook;
   `Dashboard.tsx:183-188` maps payload slices to card props via `cardDataMap`
   keyed by card type (`in_flight: { tasks: data.in_flight }`).
3. `Dashboard.tsx:246-295` looks up `CARD_REGISTRY[card.type]`, renders
   `<Component {...props} />` into a CSS grid cell (`gridColumn`/`gridRow` spans,
   `ROW_HEIGHT=60`, `GAP=16`, `Dashboard.tsx:48-49,238-241`).
4. `src/components/dashboard/cards/InFlightCard.tsx:21-53` — pure presentational:
   surface + border + `cardShadow`/`cardHighlight` elevation (`InFlightCard.tsx:27-29`),
   `SectionTitle` header (`atoms.tsx:4-12`), explicit **empty state**
   (`InFlightCard.tsx:35-43`: primary line + dim hint line).

### Loading / empty / error states
- Loading: dashboard-level only — `Dashboard.tsx:175-181` renders a centered
  `<Mobius state="thinking">` until first fetch resolves. Cards never see loading.
- Empty: per-card, two-line centered text pattern (`InFlightCard.tsx:39-42`).
- Error: silent — stale data stays on screen, no per-card error UI
  (`useDashboard.ts:19`). Save errors on layout get a transient `SaveIndicator`
  (`Dashboard.tsx:424-441`).

### Events client / live updates — key finding
- The only real-time stream is **per-session SSE**: `src/lib/store.ts:867-919` opens
  `EventSource(api.sessionEventsUrl(sessionId))` → `/sessions/{id}/events`
  (`src/lib/api.ts:336-337`), with exponential backoff 1s→30s and Last-Event-ID resume.
- `src/lib/eventBus.ts:41-64` is a frontend-local zustand buffer
  (`useEventBus.addEvent`, 1000-event ring, 24h prune). Its **only producer** today is
  Terminal (`src/components/terminal/Terminal.tsx:295-304`, marked transitional). No
  daemon-global event stream feeds it.
- Dashboard cards do NOT subscribe to events; they poll (`useDashboard.ts:25`).
- Consequence for L4: live decision updates need either (a) L1 emitting
  `decision_created`/`decision_resolved` on a global stream, or (b) 15s polling like the
  rest of the dashboard. Phase 1 ships (b); (a) is an L1 expectation (§6).
  `PermagentEventType` (`src/lib/store.ts:112-119`) would gain the two decision types.

## 3. Design system findings

- Tokens: `src/styles/tokens.ts` — `color` (`:3-21`), `font` (`:23-27`: Manrope display /
  Inter body / JetBrains Mono), `ease` (`:29-33`), `radius` (`:35`: 6/10/14/20/999),
  `shadow` (`:37-41`).
- Themes: `ThemeId = 'dark' | 'aurora' | 'silver'` (`tokens.ts:47`). Per-theme
  `ThemeColors` at `tokens.ts:81-140` (aurora = dark + different `ribbonGradient`,
  `tokens.ts:100-103`); workspace/card gradients in `THEME_GRADIENTS`
  (`tokens.ts:148-179`). Tailwind bridge via `_syncCssVars()` RGB triplets
  (`tokens.ts:223-255`, from #273).
- Access pattern: `useTheme()` (`src/styles/useTheme.ts:9-22`) returns `{ theme,
  gradient, colors, density, reduceMotion, ... }`; re-renders on `onThemeChange`.
  Components use **inline styles from tokens**, not Tailwind classes, on the
  post-#273 chat/dashboard surfaces.
- reduceMotion: persisted flag `getReduceMotion()` (`tokens.ts:289-290`), surfaced by
  `useTheme()`; exemplar: StreamingIndicator's shimmer has a static fallback (per #273).
  Rule for L4: every transition/animation gated on `reduceMotion`.
- Elevation: `boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', ')`
  (`InFlightCard.tsx:29`) — REQUIRED for silver's glass-edge highlight.
- Patterns to reuse:
  - Card shell + `SectionTitle` + `Stat`: `atoms.tsx:4-34` (uppercase 11px dim label,
    Manrope 32px value).
  - Expandable row + detail: `src/components/events/EventRow.tsx:78-111` (click-to-expand,
    type badge `rounded px-1.5 py-0.5 text-[9px] font-mono uppercase`) and
    `EventDetail.tsx:35-120`.
  - Chip: `src/components/chat/AttachmentChip.tsx:10-30`.
  - Buttons: no shared component; convention is inline-styled, `radius.md`, 12-13px
    Inter 500, 150ms ease transitions, active = cyan border + `cyanSoft` bg
    (`Dashboard.tsx:214-229`). Primary actions use `ribbonGradient` + `textOnAccent`
    (`tokens.ts:62` comment: "Brand ribbon gradient for primary buttons / AI moments").
  - Modal/overlay inside dashboard: `AddCardPicker.tsx:18-37` (fixed inset scrim
    `rgba(0,0,0,0.5)` z-100, surface panel, header row + close).

## 4. UX spec — "The Morning Briefing"

### 4.1 Home card (`decisions` registry entry)
- Registry: `decisions: { component: DecisionsCard, name: 'Decisions', description:
  "What Henry needs from you", defaultSize: { w: 5, h: 4 } }` — pairs with `hero` (w:7)
  on row one or sits beside `stats`.
- Content: uppercase dim label `NEEDS YOU`; `Stat`-style number **N** (cyan when N>0);
  line "Henry needs N answers"; hint line (mono, dim) "oldest waiting 3h"; footer link
  "Review →". When Tier-1 activity exists: secondary dim line "Henry handled K things
  overnight".
- Whole card is tappable (cursor pointer, `borderHi` on hover, no scale transform) →
  opens the inbox overlay. Card itself is glanceable only; no actions on the card.
- Empty: number area replaced by success check + "No decisions needed." + dim
  "K goals in flight." (two-line empty pattern per `InFlightCard.tsx:39-42`).
- Live: badge count refreshes on the dashboard 15s poll (Phase 1), upgrade to event
  push when L1's stream lands.

### 4.2 Inbox overlay (decisions detail view)
- Rendered from within `dashboard/` as a full-screen overlay using the AddCardPicker
  scrim pattern (wider panel, `min(720px, 92vw)`, max-height 86vh) — avoids touching
  `workspaces/`. File plan: `src/components/dashboard/decisions/DecisionInbox.tsx` +
  `useDecisions.ts` + `DecisionItem.tsx` + `EvidenceDigest.tsx`.
- Header: "Decision inbox" (Manrope 600) + count + close (FiX).
- Body order: ranked Tier-2/unblock items (max 10) → "+M more" overflow link (loads
  next page in place) → collapsed Tier-1 group → footer "History →"
  (audit list = `GET /api/decisions/history` rendered in the same overlay).

### 4.3 Item anatomy (Tier-2)
1. Row 1: rank-ordered. Kind badge (`APPROVAL` cyan / `UNBLOCK` warning, EventRow badge
   style) + **one-line ask** (Inter 13 600, ellipsis) + age (mono 11 dim, right).
2. Row 2: "Approving will: <concrete effect>" (Inter 12, textMuted) — verbatim from
   `effect_summary`, single line, ellipsis.
3. Row 3: recommendation chip — pill, `cyanSoft` bg, cyan text: "Henry recommends ·
   Approve" (+ optional confidence). Chip is informational, not a button.
4. Row 4 actions: **[Approve]** primary (`ribbonGradient` + `textOnAccent`),
   **[Reject]** ghost with `danger` text, **[Options]** ghost (opens the
   `options[]` list inline as radio rows + confirm), **[Add note]** ghost (inline
   textarea, note rides along with the answer or posts alone).
5. **Individual confirmation (Tier-2 rule):** Approve/Reject swap the action row in
   place for "Confirm <action> — <effect restated>" + [Confirm] [Cancel]. One item
   confirmable at a time. **No checkboxes, no select-all, no batch-approve affordance
   anywhere in Tier-2.**
6. Evidence digest: "Evidence ▸" toggle expands a **plain-text** block (S2):
   `JetBrains Mono 11px`, `pre-wrap`, `codeBg` background, sections CHECKS / DIFF /
   VERIFIER / COST (cost-to-date). Rendered via React text nodes only — **no markdown
   pipeline, no dangerouslySetInnerHTML, no auto-linking; URLs and `**markdown**` in
   escalation text render inert as literal characters.** `user-select: text` for copy.

### 4.4 Unblock items
- Headline = worker's `specific_ask` **verbatim**, quoted, no paraphrase. `UNBLOCK`
  badge (warning palette). Row 2 becomes "Answering will: <effect>"; primary action
  label adapts (e.g. [Send answer] when freeform, or option buttons from `options[]`).

### 4.5 Tier-1 FYI group
- Single collapsed row after ranked items: success-dot + "Henry handled K things" +
  chevron. Expanding lists one-line rows (action summary + age + "audit →" link to the
  decision's history entry). Read-only; no actions; no per-row buttons.

### 4.6 Ranking, overflow, empty
- Server provides rank; client renders order as given, max 10; "+M more" link loads the
  remainder. Empty inbox: centered "No decisions needed." + "K goals in flight." dim
  line (same copy as card empty state).

### 4.7 Motion & themes
- All transitions ≤200ms `ease.out`, and **disabled when `useTheme().reduceMotion`**
  (expand/collapse becomes instant show/hide; no shimmer anywhere).
- Colors/typography exclusively from `useTheme().colors` + `font`/`radius` tokens; both
  shadow members joined so silver gets its glass highlight. Verified against all three
  themes in the mockup.

## 5. Mockup
- `docs/decision-inbox/mockup-l4.html` — self-contained static HTML, hardcoded token
  values from `tokens.ts`/`THEME_GRADIENTS`. Dark theme default, toggle for
  dark/aurora/silver. Four states: (A) Home card incl. empty variant, (B) inbox list
  with Tier-2 item + Tier-2 confirm step + unblock item + Tier-1 collapsed group +
  "+M more", (C) expanded plain-text evidence digest (shows inert markdown/URL), (D)
  empty state. `prefers-reduced-motion` honored. **This file is the Phase 1 approval
  gate.**

## 6. Lane L1 API expectations + Phase 1 mock plan

Expected endpoints (Bearer-token auth like all `apiFetch` calls):

```
GET /api/decisions
  -> { decisions: Decision[],          // ranked, server-ordered
       total_pending: number,           // for "+M more" (M = total_pending - decisions.length)
       handled_count: number,           // Tier-1 K since last view
       goals_in_flight: number,         // empty-state copy
       oldest_pending_at: string|null } // ISO, for "oldest waiting Xh"
  Decision: { id, tier: 1|2, kind: 'approval'|'unblock'|'fyi',
              ask: string,                       // one-line headline
              specific_ask?: string,             // unblock: rendered verbatim
              effect_summary: string,            // "approving will: ..."
              recommendation: { action: string, confidence?: number },
              options?: { id, label, effect_summary }[],
              evidence: { checks: string,        // plain text, untrusted
                          diff_stat: string, verifier_rationale: string,
                          cost_to_date_usd: number },
              created_at: string, goal_id?: string, session_id?: string }

POST /api/decisions/{id}/answer
  body { action: 'approve'|'reject'|'option'|'answer', option_id?, text?, note? }
  -> 200 { decision } (resolved) | 409 if already resolved elsewhere

GET /api/decisions/history?limit=50&before=<iso>
  -> { items: ResolvedDecision[] }  // incl. Tier-1 auto-handled, for audit links
```

- Live updates (nice-to-have from L1): `decision_created` / `decision_resolved` on a
  global event stream; until then L4 polls at 15s (`useDashboard` precedent).
- Mock plan (Phase 1, until L1 merges to the integration branch):
  `src/components/dashboard/decisions/mockDecisions.ts` implementing the same
  `useDecisions()` interface — in-memory fixture set (2 approvals, 1 unblock, 7 Tier-1,
  overflow >10 case), 300ms artificial latency, mutation on answer, 409 path. Selected
  by `import.meta.env.VITE_DECISIONS_MOCK === '1'`; the real client is a drop-in swap
  because the hook is the seam. No daemon changes needed by L4.

## 7. Risks
1. **No global event stream exists** — eventBus has one frontend producer; per-session
   SSE only (store.ts:867). If L1 doesn't ship a stream, "live" is 15s-poll fidelity.
2. **DEFAULT_LAYOUT won't retrofit** — users with a persisted layout won't see the new
   card until they add it via AddCardPicker (`useLayout.ts:29-33` overwrites default).
   May need a one-time layout migration or "new card available" nudge — decide in Phase 1.
3. **Branch behind main** (#289/#274/#275 missing at 77288cb45) — trivial UI overlap
   expected, but rebase before Phase 1 PR.
4. **409/conflict UX** — if Henry resolves or another window answers first, the item
   must drop out gracefully on next poll; confirm-step must handle stale ids.
5. **Untrusted text everywhere** — ask/effect/recommendation strings are also
   escalation-derived; the S2 plain-text rule applies to ALL decision fields, not just
   the digest (React text nodes already escape; rule is "never add a markdown renderer").

## 8. Proposed issues (out of scope for this lane)
1. Dashboard error state: `useDashboard` swallows fetch errors silently — stale data
   shown with no indicator (`useDashboard.ts:19`).
2. Promote a shared Button/Chip atom — post-#273 surfaces re-implement identical
   inline-styled buttons (Dashboard.tsx:214, AddCardPicker.tsx:70, sidebar rows).
3. Wire daemon events into `useEventBus` — buffer + filters exist but only Terminal
   feeds it (Terminal.tsx:295 marked transitional).
4. Layout migration mechanism for newly-registered default cards (risk #2 above).
