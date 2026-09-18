/**
 * The Grow lens's wire types — one module, so the panels that share a payload
 * share its shape rather than each re-declaring it.
 *
 * Split out of GrowView.tsx (R9). Every interface here mirrors a Rust response
 * (camelCase); the comments naming the route and the line it mirrors moved with
 * them unchanged, because they are the only thing tying the two sides together.
 */

// The deterministic growth inbox (backend GET /api/projects/:id/growth-inbox).
// Ranked with NO LLM from the project's real signals — matches the Rust
// response (camelCase). See crates/goose-server/src/routes/grow.rs.
export type MovePriority = 'high' | 'medium' | 'low';
export interface GrowthMove {
  title: string;
  why: string;
  priority: MovePriority;
  evidenceCount: number;
}
export interface GrowthWin {
  title: string;
  why: string;
}
export interface GrowthInboxData {
  moves: GrowthMove[];
  wins: GrowthWin[];
  signal: { posts: number; shipped: number; activeGoals: number; daysSinceLastPost: number | null };
}

// ── Analytics connection (backend routes/grow_analytics.rs) ──────────────────
// Ruled decision (2026-07-20): the analytics lens is an API CLIENT to an
// existing web-analytics account (read-only stats fetch), not a self-hosted
// collector. Wire types mirror the Rust responses (camelCase).
export type AnalyticsProviderId = 'plausible' | 'plausible_v2' | 'goatcounter';
export const PROVIDER_LABELS: Record<AnalyticsProviderId, string> = {
  plausible: 'Plausible (v1 · CE)',
  plausible_v2: 'Plausible Cloud (v2)',
  goatcounter: 'GoatCounter',
};
export interface AnalyticsConnectionStatus {
  connected: boolean;
  provider: AnalyticsProviderId | null;
  baseUrl: string | null;
  siteId: string | null;
  /** Whether a key is stored server-side — the key itself is never sent back. */
  hasApiKey: boolean;
}
export interface AnalyticsStatsData {
  connected: boolean;
  provider: AnalyticsProviderId | null;
  periodDays: number | null;
  visitors: number | null;
  pageviews: number | null;
  /** Fetch failures (provider down, bad key, sovereign mode) arrive here — honest, never faked. */
  error: string | null;
}
export interface AnalyticsTestResult {
  ok: boolean;
  visitors: number | null;
  pageviews: number | null;
  error: string | null;
}

// First-party analytics (#23) — the daemon is the collector; no third party.
// Backend: routes/first_party_analytics.rs.
export interface FirstPartySetup {
  enabled: boolean;
  siteKey: string | null;
  ingestBase: string | null;
  ingestUrl: string | null;
  snippet: string | null;
  agentPrompt: string | null;
  /** Drain mode: the site relays, this daemon pulls. */
  drainUrl: string | null;
  drainSecret: string | null;
  cursor: string | null;
  lastDrainAt: string | null;
  lastError: string | null;
  receiving: boolean;
}
export interface VerifyCheck {
  id: string;
  label: string;
  passed: boolean;
  detail: string;
}

export interface VerifyResponse {
  verified: boolean;
  checks: VerifyCheck[];
  summary: string;
}

export interface FirstPartyStats {
  enabled: boolean;
  receiving: boolean;
  periodDays: number;
  pageviews: number;
  /** Distinct device signatures — NOT people. See the label in the UI. */
  deviceSignatures: number;
  eventsLast5m: number;
  botsExcluded: number;
  includingBots: boolean;
  /** When the drain loop last completed a pass — the staleness signal. */
  lastDrainAt?: string | null;
  /** Events the relay holds beyond our cursor; null for pre-v41 relays. */
  drainLagEvents?: number | null;
  byDay: { day: string; pageviews: number; visitors: number }[];
  topPages: { name: string; count: number }[];
  topReferrers: { name: string; count: number }[];
  topEvents: { name: string; count: number }[];
  topSources: { name: string; count: number }[];
  topCampaigns: { name: string; count: number }[];
  /** Answer-engine visits (medium=aeo / answer_engine_visit). */
  aeoVisits?: number;
  sessions: number;
  bounceRate: number | null;
  pagesPerSession: number | null;
  topEntryPages: { name: string; count: number }[];
}

export type GrowLens = 'actions' | 'results' | 'strategy' | 'calendar' | 'analytics';
// Async lifecycle for data-backed sections — loading / ready / error are
// distinct so a fetch failure never masquerades as an empty result.
export type LoadState = 'loading' | 'ready' | 'error';

/** Another action verified inside the same comparison window. */
export interface Confounder {
  id: string;
  title: string;
}

/** One judged window. Mirrors `OutcomeView` in routes/growth_actions.rs:49. */
export interface ActionOutcome {
  windowDays: number;
  /** helped | hindered | no_effect | inconclusive | confounded */
  verdict: string;
  /** One sentence carrying the numbers the verdict rests on. The column is NOT
   *  NULL by design (growth_actions.rs:55), so this always renders. */
  rationale: string;
  deltaPct: number | null;
  confounders: Confounder[];
  judgedAt: string;
}

/**
 * The durable half of an action. Absent when the row could not be persisted —
 * `persist` swallows database failures so a hiccup costs a Verify button rather
 * than the advice itself (growth_actions.rs:492-493), which is exactly the case
 * the "cannot be verified" branch below renders.
 */
export interface ActionIdentity {
  id: string;
  /** suggested | dismissed | done | verified | measuring | judged | archived.
   *  `archived` is the user's shelf: the action leaves the active board but
   *  keeps being measured while it still owes a window, and keeps feeding the
   *  agent's learning (store.rs `board`, `pending_measurement`). */
  status: string;
  /** pageviews | sessions | aeo_visits | bounce_rate */
  targetMetric: string | null;
  /** up | down */
  targetDir: string | null;
  /** git | content | event | self */
  verifiedBy: string | null;
  verifiedAt: string | null;
  /** The receipt for "this shipped" — the full sha of the commit the `git`
   *  check passed against. Present only when `verifiedBy === 'git'`; omitted
   *  from the JSON (not sent as null) for every other strategy, so its mere
   *  presence is itself the claim "a commit did this". */
  verifiedCommit?: string | null;
  /** The passing check's own sentence, verbatim, STORED rather than
   *  recomputed on render — re-running the git check later searches
   *  `--since=created_at` and can honestly name a different, later commit, so
   *  redrawing this from a fresh check would silently rewrite the receipt. */
  verifiedDetail?: string | null;
  outcomes: ActionOutcome[];
  /** The reading frozen at verification — what every window is compared
   *  against. Absent for an action that was never verified, and for one whose
   *  stored baseline no longer parses (the backend sends nothing rather than a
   *  zero, which would read as "there was no traffic before the change"). */
  baseline?: BaselineView | null;
}

/** One window of the frozen baseline. Mirrors `BaselineWindow` in
 *  routes/growth_actions.rs. */
export interface BaselineWindow {
  windowDays: number;
  /** Inclusive UTC date, `YYYY-MM-DD`. */
  start: string;
  /** Exclusive UTC date, `YYYY-MM-DD`. */
  end: string;
  value: number;
  /** What the value rests on — the count itself, or the session denominator for
   *  a rate. "70% bounce over 8 sessions" and "over 800" are different claims. */
  denominator: number;
}

/** The frozen baseline as the Tracking card renders it. Mirrors `BaselineView`
 *  in routes/growth_actions.rs. */
export interface BaselineView {
  /** pageviews | sessions | aeo_visits | bounce_rate */
  metric: string;
  /** up | down */
  dir: string;
  /** First fully-post-change UTC day, `YYYY-MM-DD`. */
  pivot: string;
  takenAt: string;
  windows: BaselineWindow[];
}

export interface GrowthVerifyResponse {
  verified: boolean;
  identity: ActionIdentity | null;
  /** Every strategy that was tried, passed or not, so a card can say why it
   *  could not confirm rather than reading as "not done". */
  checks: VerifyCheck[];
  reason: string | null;
}

/** One measured result from another project, named so the claim can be audited. */
export interface TransferExample {
  projectName: string;
  title: string;
  /** helped | hindered | no_effect */
  verdict: string;
  deltaPct: number | null;
}

/**
 * What this action's CATEGORY has actually done on the user's other active
 * projects. Mirrors `TransferNote` in routes/growth_actions.rs:102.
 *
 * Computed server-side from measured outcomes and never authored by the model,
 * which is the whole point: a model writing "this worked on three similar
 * projects" is self-assessed prose, while the same sentence derived from
 * `growth_action_outcomes` is a claim the user can check — hence `examples`,
 * which is why the disclosure below is not optional decoration.
 *
 * The aggregate and the segment halves are separate fields on purpose. Merging
 * them would hide the Simpson's paradox the proposal warns about: an overall
 * "helped" that quietly fails on projects shaped like this one.
 */
export interface TransferNote {
  category: string;
  /** Distinct OTHER active projects that measured this category. */
  projects: number;
  helped: number;
  hindered: number;
  noEffect: number;
  medianDeltaPct: number | null;
  /** e.g. "content site, 300+ views/wk, mostly search" — THIS project's shape. */
  segmentLabel: string;
  segmentProjects: number;
  segmentHelped: number;
  segmentHindered: number;
  segmentNoEffect: number;
  /** At most three, projects like this one first. */
  examples: TransferExample[];
}

export interface GrowthAction {
  title: string;
  /** MAY be the empty string. The durable `growth_actions` row is the truth and
   *  it has no column for evidence, so this comes from the prose cache — which
   *  a later review can prune. Absent prose renders as nothing rather than as a
   *  guess, for the same reason the backend refuses to default a target. */
  evidence: string;
  recommendation: string;
  /** MAY be empty, for the reason `evidence` may be. */
  steps: string[];
  /** prompt (paste into a coding harness) | post (social copy) | none */
  artifactKind: string;
  artifact: string | null;
  category: string;
  /** MAY be the empty string — see `evidence`. */
  impact: string;
  /** MAY be the empty string — see `evidence`. */
  confidence: string;
  /** Absent entirely when no other active project has ever measured this
   *  category (`skip_serializing_if`), because a badge that says nothing is
   *  worse than no badge. */
  transfer?: TransferNote | null;
  identity?: ActionIdentity | null;
}

export interface GrowthActionsData {
  /** The active board: work still asking the user for a decision. Assembled
   *  from the durable rows, so an action the last review did not re-emit is
   *  still in the payload while the sweep still measures it — in `tracking`. */
  actions: GrowthAction[];
  /** What we changed and are now measuring — verified, measuring and judged.
   *
   *  Its own list because Actions and Tracking answer different questions:
   *  "what should I do" and "did what I did work". #1053 kept these rows on the
   *  active board so in-flight work could not silently vanish while the sweep
   *  still measured it; that guarantee is kept by MOVING them somewhere they
   *  are still visible, never by hiding them. */
  tracking: GrowthAction[];
  /** Filed away by the user, newest first. */
  archived: GrowthAction[];
  /** Advice the user turned down, newest first. Distinct from `archived`
   *  because a dismissed action stays on the agent's board — its text can never
   *  be re-proposed — while an archived one is released. */
  dismissed: GrowthAction[];
  generatedAt: string | null;
  reason: string | null;
  periodDays: number | null;
  /** How many suggestions the last review discarded for naming no measurable
   *  prediction, and how many for restating something already on the board.
   *  Both are surfaced because a drop withholds advice, and a silent drop is
   *  not auditable. */
  droppedForNoTarget: number;
  droppedAsRestatement: number;
  /** Suggested actions the Steward dismissed because the change is already in
   *  this project's repo. Surfaced because a silent dismiss looks like Review
   *  did nothing. */
  droppedAsAlreadyPresent: number;
  /** A review is running on the server right now.
   *
   *  Server truth, and that is the whole point. The spinner used to be a
   *  `useState` in the panel component, so leaving the tab unmounted it and the
   *  flag was lost: the user came back to an idle button while the review was
   *  still running, and its result landed in the database unseen. Every read of
   *  this surface reports what is actually in flight, so the UI reconciles with
   *  the server on remount instead of trusting its own memory. */
  generating: boolean;
  /** When the running review started (RFC3339). Absent when none is. */
  generationStartedAt?: string | null;
}
