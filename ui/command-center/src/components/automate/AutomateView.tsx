import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import type { CSSProperties } from 'react';
import { FiChevronRight, FiFolder, FiMessageSquare, FiSearch } from 'react-icons/fi';
import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { cronToEnglish } from '../../lib/schedule-format';
import { useCommandCenter } from '../../lib/store';
import { AUTOMATION } from '../../lib/vocabulary';
import { useCopyToClipboard } from '../../lib/clipboard';
import { apiFetch } from '../../lib/api';
import { useScheduleEvents } from '../../lib/useScheduleEvents';
import { usePollWhenVisible } from '../../lib/usePollWhenVisible';
import {
  formatBytes,
  sumRunRecovered,
  sumPendingBytes,
  type RecoveryTotals,
} from '../../lib/cleanupRecovery';
import { usePersona } from '../settings/useSettings';
import { RunRoster } from './RunRoster';
import { ModelReviewFindings } from './ModelReviewFindings';
import { Button, SUCCESS_FLASH_MS } from '../common/Button';
import { JobProgress } from '../common/JobProgress';
import { useLongRunningJob, type LongRunningJob } from '../../hooks/useLongRunningJob';
import { FormModal } from '../common/FormModal';
import { ViewHeader } from '../common/ViewHeader';
import { AsOf } from '../common/AsOf';
import { getApiBaseUrl, loadDaemonToken } from '../../lib/api';
import {
  bulkEligible,
  cleanupBytes,
  categoryOf,
  CATEGORY_LABELS,
  REGENERABLE_ACK,
  REVIEW_ACK,
  type CategoryKey,
} from './findingCategories';

import { Tooltip } from '../common/Tooltip';
import {
  acknowledgeFailure,
  historicalFailureNote,
  isFailureAcknowledged,
  lastFailure,
  nextRunLabel,
  runChip,
  runState,
  weekdayTime,
  type ScheduleRunStatus,
} from './scheduleHealth';
import { LearnedList } from './LearnedList';
import { JobHealthStrip } from './JobHealthStrip';
import { summarizeHealth, type JobHealthDigest } from './jobHealthModel';
import { SectionNav, SECTION_NAV_OFFSET, type SectionNavItem } from './SectionNav';
import { Chip } from '../common/Chip';
import { StateBlock } from '../common/StateBlock';
// ── Types ────────────────────────────────────────────────────────────


interface ScheduledJob {
  id: string;
  source: string;
  cron: string;
  last_run: string | null;
  currently_running: boolean;
  paused: boolean;
  current_session_id: string | null;
  process_start_time: string | null;
  worker_persona: string | null;
  // Reliability (all optional — old jobs / old daemons omit them).
  run_count?: number;
  retry_count?: number;
  max_retries?: number;
  last_status?: ScheduleRunStatus | null;
  last_error?: string | null;
  /** Failures since the last success; the daemon auto-pauses at a threshold. */
  consecutive_failures?: number;
  /** Set by every Resume. A failure before it is history, not the current state. */
  resumed_at?: string | null;
  /** Each failed run with its own end time (newest last). */
  failed_runs?: Array<{ run_number?: number; timestamp: string; session_id?: string | null; error?: string }> | null;
  /** Newer daemons only: the recipe YAML's mtime, and the next fire time. */
  recipe_modified_at?: string | null;
  next_run?: string | null;
  // Richer schedule kinds (optional — a plain cron job omits them).
  at?: string | null;
  every_seconds?: number | null;
  tz?: string | null;
  display_name: string | null;
  description: string | null;
  starter_id: string | null;
  starter_version: string | null;
  user_customized: boolean | null;
  version: string | null;
  embedded_version: string | null;
}

/** GET /schedule/{id}/inspect (routes/schedule.rs) — camelCase on the wire. */
interface InspectResponse {
  sessionId?: string | null;
  processStartTime?: string | null;
  runningDurationSeconds?: number | null;
  recipe?: {
    title: string;
    description: string;
    extensions: string[];
    instructionsPreview?: string | null;
  } | null;
  nextRuns: string[];
}

interface SessionInfo {
  id: string;
  name: string;
  createdAt: string;
  workingDir: string;
  scheduleId: string | null;
  messageCount: number;
  totalTokens: number | null;
  inputTokens: number | null;
  outputTokens: number | null;
}

export interface Finding {
  id: string;
  type: string;
  path: string;
  size_bytes: number;
  age_days: number | null;
  recommendation: string;
  action_taken: string | null;
  actioned_at: string | null;
  size_recovered_bytes: number | null;
  error_message?: string | null;
  // Storage-safety fix — optional, older ledgers won't have them. See
  // findingCategories.ts for the fallback rule and the bulk-safety
  // guarantees these fields drive.
  category?: string | null;
  consequence?: string | null;
  available?: boolean;
  availability_reason?: string;
  protection?: string;
  profiles?: CargoProfile[];
  action_source?: string | null;
}

interface ExtensionInfo {
  enabled: boolean;
  type: string;
  name: string;
  description: string;
  display_name: string;
  bundled: boolean;
  available_tools: string[];
}

// Backstop poll interval for the schedule list + run-history summary.
// `useScheduleEvents` delivers live updates on every real write, so this is
// only the fallback for whatever it misses (a dropped WebSocket, a
// cron-triggered run that fires with no route call to emit from) — the
// 2026-08-25 "schedule polling storm" health review found the old 5s
// indefinite poll driving an unindexed SQL query (97 slow-query bursts in 9
// minutes). `usePollWhenVisible` also stops this entirely while the tab is
// hidden.
export const SCHEDULE_LIST_POLL_MS = 60_000;

const CRON_PRESETS = [
  { label: 'Every weekday morning (8 AM)', cron: '0 8 * * 1-5' },
  { label: 'Every morning (8 AM)', cron: '0 8 * * *' },
  { label: 'Every Sunday evening (7 PM)', cron: '0 19 * * 0' },
  { label: 'Every Monday morning (9 AM)', cron: '0 9 * * 1' },
  { label: 'Every hour', cron: '0 * * * *' },
  { label: 'Every day at midnight', cron: '0 0 * * *' },
  { label: 'First day of every month', cron: '0 0 1 * *' },
  { label: 'Custom (cron expression)', cron: '' },
];

function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

/**
 * "Ran 3h ago" / "Running now · started 2m ago" / "Never run yet" — the ONE
 * place this sentence is built, called from both the card meta line and the
 * detail panel's "Last Run" field so they can't read as two different facts
 * (see scheduleHealth.runState for why "never" alone was not enough).
 */
function runStateWords(job: Parameters<typeof runState>[0]): string {
  const state = runState(job);
  switch (state.kind) {
    case 'running': return state.sinceIso ? `Running now · started ${timeAgo(state.sinceIso)}` : 'Running now';
    case 'ran': return `Ran ${timeAgo(state.sinceIso)}`;
    case 'never': return 'Never run yet';
  }
}

// Human summary of a job's schedule across kinds (cron / one-time / interval).
function scheduleSummary(job: ScheduledJob): string {
  if (job.at) return `Once on ${new Date(job.at).toLocaleString()}`;
  if (job.every_seconds != null) return `Every ${formatInterval(job.every_seconds)}`;
  const base = cronToEnglish(job.cron);
  return job.tz ? `${base} (${job.tz})` : base;
}

// A2: the schedule reads as English everywhere; the raw cron expression is
// still available (for anyone editing the recipe by hand) on hover/focus via
// the shared Tooltip, never printed inline. Only cron jobs have one to show —
// "once" and "every N" schedules have no expression underneath.
function rawCronTooltip(job: ScheduledJob): string | undefined {
  if (job.at || job.every_seconds != null) return undefined;
  return job.cron?.trim() ? `cron: ${job.cron}` : undefined;
}

function formatInterval(secs: number): string {
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? '' : 's'}`;
  if (secs % 86400 === 0) return plural(secs / 86400, 'day');
  if (secs % 3600 === 0) return plural(secs / 3600, 'hour');
  if (secs % 60 === 0) return plural(secs / 60, 'minute');
  return plural(secs, 'second');
}

// Derive a translucent tint from a semantic token so status backgrounds track
// the theme's hue (e.g. Silver's darker green) instead of a frozen literal.
// Non-hex tokens (already-rgba values) are returned unchanged.
function withAlpha(hex: string, alpha: number): string {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return hex;
  let h = m[1];
  if (h.length === 3) h = h.split('').map(c => c + c).join('');
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

// ── The action-row look, as the button primitive's custom properties ──
//
// This page used to carry its own `Btn` component, which hard-coded three
// looks, its own 700ms minimum-pending floor and its own success phase. The
// floor is now `MIN_PENDING_MS` inside `components/common/Button`, so `Btn` is
// gone and only its LOOK survives — expressed the way the primitive wants it,
// through `--pa-btn-*`, so hover/press/pending/disabled all come from `.pa-btn`
// instead of a pair of `filter: brightness()` mouse handlers.
//
// `Btn`'s hover was a flat `brightness(1.12)` on whatever fill it had. That has
// no CSS-custom-property equivalent, so each tone names a real hover fill: the
// same tint one step stronger, matching the danger house style already used in
// FinanceView.
function actionVars(colors: ThemeColors, tone: 'plain' | 'danger' = 'plain'): CSSProperties {
  const base = {
    '--pa-btn-pad': '4px 10px',
    '--pa-btn-radius': `${radius.sm}px`,
    fontFamily: font.body,
  };
  if (tone === 'danger') {
    return {
      ...base,
      '--pa-btn-bg': withAlpha(colors.danger, 0.1),
      '--pa-btn-fg': colors.danger,
      '--pa-btn-border': withAlpha(colors.danger, 0.2),
      '--pa-btn-bg-hover': withAlpha(colors.danger, 0.2),
      '--pa-btn-border-hover': withAlpha(colors.danger, 0.4),
    } as CSSProperties;
  }
  return {
    ...base,
    '--pa-btn-bg': colors.border,
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border-hover': colors.borderHi,
  } as CSSProperties;
}

/** `Btn ... primary` was the `primary` variant with a wider pad. */
const PRIMARY_ACTION_VARS = {
  '--pa-btn-pad': '6px 16px',
  fontFamily: font.body,
} as CSSProperties;

// D9: transitions ride the spring tokens, not a bare bezier-less "150ms". The
// hand-rolled hover/focus states on this page (RecipeCard, the skill tile —
// anything not built on `Button`, which already carries `.pa-btn`'s own
// spring transition) share one motion string per property list so the same
// state change always settles at the same rate.
const springTransition = (...props: string[]) =>
  props.map(p => `${p} ${duration.fast}ms ${ease.smooth}`).join(', ');

/**
 * The mount guard `Btn` used to release through its `onDone` callback.
 *
 * The guard exists because `onActionDone` refetches, and a refetch can drop the
 * button's own mount condition — "Reset to default" is only rendered while the
 * job is still customized OR its action is in flight. `Btn` released it when
 * its success phase ended; `Button` owns that phase now, so hold the guard for
 * the tick's length and release after, or the button vanishes mid-tick.
 * A failure releases immediately — there is no tick to protect.
 */
function guardedAction(
  key: string,
  work: () => Promise<void>,
  onActionDone: (key: string, afterRefresh?: () => void) => void,
): Promise<boolean> {
  return work().then(
    () => { setTimeout(() => onActionDone(key), SUCCESS_FLASH_MS); return true; },
    () => { onActionDone(key); return false; },
  );
}

// Category badge color per findingCategories.CategoryKey. `colors.warning`
// (amber) stands in for "costly but not urgent"; `review_before_removing`
// gets no alarm color — it is not a threat, just not proven safe.
function categoryColor(category: CategoryKey, colors: ThemeColors): string {
  switch (category) {
    case 'safe_to_remove': return colors.success;
    case 'regenerable_costly': return colors.warning;
    case 'in_use': return colors.danger;
    case 'managed_by_macos': return colors.textMuted;
    case 'review_before_removing': return colors.cyan;
  }
}

// ── Bulk-action wire client (findings) ──────────────────────────────
// A hand-rolled fetch (mirrors dashboard/decisions/client.ts's
// `decisionsFetch`) rather than `apiFetch`, because a 409 here carries a
// `{ error, blocked }` body the caller MUST see — apiFetch's error path
// discards everything but `.message`/`.status`.

export interface BulkActionResult {
  finding_id: string;
  action_taken: string | null;
  size_recovered_bytes: number | null;
  error: string | null;
}

export interface BulkActionBlocked {
  finding_id: string;
  category: string;
  consequence: string | null;
}

interface CategoryAcknowledgments { include_regenerable: boolean; include_review: boolean; profile_paths?: Record<string, string[]> }
interface CargoProfile {
  path: string; bytes: number; status: 'eligible' | 'busy' | 'unsupported' | 'moved' | 'skipped' | 'failed';
  reason: string; moved_bytes: number; moved: number; skipped: number; failed: number;
}
interface Availability {
  finding_id: string; category: string; eligible: boolean; reason: string; protection: string; profiles?: CargoProfile[];
}
interface BulkPreview { findings: Finding[]; availability: Availability[] }

async function previewFindings(runId: string, ids: string[]): Promise<BulkPreview> {
  return apiFetch<BulkPreview>(`/automation/run/${encodeURIComponent(runId)}/bulk-preview?include_regenerable=true&include_review=true&finding_ids=${encodeURIComponent(ids.join(','))}`);
}

export interface BulkActionResponse {
  availability?: Availability[];
  run_id: string;
  results: BulkActionResult[];
  blocked: BulkActionBlocked[];
}

/** What a finished sweep is able to say about itself. The server answers
 *  per item, so the summary line can too — "Cleaning..." with no count was
 *  the same sentence for 2 files and for 2,000. */
export interface BulkSweepSummary {
  cleaned: number;
  failed: number;
  bytes: number;
  profiles?: CargoProfile[];
}

export class BulkActionBlockedError extends Error {
  blocked: BulkActionBlocked[];
  constructor(message: string, blocked: BulkActionBlocked[]) {
    super(message);
    this.name = 'BulkActionBlockedError';
    this.blocked = blocked;
  }
}

async function bulkTrashFindings(runId: string, findingIds: string[], acknowledgments: CategoryAcknowledgments): Promise<BulkActionResponse> {
  const token = await loadDaemonToken();
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const response = await fetch(`${getApiBaseUrl()}/automation/run/${encodeURIComponent(runId)}/bulk-action`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ action: 'trash', finding_ids: findingIds, confirmed: true, action_source: 'ui_bulk', ...acknowledgments }),
  });
  if (response.status === 409) {
    const body = await response.json().catch(() => ({ error: 'Blocked', blocked: [] }));
    throw new BulkActionBlockedError(body.error || 'Some findings cannot be bulk-removed.', body.blocked || []);
  }
  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: 'Unknown error' }));
    throw new Error(error.message || error.error || `HTTP ${response.status}`);
  }
  return response.json();
}

// ── Detail panel types ──────────────────────────────────────────────

type DetailTarget =
  | { kind: 'recipe'; job: ScheduledJob }
  | { kind: 'extension'; ext: ExtensionInfo }
  | { kind: 'run'; run: SessionInfo & { jobId: string }; displayName: string };

// ═══════════════════════════════════════════════════════════════════════
// Main AutomateView — single scrollable page with semantic sections
// ═══════════════════════════════════════════════════════════════════════

export function AutomateView() {
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  const [jobs, setJobs] = useState<ScheduledJob[]>([]);
  const [sessions, setSessions] = useState<Map<string, SessionInfo[]>>(new Map());
  const [extensions, setExtensions] = useState<ExtensionInfo[]>([]);
  const [extensionsError, setExtensionsError] = useState<string | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [completionToast, setCompletionToast] = useState<{ name: string; jobId: string } | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  const [jobsLoading, setJobsLoading] = useState(true);
  const [jobsError, setJobsError] = useState<string | null>(null);
  const [runsLoading, setRunsLoading] = useState(true);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [actionStates, setActionStates] = useState<Record<string, 'loading' | 'success'>>({});
  /** Hash of the proposal whose save failed. `saveProposal` used to swallow its
   *  error and resolve like a success, so the card just sat there. */
  const [proposalError, setProposalError] = useState<string | null>(null);
  const [detail, setDetail] = useState<DetailTarget | null>(null);
  const [search, setSearch] = useState('');
  const [showSearch, setShowSearch] = useState(false);
  const [showInstalledExpanded, setShowInstalledExpanded] = useState(() => {
    try { return localStorage.getItem('automate:installed-expanded') === 'true'; } catch { return false; }
  });
  /** `/api/job-health`; null until it answers (or when it cannot). */
  const [health, setHealth] = useState<JobHealthDigest | null>(null);
  const [healthLoaded, setHealthLoaded] = useState(false);
  const prevRunningRef = useRef<Set<string>>(new Set());
  /** The page's own scroller — SectionNav reads/drives it (A5). */
  const scrollRootRef = useRef<HTMLDivElement>(null);
  const jobsRequestGeneration = useRef(0);
  const { gradient, colors } = useTheme();

  const skills = useCommandCenter(s => s.skills);
  const skillsLoading = useCommandCenter(s => s.skillsLoading);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const proposals = useCommandCenter(s => s.proposals);
  const loadProposals = useCommandCenter(s => s.loadProposals);
  const saveProposal = useCommandCenter(s => s.saveProposal);
  const dismissProposal = useCommandCenter(s => s.dismissProposal);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const setSelectedSkillId = useCommandCenter(s => s.setSelectedSkillId);

  // Open a saved skill in the full Skills Library overlay (master list +
  // SkillDetailPanel) with it preselected — turns the read-only Learned cards
  // into a real path into the library.
  const openSkill = useCallback((id: string) => {
    setSelectedSkillId(id);
    setActivePanel('skills');
  }, [setSelectedSkillId, setActivePanel]);

  // Persist Installed collapse state
  const toggleInstalledExpanded = useCallback((val: boolean) => {
    setShowInstalledExpanded(val);
    try { localStorage.setItem('automate:installed-expanded', String(val)); } catch {}
  }, []);

  // ── Data fetching ──

  const fetchJobs = useCallback(async () => {
    const generation = ++jobsRequestGeneration.current;
    try {
      const data = await apiFetch<{ jobs: ScheduledJob[] }>('/schedule/list');
      if (generation !== jobsRequestGeneration.current) return;
      const newJobs: ScheduledJob[] = data.jobs || [];
      setJobs(newJobs);
      const nowRunning = new Set(newJobs.filter(j => j.currently_running).map(j => j.id));
      const prev = prevRunningRef.current;
      for (const id of prev) {
        if (!nowRunning.has(id)) {
          const name = newJobs.find(j => j.id === id)?.display_name || id;
          // The banner is the gateway to the produced result (Review button) —
          // give it a longer life than a plain "completed" note, and only let
          // the timer clear ITS OWN toast (not a newer completion's).
          setCompletionToast({ name, jobId: id });
          setTimeout(() => setCompletionToast(p => (p?.jobId === id ? null : p)), 30000);
        }
      }
      prevRunningRef.current = nowRunning;
      setJobsError(null);
    } catch (err) {
      if (generation !== jobsRequestGeneration.current) return;
      // Keep any jobs already loaded (a transient poll failure shouldn't blank
      // the list); surface the error so an empty area isn't mistaken for "no
      // automations yet".
      setJobsError(err instanceof Error ? err.message : "Couldn't load automations.");
    } finally {
      if (generation !== jobsRequestGeneration.current) return;
      setJobsLoading(false);
    }
  }, []);

  // Recent runs for EVERY schedule, in ONE request — replaces the old
  // per-job loop (`/schedule/{id}/sessions` called once per job, every poll
  // tick: N queries hitting an unindexed `sessions.schedule_id` scan). See
  // `/schedule/sessions_recent` + `list_recent_sessions_by_schedule` (the
  // 2026-08-25 schedule-polling-storm fix).
  const fetchAllSessions = useCallback(async () => {
    try {
      const data = await apiFetch<{ sessions: Record<string, SessionInfo[]> }>('/schedule/sessions_recent?limit=5');
      setSessions(new Map(Object.entries(data.sessions || {})));
      setRunsError(null);
    } catch (err) {
      setRunsError(err instanceof Error ? err.message : "Couldn't load run history.");
    } finally {
      setRunsLoading(false);
    }
  }, []);

  /**
   * The only bare `catch {}` left on this page — its two siblings above both
   * track their failures and render `SectionState`. Swallowing here produced a
   * specific false sentence: "{agent} has 0 capabilities", which is a claim
   * about the agent when the truth was a claim about the connection. Zero is
   * also the one count that reads as "this feature does nothing", so it is the
   * worst possible thing to guess.
   */
  const fetchExtensions = useCallback(async () => {
    try {
      const data = await apiFetch<{ extensions: ExtensionInfo[]; warnings: string[] }>('/config/extensions');
      setExtensions((data.extensions || []).filter(e => e.enabled));
      setExtensionsError(null);
      return true;
    } catch (e) {
      setExtensionsError(e instanceof Error ? e.message : 'the daemon did not answer');
      // `false` is the Button contract's "it failed" — a retry that could not
      // reach the daemon must not tick as though it had.
      return false;
    }
  }, []);

  // The standing health table — see jobHealthModel.ts. A failed fetch leaves
  // the strip saying "unknown", never green.
  const fetchHealth = useCallback(async () => {
    try {
      setHealth(await apiFetch<JobHealthDigest>('/api/job-health'));
    } catch (err) {
      setHealth({ error: err instanceof Error ? err.message : 'the daemon did not answer' });
    } finally {
      setHealthLoaded(true);
    }
  }, []);

  useEffect(() => {
    fetchJobs();
    fetchAllSessions();
    fetchHealth();
    fetchExtensions();
    loadSkills();
    loadProposals();
  }, [fetchJobs, fetchAllSessions, fetchHealth, fetchExtensions, loadSkills, loadProposals]);

  useEffect(() => () => { ++jobsRequestGeneration.current; }, []);

  // Live updates: a real schedule write (create/update/delete/pause/unpause/
  // run start/finish) pushes a `schedule_changed` event over the daemon's
  // event stream — refetch both the job list and the run-history summary
  // immediately instead of waiting for the poll backstop below.
  const onScheduleEvent = useCallback(() => {
    fetchJobs();
    fetchAllSessions();
    fetchHealth();
  }, [fetchJobs, fetchAllSessions, fetchHealth]);
  useScheduleEvents(onScheduleEvent);

  // Backstop only — see `SCHEDULE_LIST_POLL_MS`. Skips entirely while the
  // tab is hidden (`usePollWhenVisible`), and stops on unmount.
  usePollWhenVisible(fetchJobs, SCHEDULE_LIST_POLL_MS);
  usePollWhenVisible(fetchAllSessions, SCHEDULE_LIST_POLL_MS);
  usePollWhenVisible(fetchHealth, SCHEDULE_LIST_POLL_MS);

  // ── Derived data ──

  const allRuns = useMemo(() =>
    Array.from(sessions.entries())
      .flatMap(([jobId, runs]) => runs.map(r => ({ ...r, jobId })))
      .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime())
      .slice(0, 10),
    [sessions],
  );
  const jobNameMap = new Map(jobs.map(j => [j.id, j.display_name || j.id]));

  // ── Search filter ──

  const q = search.toLowerCase().trim();
  const filteredJobs = q ? jobs.filter(j => (j.display_name || j.id).toLowerCase().includes(q) || (j.description || '').toLowerCase().includes(q)) : jobs;
  const filteredSkills = q ? skills.filter(s => s.name.toLowerCase().includes(q) || (s.description || '').toLowerCase().includes(q)) : skills;
  const filteredProposals = q ? proposals.filter(p => p.description.toLowerCase().includes(q)) : proposals;
  const filteredExtensions = q ? extensions.filter(e => e.display_name.toLowerCase().includes(q) || e.description.toLowerCase().includes(q)) : extensions;

  // A6: the standing health line at the top of the page — see jobHealthModel.
  // Recomputed from the same `jobs` list the cards use, so a resume that
  // clears a card's ERROR chip clears its health-strip entry in the same
  // render (both read `lastFailure`, not a second copy of the rule).
  const healthSummary = useMemo(
    () => healthLoaded ? summarizeHealth(health, jobs) : null,
    [health, healthLoaded, jobs],
  );
  const jobIds = useMemo(() => new Set(jobs.map(j => j.id)), [jobs]);

  // ── Action state tracking (in-button feedback) ──

  const setActionState = useCallback((key: string, state: 'loading' | 'success' | null) => {
    setActionStates(prev => {
      if (state === null) { const next = { ...prev }; delete next[key]; return next; }
      return { ...prev, [key]: state };
    });
  }, []);

  // ── Derive detail panel job from current jobs state ──

  const currentDetail = useMemo<DetailTarget | null>(() => {
    if (!detail) return null;
    if (detail.kind === 'recipe') {
      const fresh = jobs.find(j => j.id === detail.job.id);
      return fresh ? { ...detail, job: fresh } : detail;
    }
    return detail;
  }, [detail, jobs]);

  // ── Actions ──
  // Handlers set actionStates as a mount guard (prevents polling from unmounting
  // the button mid-animation), then call the API. The visual feedback — spinner
  // for the round trip, tick on success — belongs to the `Button` primitive:
  // every one of these REJECTS on failure, which is how it knows not to tick.

  const handleRunNow = async (id: string) => {
    setRunError(null);
    // The POST is held open for the whole run; currently_running flips server-side
    // immediately, so poll early (rather than waiting up to 5s) to resolve the
    // optimistic "Starting..." into the poll-derived "Running Now" state promptly.
    const earlyPoll = setTimeout(() => { fetchJobs(); }, 1000);
    try {
      await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/run_now`, { method: 'POST' });
    } catch (err) {
      clearTimeout(earlyPoll);
      const name = jobNameMap.get(id) || id;
      const msg = err instanceof Error ? err.message : String(err);
      setRunError(`Couldn't run "${name}": ${msg}`);
      setTimeout(() => setRunError(null), 8000);
      throw err; // reject so the button drops its spinner without ticking
    } finally {
      fetchJobs();
    }
  };
  // Shared failure surface for the schedule mutations (2026-07 wiring audit):
  // the old bare `catch {}` made every failed POST look like success — a
  // resolved promise flashed "✓ Paused/Deleted/…" even on a 4xx/5xx. Show the
  // failure on the same banner Run Now uses, and RE-THROW so the button reverts
  // to idle instead of faking success.
  const failAction = (id: string, verb: string, err: unknown): never => {
    const name = jobNameMap.get(id) || id;
    const msg = err instanceof Error ? err.message : String(err);
    setRunError(`Couldn't ${verb} "${name}": ${msg}`);
    setTimeout(() => setRunError(null), 8000);
    throw err;
  };
  // Pause/Resume refetch the list the moment the daemon says yes. They used to
  // wait for `guardedAction`'s post-tick refresh (and, whenever the card's own
  // button unmounted first, for the next event or poll), so a resumed card
  // could keep saying PAUSED for seconds after a 204 (QA 2026-09-24). The
  // flipped label and the swapped Pause/Resume button are the confirmation.
  const handlePause = async (id: string) => {
    setActionState(`${id}:pause`, 'loading');
    try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/pause`, { method: 'POST' }); }
    catch (err) { failAction(id, 'pause', err); }
    void fetchJobs();
  };
  const handleUnpause = async (id: string) => {
    setActionState(`${id}:unpause`, 'loading');
    try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/unpause`, { method: 'POST' }); }
    catch (err) { failAction(id, 'resume', err); }
    void fetchJobs();
  };
  // Deleting an automation is confirmed on the row itself (see
  // `DeleteAutomationControl`), so this only ever runs after the user has said
  // yes in place. It deliberately does NOT go through `failAction`: the
  // control that asked the question is where the answer belongs, and it holds
  // the failure until the user retries or backs out rather than for eight
  // seconds on a banner at the top of the page. Rejecting is the signal.
  const handleDelete = async (id: string) => {
    await apiFetch<unknown>(`/schedule/delete/${encodeURIComponent(id)}`, { method: 'DELETE' });
    await fetchJobs();
  };
  const handleKill = async (id: string) => {
    try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/kill`, { method: 'POST' }); fetchJobs(); }
    catch (err) { failAction(id, 'stop', err); }
  };
  const handleResetToDefault = async (id: string) => {
    setActionState(`${id}:reset`, 'loading');
    try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/reset_to_default`, { method: 'POST' }); }
    catch (err) { failAction(id, 'reset', err); }
  };

  // Releases the mount guard and refreshes — see `guardedAction`, which holds it
  // for the length of the primitive's success tick before calling this.
  // "Review result" on the completion banner: jump straight to the finished
  // run's detail (findings / report) without scrolling to Recent Activity.
  const reviewCompleted = useCallback(async (jobId: string, name: string) => {
    try {
      const data = await apiFetch<SessionInfo[]>(`/schedule/${encodeURIComponent(jobId)}/sessions?limit=1`);
      const run = data[0];
      if (run) {
        setDetail({ kind: 'run', run: { ...run, jobId }, displayName: name });
        setCompletionToast(null);
        return true;
      }
      return false;
    } catch {
      // Keep the banner — the Recent Activity row remains the fallback path.
      // `false` also keeps the button from ticking on a lookup that failed.
      return false;
    }
  }, []);

  // UX_BENCHMARK_RESEARCH §4: "one click to that run's output" — from the
  // schedule row, from its failure alert, or from the detail panel's error
  // box, all the same target. Prefers the cached `sessions` map (already
  // fetched for Recent Activity); falls back to a live lookup for a job whose
  // last run fell outside that map's 5-per-job window.
  const openLastRun = useCallback(async (jobId: string) => {
    const name = jobNameMap.get(jobId) || jobId;
    const cached = sessions.get(jobId)?.[0];
    if (cached) { setDetail({ kind: 'run', run: { ...cached, jobId }, displayName: name }); return; }
    try {
      const data = await apiFetch<SessionInfo[]>(`/schedule/${encodeURIComponent(jobId)}/sessions?limit=1`);
      const run = data[0];
      if (run) setDetail({ kind: 'run', run: { ...run, jobId }, displayName: name });
    } catch {
      // No run to show (or the daemon didn't answer) — leave the current
      // panel open rather than navigating to nothing.
    }
  }, [sessions, jobNameMap]);

  const handleActionDone = useCallback((key: string, afterRefresh?: () => void) => {
    setActionState(key, null);
    fetchJobs().then(() => afterRefresh?.());
  }, [setActionState, fetchJobs]);

  // ── Truly-empty check ──
  // Only "truly empty" once loads have settled with no error — otherwise the
  // invitation would flash over a loading state or hide a failed fetch.
  const trulyEmpty = jobs.length === 0 && skills.length === 0 && proposals.length === 0
    && !skillsLoading && !jobsLoading && !jobsError;

  // A5: the jump bar only lists sections that actually render below, in the
  // same order and under the same condition as the JSX — so it never offers a
  // jump to a section that isn't there.
  const sectionNavItems: SectionNavItem[] = [
    ...(jobs.length > 0 ? [{ id: 'recent-activity', label: 'Recent Activity', count: allRuns.length }] : []),
    ...(!trulyEmpty ? [{ id: 'automations', label: AUTOMATION.title, count: filteredJobs.length }] : []),
    { id: 'learned', label: 'Learned', count: filteredSkills.length + filteredProposals.length },
    { id: 'installed', label: 'Installed', count: filteredExtensions.length },
  ];

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>

      {/* ── Main column: fixed header + scrolling content ──
          The header used to live INSIDE the scroll container and slid out of
          view as you scrolled, taking the view's identity and its Create button
          with it. It is now a fixed sibling above the scroller, matching every
          other view. */}
      <div style={{ flex: 1, minWidth: 0, height: '100%', display: 'flex', flexDirection: 'column' }}>
        <ViewHeader
          title="Automate"
          actions={<>
            {showSearch && (
              <input
                autoFocus
                id="automate-filter"
                value={search}
                onChange={e => setSearch(e.target.value)}
                placeholder="Filter..."
                style={{
                  width: 180, padding: `${space.xs}px ${space.lg}px`, borderRadius: radius.sm,
                  background: colors.surface, border: `1px solid ${colors.border}`,
                  color: colors.text, fontSize: textSize.caption, fontFamily: font.mono, outline: 'none',
                }}
                onKeyDown={e => { if (e.key === 'Escape') { setSearch(''); setShowSearch(false); } }}
              />
            )}
            {/* Disclosure, not action: its whole job is to show/hide the filter
                box beside it, so it keeps the element (and the aria-expanded /
                aria-controls pairing that describes it) and takes only the
                shared `.pa-btn` interaction rules. */}
            <button
              type="button"
              className="pa-btn"
              aria-expanded={showSearch}
              aria-controls="automate-filter"
              aria-label={`Filter ${AUTOMATION.many}`}
              onClick={() => setShowSearch(!showSearch)}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': colors.border,
                '--pa-btn-pad': '4px',
                '--pa-btn-radius': `${radius.xs}px`,
              } as CSSProperties}
            >
              <FiSearch size={16} />
            </button>
            <Button
              colors={colors}
              variant="primary"
              onClick={() => setShowModal(true)}
              style={{
                '--pa-btn-pad': '6px 14px',
                '--pa-btn-radius': `${radius.md}px`,
                fontFamily: font.body,
                fontSize: textSize.caption,
              } as CSSProperties}
            >+ Create {AUTOMATION.one}</Button>
          </>}
        />

        <div ref={scrollRootRef} style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: `${space.xxxl}px 32px 40px` }}>

        {/* A5: sticky jump bar — the page is one long scroll, and Learned was
            reachable only by scrolling past everything above it. */}
        <SectionNav items={sectionNavItems} scrollRoot={scrollRootRef} />

        {/* A6: one line up top — "everything is running", or which jobs need
            attention (failing / missed / never run / wedged / undelivered).
            `null` while the first fetch is in flight (R2 loading state is the
            strip's own absence, not a spinner). */}
        <JobHealthStrip
          summary={healthSummary}
          onOpen={id => { const job = jobs.find(j => j.id === id); if (job) setDetail({ kind: 'recipe', job }); }}
          canOpen={id => jobIds.has(id)}
        />

        {/* Live run roster — everything at work right now, at a glance.
            R7: this is now the ONLY place a running schedule's card lives —
            a row opens the same detail panel "Running Now" used to. */}
        <RunRoster schedules={jobs} onOpenSchedule={id => { const job = jobs.find(j => j.id === id); if (job) setDetail({ kind: 'recipe', job }); }} />

        {/* Run completion banner — the no-scroll path to the produced result:
            "Review result" opens the run detail (findings / report) directly. */}
        {completionToast && (
          <div style={{
            marginBottom: space.xxl, padding: `${space.lg}px ${space.xxl}px`, borderRadius: radius.md,
            background: withAlpha(colors.success, 0.1), border: `1px solid ${withAlpha(colors.success, 0.25)}`,
            display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl,
          }}>
            <span style={{ fontSize: textSize.small, color: colors.success, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              &#10003; "{completionToast.name}" completed.
            </span>
            <Button
              colors={colors}
              variant="primary"
              onClick={() => reviewCompleted(completionToast.jobId, completionToast.name)}
              style={{
                '--pa-btn-bg': colors.success,
                '--pa-btn-bg-hover': colors.success,
                '--pa-btn-bg-active': colors.success,
                '--pa-btn-pad': '5px 12px',
                '--pa-btn-radius': `${radius.md}px`,
                fontFamily: font.body,
                fontSize: textSize.caption,
                flexShrink: 0,
              } as CSSProperties}
            >Review result</Button>
            <Button
              colors={colors}
              variant="bare"
              onClick={() => setCompletionToast(null)}
              aria-label="Dismiss this notice"
              style={{
                '--pa-btn-fg': colors.textDim,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                fontSize: textSize.heading,
                lineHeight: 1.5,
              } as CSSProperties}
            >&times;</Button>
          </div>
        )}

        {/* Run-now failure banner (surfaced, not swallowed) */}
        {runError && (
          <div style={{
            marginBottom: space.xxl, padding: `${space.lg}px ${space.xxl}px`, borderRadius: radius.md,
            background: withAlpha(colors.danger, 0.1), border: `1px solid ${withAlpha(colors.danger, 0.25)}`,
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          }}>
            <span style={{ fontSize: textSize.small, color: colors.danger }}>{runError}</span>
            <Button
              colors={colors}
              variant="bare"
              onClick={() => setRunError(null)}
              aria-label="Dismiss this error"
              style={{
                '--pa-btn-fg': colors.textDim,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                fontSize: textSize.heading,
                lineHeight: 1.5,
              } as CSSProperties}
            >&times;</Button>
          </div>
        )}

        {/* Truly-empty invitation */}
        {trulyEmpty && jobs.length === 0 && (
          <div style={{
            padding: '32px 28px', borderRadius: radius.lg, marginBottom: space.huge,
            background: colors.surface, border: `1px solid ${colors.border}`, textAlign: 'center',
          }}>
            <div style={{ fontSize: textSize.body, fontWeight: 600, fontFamily: font.display, marginBottom: space.md }}>
              Your agent can do a lot already
            </div>
            <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.6, maxWidth: 420, margin: '0 auto' }}>
              Try asking {agentName} to summarize your Downloads folder — if you like the result,
              ask to remember how. Skills appear here when you save them.
            </div>
            <div style={{ marginTop: space.xxl, display: 'flex', gap: space.xl, justifyContent: 'center' }}>
              <Button
                colors={colors}
                variant="primary"
                onClick={() => setShowModal(true)}
                style={{
                  '--pa-btn-pad': '8px 20px',
                  '--pa-btn-radius': `${radius.md}px`,
                  fontFamily: font.body,
                  fontSize: textSize.small,
                } as CSSProperties}
              >Schedule a task</Button>
            </div>
            <Button
              colors={colors}
              variant="bare"
              onClick={() => toggleInstalledExpanded(true)}
              style={{
                '--pa-btn-fg': colors.textDim,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                fontFamily: font.body,
                fontSize: textSize.micro,
                marginTop: space.xl,
              } as CSSProperties}
            >or browse what your agent can do &rarr;</Button>
          </div>
        )}

        {/* ── RECENT ACTIVITY ── (above Recipes: a finished run's produced
            result is the thing most likely to need a click — confirm/reject —
            so it must be reachable without scrolling.) */}
        {jobs.length > 0 && (
          <Section id="recent-activity" title="Recent Activity" count={allRuns.length}>
            {runsLoading && allRuns.length === 0 ? (
              <StateBlock tone="loading" title="Loading run history…" compact />
            ) : runsError && allRuns.length === 0 ? (
              <StateBlock tone="error" title={runsError!} onRetry={() => { setRunsLoading(true); fetchAllSessions(); }} compact />
            ) : allRuns.length === 0 ? (
              <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '8px 0' }}>
                No runs yet. History appears here once an {AUTOMATION.one} runs — use "Run Now" on one to try it.
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxs }}>
                {allRuns.map(run => {
                  const displayName = jobNameMap.get(run.jobId) || run.jobId;
                  return (
                    /* Keeps the element, takes the rules: this row IS the button
                       and its four pieces are flex children OF it (a `flex: 1`
                       name that truncates, meta pushed right). `Button` folds
                       its children into one `pa-btn__label` span, which would
                       collapse that distribution — so this gets `.pa-btn` the
                       way FinanceView's PickRow does. The hand-rolled hover goes
                       to `--pa-btn-bg-hover`, and the hand-rolled focus paint
                       goes to the global `:focus-visible` ring in index.css. */
                    <button
                      key={run.id}
                      type="button"
                      className="pa-btn pa-btn--composite"
                      onClick={() => setDetail({ kind: 'run', run, displayName })}
                      style={{
                        '--pa-btn-fg': colors.text,
                        '--pa-btn-bg-hover': colors.border,
                        '--pa-btn-pad': '8px 12px',
                        '--pa-btn-radius': `${radius.sm}px`,
                        fontFamily: font.body,
                        gap: space.lg,
                        width: '100%',
                        justifyContent: 'flex-start',
                        textAlign: 'left',
                      } as CSSProperties}
                    >
                      <span style={{ fontSize: textSize.micro, color: colors.success }}>&#10003;</span>
                      <span style={{ fontSize: textSize.caption, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{displayName}</span>
                      <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono, flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>{run.messageCount} msgs</span>
                      <span style={{ fontSize: 10, color: colors.textDim, flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>{timeAgo(run.createdAt)}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </Section>
        )}

        {/* ── AUTOMATIONS ── (the word is the tab's own: this header said
            "Recipes" while the button beside it said Create Automation, the
            modal said New Automation and the delete said "Delete automation".
            "Recipe" is the internal type's name — `RecipeCard`, the
            `kind: 'recipe'` detail tag — and it stays there.) */}
        {!trulyEmpty && (
          <Section title={AUTOMATION.title} count={filteredJobs.length} id="automations">
            {jobsLoading && jobs.length === 0 ? (
              <StateBlock tone="loading" title="Loading automations…" compact />
            ) : jobsError && jobs.length === 0 ? (
              <StateBlock tone="error" title={`Couldn't load automations. ${jobsError}`} onRetry={() => { setJobsLoading(true); fetchJobs(); }} compact />
            ) : filteredJobs.length === 0 ? (
              <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '8px 0' }}>
                {q
                  ? `No ${AUTOMATION.many} match "${search.trim()}".`
                  : `No ${AUTOMATION.many} yet. Click "+ Create ${AUTOMATION.one}" to schedule your first one.`}
              </div>
            ) : (
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: space.xl }}>
                {filteredJobs.filter(j => !j.currently_running).map(job => (
                  <RecipeCard key={job.id} job={job} onRunNow={handleRunNow} onPause={handlePause}
                    onUnpause={handleUnpause} onDelete={handleDelete} onKill={handleKill}
                    onResetToDefault={handleResetToDefault} actionStates={actionStates} onActionDone={handleActionDone}
                    onSelect={() => setDetail({ kind: 'recipe', job })}
                    onOpenLastRun={openLastRun}
                    selected={detail?.kind === 'recipe' && detail.job.id === job.id} />
                ))}
              </div>
            )}
          </Section>
        )}

        {/* ── LEARNED ── */}
        <Section id="learned" title="Learned" count={filteredSkills.length + filteredProposals.length}>
          {skillsLoading ? (
            <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Loading...</div>
          ) : filteredSkills.length === 0 && filteredProposals.length === 0 ? (
            <div style={{
              padding: `${space.xxxl}px ${space.huge}px`, borderRadius: radius.lg,
              background: withAlpha(colors.purple, 0.04), border: `1px solid ${withAlpha(colors.purple, 0.12)}`,
            }}>
              <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.6 }}>
                When you repeat tasks, {agentName} notices patterns and offers to save them.
                Your first skill will appear here.
              </div>
              <div style={{ fontSize: textSize.caption, color: colors.textDim, marginTop: space.md }}>
                Try asking {agentName} to do something twice — like "summarize my Downloads folder"
                — and you'll be offered a way to remember how.
              </div>
            </div>
          ) : (
            <LearnedList
              skills={filteredSkills}
              proposals={filteredProposals}
              onOpenSkill={openSkill}
              onSaveProposal={async proposal => {
                // `saveProposal` resolves `false` on failure, so the tick is
                // honest and a rejected save says so on its card.
                setProposalError(null);
                const ok = await saveProposal(proposal);
                if (!ok) setProposalError(proposal.argument_shape_hash);
                return ok;
              }}
              onDismissProposal={dismissProposal}
              proposalError={proposalError}
            />
          )}
        </Section>

        {/* ── INSTALLED ── */}
        <Section id="installed" title="Installed" count={filteredExtensions.length} collapsed>
          {extensionsError ? (
            <StateBlock
              tone="error"
              title={`Couldn't read what ${agentName} can do. ${extensionsError}`}
              onRetry={fetchExtensions}
              compact
            />
          ) : !showInstalledExpanded ? (
            /* Disclosure pair (this and "Hide ↑" below): both do nothing but
               open/close the capability list, so they keep the element plus the
               aria-expanded that names the state, and take the shared `.pa-btn`
               interaction rules rather than the pending/success machinery. */
            <button
              type="button"
              className="pa-btn"
              aria-expanded={false}
              onClick={() => toggleInstalledExpanded(true)}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '4px 0',
                fontSize: textSize.caption,
                fontFamily: font.body,
                textAlign: 'left',
              } as CSSProperties}
            >
              {agentName} has {extensions.length} capabilities &middot; <span style={{ color: colors.cyan }}>Show what your agent can do &rarr;</span>
            </button>
          ) : (
            <>
              <button
                type="button"
                className="pa-btn"
                aria-expanded
                aria-controls="automate-installed-list"
                onClick={() => toggleInstalledExpanded(false)}
                style={{
                  '--pa-btn-fg': colors.textDim,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-pad': '0 0 8px',
                  fontSize: textSize.micro,
                  fontFamily: font.body,
                  textAlign: 'left',
                } as CSSProperties}
              >Hide &uarr;</button>
              <div id="automate-installed-list" style={{ display: 'flex', flexWrap: 'wrap', gap: space.md }}>
                {filteredExtensions.map(ext => {
                  const isSelected = detail?.kind === 'extension' && detail.ext.name === ext.name;
                  return (
                    <Button
                      key={ext.name}
                      colors={colors}
                      variant={isSelected ? 'ghostOn' : 'ghost'}
                      aria-pressed={isSelected}
                      onClick={() => setDetail({ kind: 'extension', ext })}
                      style={{
                        '--pa-btn-bg': isSelected ? colors.cyanSoft : colors.surface,
                        '--pa-btn-fg': colors.text,
                        '--pa-btn-border': isSelected ? colors.borderHi : colors.border,
                        '--pa-btn-bg-hover': isSelected ? colors.cyanSoft : colors.surfaceHi,
                        '--pa-btn-border-hover': colors.borderHi,
                        '--pa-btn-pad': '8px 14px',
                        '--pa-btn-radius': `${radius.md}px`,
                        '--pa-btn-weight': 400,
                        fontFamily: font.body,
                        fontSize: textSize.caption,
                        // Two stacked lines, so the chip's height is real: keep
                        // the leading the raw button inherited rather than
                        // `.pa-btn`'s 14px, which would close the gap.
                        lineHeight: 1.5,
                        textAlign: 'left',
                      } as CSSProperties}
                    >
                      <div style={{ fontWeight: 600, marginBottom: space.xxs }}>{ext.display_name}</div>
                      <div style={{ fontSize: 10, color: colors.textDim, maxWidth: 220, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{ext.description}</div>
                    </Button>
                  );
                })}
              </div>
            </>
          )}
        </Section>

        </div>
      </div>

      {/* ── Detail panel (slides in from right) ── */}
      {currentDetail && (
        <DetailPanel detail={currentDetail}
          lastStatus={currentDetail.kind === 'run' && sessions.get(currentDetail.run.jobId)?.[0]?.id === currentDetail.run.id
            ? jobs.find(job => job.id === currentDetail.run.jobId)?.last_status : undefined}
          starterId={currentDetail.kind === 'run' ? jobs.find(job => job.id === currentDetail.run.jobId)?.starter_id : undefined}
          onClose={() => setDetail(null)}
          onRunNow={handleRunNow} onPause={handlePause} onUnpause={handleUnpause}
          onDelete={handleDelete} onKill={handleKill} onResetToDefault={handleResetToDefault}
          onOpenLastRun={openLastRun}
          actionStates={actionStates} onActionDone={handleActionDone} />
      )}

      {showModal && <NewAutomationModal onClose={() => setShowModal(false)} onCreated={() => { setShowModal(false); fetchJobs(); }} />}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Section header
// ═══════════════════════════════════════════════════════════════════════

function Section({ id, title, count, accentColor, collapsed, children }: {
  /** A5: matched by SectionNav's jump targets — see `data-section` below. */
  id?: string;
  title: string; count: number; accentColor?: string; collapsed?: boolean;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <div
      data-section={id}
      style={{ marginBottom: 28, scrollMarginTop: id ? SECTION_NAV_OFFSET : undefined }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md, marginBottom: space.xl }}>
        <div style={{ fontSize: 10, fontWeight: 700, fontFamily: font.mono, letterSpacing: '0.08em', textTransform: 'uppercase', color: accentColor || colors.textDim }}>
          {title}
        </div>
        {/* R6: the one count-chip style, via the shared primitive. */}
        {!collapsed && count > 0 && <Chip kind="count">{count}</Chip>}
        <div style={{ flex: 1, height: 1, background: colors.border }} />
      </div>
      {children}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Recipe card (for Recipes + Running Now sections)
// ═══════════════════════════════════════════════════════════════════════

function RecipeCard({
job, onRunNow, onPause, onUnpause, onDelete, onKill, onResetToDefault, actionStates, onActionDone, onSelect, onOpenLastRun, selected }: {
  // Every one of these is an async handler that REJECTS on failure (see
  // `failAction`), and the button primitive reads that rejection: it is what
  // holds the spinner for the round trip and withholds the success tick when
  // the daemon says no. Typing them `void` hid the contract.
  job: ScheduledJob;
  onRunNow: (id: string) => Promise<void>;
  onPause: (id: string) => Promise<void>;
  onUnpause: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onKill: (id: string) => Promise<void>;
  onResetToDefault: (id: string) => Promise<void>;
  actionStates: Record<string, 'loading' | 'success'>;
  onActionDone: (key: string, afterRefresh?: () => void) => void;
  onSelect: () => void;
  /** Opens the job's most recent run (its output) — see UX_BENCHMARK_RESEARCH
   *  §4: an unfixed failure is one click from its run, not from the schedule. */
  onOpenLastRun: (jobId: string) => void;
  selected: boolean;
}) {
  const { colors } = useTheme();
  const name = job.display_name || job.id;
  const desc = job.description || '';
  const schedule = scheduleSummary(job);
  const cronHint = rawCronTooltip(job);
  const statusColor = job.currently_running ? colors.success : job.paused ? colors.textDim : colors.cyan;
  const hasUpdate = job.starter_id && job.user_customized && job.embedded_version && job.version !== job.embedded_version;
  const runStatus = runChip(job);
  const failure = lastFailure(job);
  const nextRun = failure.kind !== 'historical' ? nextRunLabel(job.next_run) : null;
  // A failing run that nobody has looked at yet is a standing alert, not a
  // dot that clears itself on refresh — it stays flagged, across reloads,
  // until it's actually opened (see scheduleHealth's ack helpers).
  const [ackTick, setAckTick] = useState(0);
  // `ackTick` forces this to re-read localStorage right after `activate()`
  // acknowledges the failure, so the same click that opens the run also
  // drops the card out of its "needs attention" state.
  const unacknowledged = ackTick >= 0 && failure.kind === 'current' && !isFailureAcknowledged(job.id, failure);
  // While the failure sits unacknowledged, the card's own click/Enter opens
  // the failing run (and acknowledges it) instead of the schedule detail —
  // "one click to the run that needs attention", same shape as a Recent
  // Activity row. Acknowledging flips it back to the ordinary card the next
  // render, so a second click opens the detail like any other row.
  const activate = () => {
    if (unacknowledged) {
      acknowledgeFailure(job.id, failure);
      setAckTick(t => t + 1);
      onOpenLastRun(job.id);
    } else {
      onSelect();
    }
  };

  return (
    <div
      onClick={activate}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      aria-label={unacknowledged ? `${name} failed — open the failing run` : `Open details for ${name}`}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); activate(); } }}
      // D10: hover/press get a real fill, not just the border tint the card
      // already had — `selected`'s own cyan tint takes priority and is never
      // overwritten by the pointer state.
      onMouseEnter={e => { if (!selected) { e.currentTarget.style.borderColor = colors.borderHi; e.currentTarget.style.background = colors.fillHover; } }}
      onMouseLeave={e => { if (!selected) { e.currentTarget.style.borderColor = colors.border; e.currentTarget.style.background = colors.surface; } }}
      onMouseDown={e => { if (!selected) e.currentTarget.style.background = colors.fillActive; }}
      onMouseUp={e => { if (!selected) e.currentTarget.style.background = colors.fillHover; }}
      onFocus={e => { if (!selected) e.currentTarget.style.borderColor = colors.borderHi; e.currentTarget.style.boxShadow = `0 0 0 2px ${colors.cyanGlow}`; }}
      onBlur={e => { if (!selected) e.currentTarget.style.borderColor = colors.border; e.currentTarget.style.boxShadow = 'none'; }}
      style={{
      padding: `${space.xxl}px ${space.xxxl}px`, borderRadius: radius.lg, cursor: 'pointer', outline: 'none',
      background: selected ? withAlpha(colors.cyan, 0.04) : colors.surface,
      border: `1px solid ${selected ? colors.borderHi : colors.border}`,
      transition: springTransition('border-color', 'background', 'box-shadow'),
    }}>
      {/* A1: the title owns its row. It shared one line with three chips and
          lost — "Nightly …", "Moni…" at 1456px. It now wraps to two lines and
          the chips sit on a meta row beneath it. */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: space.lg, marginBottom: space.sm }}>
        <div style={{
          width: 8, height: 8, borderRadius: '50%', background: statusColor, flexShrink: 0, marginTop: space.sm,
          boxShadow: job.currently_running ? `0 0 8px ${statusColor}` : 'none',
        }} />
        <div
          data-testid="automation-title"
          style={{ fontSize: textSize.body, fontWeight: 600, fontFamily: font.display, flex: 1, minWidth: 0, lineHeight: 1.35, overflow: 'hidden', overflowWrap: 'anywhere', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' } as CSSProperties}
        >{name}</div>
      </div>
      {/* R6: one chip primitive for state/status/version, via the shared
          `Chip` — this used to be three hand-rolled `<span>`s with their own
          colour math and no hover/focus affordance on the ones that had a
          reason ("why did it fail?") to explain. */}
      <div data-testid="automation-chips" style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.sm, marginBottom: space.md }}>
        <Chip kind="state" tone="accent" pulse={job.currently_running} data-testid="automation-state-chip">
          {job.currently_running ? 'RUNNING' : job.paused ? 'PAUSED' : 'SCHEDULED'}
        </Chip>
        {!job.currently_running && runStatus && (
          <Chip
            kind="state"
            tone={runStatus.token === 'textDim' ? 'neutral' : runStatus.token}
            title={runStatus.hint}
            data-testid="automation-run-chip"
          >{runStatus.label}</Chip>
        )}
        {job.version && (
          <Chip kind="static" tone="neutral" quiet>v{job.version}</Chip>
        )}
        {unacknowledged && (
          // Persistent, not a dot: stays until the run is actually opened —
          // clicking anywhere on the card does the same thing (`activate`).
          <Chip kind="link" tone="danger" onClick={activate} data-testid="automation-needs-attention">
            Needs attention — view run
          </Chip>
        )}
      </div>
      {desc && <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5, marginBottom: space.md, overflow: 'hidden', textOverflow: 'ellipsis', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' } as React.CSSProperties}>{desc}</div>}
      <div style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono, marginBottom: failure.kind === 'historical' ? space.xs : space.lg }}>
        {cronHint ? (
          <Tooltip content={cronHint}><span tabIndex={0} style={{ outline: 'none' }}>{schedule}</span></Tooltip>
        ) : schedule}
        {' '}&middot; {runStateWords(job)}
        {!!job.run_count && <> &middot; {job.run_count} run{job.run_count === 1 ? '' : 's'}</>}
        {/* Benchmark research §4: "next run" is a column of its own, not
            buried in the detail panel — shown here whenever the daemon sends
            one and the row isn't already covering it in the historical note. */}
        {nextRun && <> &middot; {nextRun}</>}
        {hasUpdate && <> &middot; <span style={{ color: colors.warning }}>Update available</span></>}
      </div>
      {failure.kind === 'historical' && (
        <div data-testid="automation-failure-history" style={{ fontSize: 10, color: colors.textMuted, fontFamily: font.body, lineHeight: 1.4, marginBottom: space.lg }}>
          {historicalFailureNote(failure, job, schedule)}
        </div>
      )}
      <div style={{ display: 'flex', gap: space.sm, flexWrap: 'wrap' }} onClick={e => e.stopPropagation()}>
        {job.currently_running ? (
          // No tick on Stop: the run leaving "RUNNING" is the confirmation.
          <Button colors={colors} onClick={() => onKill(job.id)} flashSuccess={false} style={actionVars(colors, 'danger')}>Stop</Button>
        ) : (
          <>
            <Button colors={colors} variant="primary" onClick={() => onRunNow(job.id)} style={PRIMARY_ACTION_VARS}>Run Now</Button>
            {job.paused
              ? <Button colors={colors} onClick={() => guardedAction(`${job.id}:unpause`, () => onUnpause(job.id), onActionDone)} style={actionVars(colors)}>Resume</Button>
              : <Button colors={colors} onClick={() => guardedAction(`${job.id}:pause`, () => onPause(job.id), onActionDone)} style={actionVars(colors)}>Pause</Button>}
          </>
        )}
        {((job.user_customized && job.starter_id) || actionStates[`${job.id}:reset`]) && (
          <Button colors={colors} onClick={() => guardedAction(`${job.id}:reset`, () => onResetToDefault(job.id), onActionDone)} style={actionVars(colors)}>Reset to default</Button>
        )}
        <DeleteAutomationControl name={name} onDelete={() => onDelete(job.id)} />
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Detail panel (right slide-in)
// ═══════════════════════════════════════════════════════════════════════

function DetailPanel({
detail, lastStatus, starterId, onClose, onRunNow, onPause, onUnpause, onDelete, onKill, onResetToDefault, onOpenLastRun, actionStates, onActionDone }: {
  detail: DetailTarget;
  lastStatus?: ScheduleRunStatus | null;
  /** The run's job's starter (a run carries only its job id). */
  starterId?: string | null;
  onClose: () => void;
  onRunNow: (id: string) => Promise<void>;
  onPause: (id: string) => Promise<void>;
  onUnpause: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onKill: (id: string) => Promise<void>;
  onResetToDefault: (id: string) => Promise<void>;
  onOpenLastRun: (jobId: string) => void;
  actionStates: Record<string, 'loading' | 'success'>;
  onActionDone: (key: string, afterRefresh?: () => void) => void;
}) {
  const { colors } = useTheme();
  // D1/D6: this rail is the one surface on the screen that earns Liquid Glass
  // — Apple's inspector, verbatim: "edge-to-edge glass that sits alongside
  // the content" (HIG sidebars, §1.7). It floats above the same content it
  // sits beside, so it takes `glassHi` (the large-surface, more-opaque
  // variant) rather than the default `glass`. It is edge-to-edge (flush to
  // the window's top/right/bottom, only the left edge borders content), so it
  // carries no radius — a rounded floating card is the wrong shape here, not
  // a corner-nesting case `concentric()` has anything to derive.
  const material = useGlass('glassHi');
  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    // Spread, not named — `backdropFilter`/`WebkitBackdropFilter` are written
    // ONLY inside `<Glass>`/`glassSurface()` (backdropFilter.test.ts). Naming
    // the keys here would be exactly the hand-rolled glass this lane exists
    // to remove, even though the values come from the token.
    <div style={{
      width: '50%', minWidth: 480, maxWidth: 640, height: '100%',
      ...material,
      borderLeft: `1px solid ${colors.borderHi}`,
      overflowY: 'auto', padding: `${space.xxxl}px ${space.huge}px`, flexShrink: 0,
    }}>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: space.xl }}>
        <Tooltip content="Close (Esc)">
          <Button
            colors={colors}
            variant="bare"
            onClick={onClose}
            aria-label="Close"
            style={{
              '--pa-btn-fg': colors.textDim,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              fontSize: 18,
              // This × is the only thing in its row, so its box height sets the
              // gap above the panel. `.pa-btn`'s 14px line-height would close
              // that gap by half; keep the 1.5 the raw button inherited.
              lineHeight: 1.5,
            } as CSSProperties}
          >&times;</Button>
        </Tooltip>
      </div>

      {detail.kind === 'recipe' && (
        <RecipeDetail job={detail.job} onRunNow={onRunNow} onPause={onPause}
          onUnpause={onUnpause} onDelete={onDelete} onKill={onKill} onResetToDefault={onResetToDefault}
          onOpenLastRun={onOpenLastRun}
          actionStates={actionStates} onActionDone={onActionDone} />
      )}
      {detail.kind === 'extension' && <ExtensionDetail ext={detail.ext} />}
      {detail.kind === 'run' && <RunDetail run={detail.run} displayName={detail.displayName} lastStatus={lastStatus} starterId={starterId} />}
    </div>
  );
}

function RecipeDetail({
job, onRunNow, onPause, onUnpause, onDelete, onKill, onResetToDefault, onOpenLastRun, actionStates, onActionDone }: {
  job: ScheduledJob;
  onRunNow: (id: string) => Promise<void>; onPause: (id: string) => Promise<void>;
  onUnpause: (id: string) => Promise<void>; onDelete: (id: string) => Promise<void>;
  onKill: (id: string) => Promise<void>; onResetToDefault: (id: string) => Promise<void>;
  onOpenLastRun: (jobId: string) => void;
  actionStates: Record<string, 'loading' | 'success'>;
  onActionDone: (key: string, afterRefresh?: () => void) => void;
}) {
  const { colors } = useTheme();
  const name = job.display_name || job.id;
  const statusColor = job.currently_running ? colors.success : job.paused ? colors.textDim : colors.cyan;
  const hasUpdate = job.starter_id && job.user_customized && job.embedded_version && job.version !== job.embedded_version;
  const detailFailure = lastFailure(job);
  // A7: /schedule/{id}/inspect — the recipe's own extensions and the next few
  // times it will actually fire, read once per job (best-effort: a failed or
  // slow fetch just means this block doesn't appear, not an error banner —
  // everything else on this panel already came from `/schedule/list`).
  const [inspect, setInspect] = useState<InspectResponse | null>(null);
  useEffect(() => {
    let cancelled = false;
    setInspect(null);
    apiFetch<InspectResponse>(`/schedule/${encodeURIComponent(job.id)}/inspect`)
      .then(data => { if (!cancelled) setInspect(data); })
      .catch(() => { /* silent — this panel has plenty to show without it */ });
    return () => { cancelled = true; };
  }, [job.id]);
  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.lg, marginBottom: space.xxl }}>
        <div style={{ width: 10, height: 10, borderRadius: '50%', background: statusColor, boxShadow: job.currently_running ? `0 0 8px ${statusColor}` : 'none' }} />
        <div style={{ fontSize: 18, fontWeight: 600, fontFamily: font.display }}>{name}</div>
        {job.version && <span style={{ fontSize: textSize.micro, fontFamily: font.mono, color: colors.textDim }}>v{job.version}</span>}
      </div>
      {job.description && <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.6, marginBottom: space.xxl }}>{job.description}</div>}
      {hasUpdate && (
        <div style={{ marginBottom: space.xxl, padding: `${space.lg}px ${space.xxl}px`, borderRadius: radius.md, background: withAlpha(colors.warning, 0.08), border: `1px solid ${withAlpha(colors.warning, 0.2)}`, fontSize: textSize.caption, color: colors.warning }}>
          Update available (v{job.embedded_version}). Reset to default to apply.
        </div>
      )}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px 24px', marginBottom: detailFailure.kind !== 'none' ? 12 : 20 }}>
        <MetaField label="Schedule" value={scheduleSummary(job)} hint={rawCronTooltip(job)} />
        <MetaField label="Status" value={job.currently_running ? 'Running' : job.paused ? 'Paused' : 'Active'} />
        <MetaField
          label={job.currently_running ? 'Started' : 'Last Run'}
          value={(() => {
            const state = runState(job);
            switch (state.kind) {
              case 'running': return state.sinceIso ? timeAgo(state.sinceIso) : 'just now';
              case 'ran': return timeAgo(state.sinceIso);
              case 'never': return 'Never';
            }
          })()}
        />
        <MetaField label="Last Result" value={detailFailure.kind === 'historical' ? 'Waiting for next run' : runChip(job)?.label ?? '—'} />
        {!!job.run_count && <MetaField label="Runs" value={String(job.run_count)} />}
        {!!job.max_retries && <MetaField label="Max Retries" value={String(job.max_retries)} />}
        {/* A bare UUID under a bare "ID" is chrome: it tells a reader nothing
            about what it is for. It is a reference to quote when something goes
            wrong, so it says that and can be copied without being selected by
            hand. */}
        <ReferenceIdField id={job.id} />
      </div>
      {/* A3: a failure the user has resumed past (or fixed the recipe
          after) is history — say so in neutral colours with the next run,
          instead of a red "Last error" that reads as still broken. */}
      {detailFailure.kind === 'current' && (
        <div data-testid="automation-last-error" style={{ marginBottom: space.xxxl, padding: `${space.md}px ${space.xl}px`, borderRadius: radius.sm, background: withAlpha(colors.danger, 0.08), border: `1px solid ${withAlpha(colors.danger, 0.2)}`, fontSize: textSize.micro, color: colors.danger, fontFamily: font.mono, wordBreak: 'break-word' }}>
          <div>Last error: {detailFailure.cause}</div>
          {/* Benchmark research §4: the run that produced this error is one
              click away, not just its message. Opening it also clears the
              card's own "needs attention" alert. */}
          <Button
            colors={colors}
            variant="bare"
            onClick={() => { acknowledgeFailure(job.id, detailFailure); onOpenLastRun(job.id); }}
            style={{
              '--pa-btn-fg': colors.danger, '--pa-btn-fg-hover': colors.text, '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0', fontFamily: font.body, fontSize: textSize.micro, marginTop: space.sm,
            } as CSSProperties}
          >View this run &rarr;</Button>
        </div>
      )}
      {detailFailure.kind === 'historical' && (
        <div data-testid="automation-failure-history" style={{ marginBottom: space.xxxl, padding: `${space.md}px ${space.xl}px`, borderRadius: radius.sm, background: colors.surface, border: `1px solid ${colors.border}`, fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.body, lineHeight: 1.5, wordBreak: 'break-word' }}>
          <div>{historicalFailureNote(detailFailure, job, scheduleSummary(job))}</div>
          <div style={{ fontFamily: font.mono, color: colors.textDim, marginTop: space.xs }}>That run's error: {detailFailure.cause}</div>
          <Button
            colors={colors}
            variant="bare"
            onClick={() => onOpenLastRun(job.id)}
            style={{
              '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.text, '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0', fontFamily: font.body, fontSize: textSize.micro, marginTop: space.sm,
            } as CSSProperties}
          >View that run &rarr;</Button>
        </div>
      )}
      {/* A7: what /schedule/{id}/inspect adds beyond the list row — the next
          few real fire times, and what the recipe can actually reach.
          Absent (not an error state) when the daemon has no answer yet, or
          for an older daemon / a recipe that failed to parse. */}
      {!!inspect?.nextRuns?.length && (
        <div data-testid="automation-next-runs" style={{ marginBottom: space.xl, fontSize: textSize.caption, color: colors.textMuted }}>
          <span style={{ fontWeight: 600, color: colors.text }}>Next {inspect.nextRuns.length} run{inspect.nextRuns.length === 1 ? '' : 's'}: </span>
          {inspect.nextRuns.map(iso => weekdayTime(iso)).join(' · ')}
        </div>
      )}
      {inspect?.recipe && (
        <div data-testid="automation-can-use" style={{ marginBottom: space.xl, fontSize: textSize.caption, color: colors.textMuted }}>
          <span style={{ fontWeight: 600, color: colors.text }}>Can use: </span>
          {inspect.recipe.extensions.length > 0 ? inspect.recipe.extensions.join(', ') : "the session's default tools"}
        </div>
      )}
      <div style={{ display: 'flex', gap: space.md, flexWrap: 'wrap' }}>
        {job.currently_running ? (
          <Button colors={colors} onClick={() => onKill(job.id)} flashSuccess={false} style={actionVars(colors, 'danger')}>Stop</Button>
        ) : (
          <>
            <Button colors={colors} variant="primary" onClick={() => onRunNow(job.id)} style={PRIMARY_ACTION_VARS}>Run Now</Button>
            {job.paused
              ? <Button colors={colors} onClick={() => guardedAction(`${job.id}:unpause`, () => onUnpause(job.id), onActionDone)} style={actionVars(colors)}>Resume</Button>
              : <Button colors={colors} onClick={() => guardedAction(`${job.id}:pause`, () => onPause(job.id), onActionDone)} style={actionVars(colors)}>Pause</Button>}
          </>
        )}
        {((job.user_customized && job.starter_id) || actionStates[`${job.id}:reset`]) && (
          <Button colors={colors} onClick={() => guardedAction(`${job.id}:reset`, () => onResetToDefault(job.id), onActionDone)} style={actionVars(colors)}>Reset to default</Button>
        )}
        <DeleteAutomationControl name={name} onDelete={() => onDelete(job.id)} />
      </div>
    </>
  );
}

function ExtensionDetail({ ext }: { ext: ExtensionInfo }) {
  const { colors } = useTheme();
  return (
    <>
      <div style={{ fontSize: 18, fontWeight: 600, fontFamily: font.display, marginBottom: space.md }}>{ext.display_name}</div>
      <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.6, marginBottom: space.xxl }}>{ext.description}</div>
      <MetaField label="Type" value={ext.type} />
      <div style={{ marginTop: space.xxl }}>
        <div style={{ fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase', color: colors.textDim, marginBottom: space.sm, letterSpacing: '0.05em' }}>
          Tools provided
        </div>
        {ext.available_tools.length > 0 ? (
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm }}>
            {ext.available_tools.map(t => (
              // R6: a fixed capability label, not a live claim — `static`.
              <Chip key={t} kind="static" tone="neutral" quiet>{t}</Chip>
            ))}
          </div>
        ) : (
          <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Tool list not available — tools are loaded at runtime.</div>
        )}
      </div>
    </>
  );
}

interface RunFindings {
  findings: Finding[];
  ledger_state?: 'valid' | 'missing' | 'unverified';
  created_at?: string | null;
}

export function findingsReportNotice(data: RunFindings, status?: ScheduleRunStatus | null): string | null {
  if (status === 'error' || data.ledger_state !== 'valid') {
    return 'Report incomplete — cleanup findings could not be saved.';
  }
  return data.findings.length === 0 ? 'Scan complete — no cleanup candidates found.' : null;
}

export function RunDetail({ run, displayName, lastStatus, starterId }: {
  run: SessionInfo & { jobId: string }; displayName: string; lastStatus?: ScheduleRunStatus | null;
  /** `model_review` swaps the storage findings ledger for the model findings
   *  ledger (W1b). Model findings are never storage Trash/Keep (ruling 13). */
  starterId?: string | null;
}) {
  const isModelReview = starterId === 'model_review';
  const { colors } = useTheme();
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  // A run's id IS its session id — open the full conversation in the chat dock.
  const openConversation = useCallback(() => {
    switchToSession(run.id).catch(err => console.error('[automate] open session failed:', err));
    openChatDock();
  }, [switchToSession, openChatDock, run.id]);
  const [reportText, setReportText] = useState<string | null>(null);
  const [reportNotice, setReportNotice] = useState<string | null>(null);
  const [previousReport, setPreviousReport] = useState<{ date: string; text: string; findings: Finding[] } | null>(null);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [actionInFlight, setActionInFlight] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);
  // Cumulative recovery across ALL runs (issue #242) — derived server-side
  // from every persisted run ledger, so it survives restarts.
  const [lifetime, setLifetime] = useState<RecoveryTotals | null>(null);

  const refreshLifetime = useCallback(async () => {
    try {
      const totals = await apiFetch<RecoveryTotals>('/automation/recovery/total');
      setLifetime(totals);
    } catch {
      // Non-fatal — the per-run view still works; the all-time line stays hidden.
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(false);
    setFindings([]);
    setReportText(null);
    setReportNotice(null);
    setPreviousReport(null);
    (async () => {
      let reportOk = false;
      let findingsOk = false;
      try {
        const session = await apiFetch<{ conversation?: Array<{ role: string; content?: Array<{ type: string; text?: string }> }> }>(`/api/sessions/${encodeURIComponent(run.id)}`);
        if (!cancelled) {
          const msgs = session.conversation || [];
          const text = msgs.filter((m: any) => m.role === 'assistant')
            .flatMap((m: any) => (m.content || []).filter((c: any) => c.type === 'text').map((c: any) => c.text))
            .join('\n\n');
          setReportText(text.replace(/<findings>[\s\S]*?<\/findings>/g, '').trim() || 'No output captured.');
        }
        reportOk = true;
      } catch {}
      let notice: string | null = null;
      // A Model Review run has no storage cleanup ledger; asking for one would
      // only produce a false "cleanup findings could not be saved" notice.
      if (isModelReview) {
        if (!cancelled) { setLoadError(false); setLoading(false); }
        return;
      }
      try {
        const data = await apiFetch<RunFindings>(`/automation/run/${encodeURIComponent(run.id)}/findings`);
        notice = findingsReportNotice(data, lastStatus);
        if (!cancelled) {
          setFindings(data.findings || []);
          setReportNotice(notice);
        }
        findingsOk = true;
      } catch {
        notice = 'Report incomplete — cleanup findings could not be saved.';
        if (!cancelled) setReportNotice(notice);
      }
      try {
        if (notice === 'Report incomplete — cleanup findings could not be saved.') {
          // Only consider this job's earlier runs; preserve the ledger's original date.
          const history = await apiFetch<SessionInfo[]>(`/schedule/${encodeURIComponent(run.jobId)}/sessions?limit=100`);
          const earlier = history.filter(candidate => candidate.id !== run.id && candidate.createdAt < run.createdAt)
            .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
          for (const candidate of earlier) {
            try {
              const ledger = await apiFetch<RunFindings>(`/automation/run/${encodeURIComponent(candidate.id)}/findings`);
              if (ledger.ledger_state !== 'valid' || !ledger.created_at) continue;
              let text = findingsReportNotice(ledger) || '';
              try {
                const prior = await apiFetch<{ conversation?: Array<{ role: string; content?: Array<{ type: string; text?: string }> }> }>(`/api/sessions/${encodeURIComponent(candidate.id)}`);
                text = (prior.conversation || []).filter(m => m.role === 'assistant')
                  .flatMap(m => (m.content || []).filter(c => c.type === 'text').map(c => c.text || ''))
                  .join('\n\n').replace(/<findings>[\s\S]*?<\/findings>/g, '').trim() || text;
              } catch { /* The valid ledger remains available even without conversation text. */ }
              if (!cancelled) setPreviousReport({ date: ledger.created_at, text, findings: ledger.findings });
              break;
            } catch { /* A missing older ledger does not hide the next valid report. */ }
          }
        }
      } catch { /* History failure must not hide the current report notice. */ }
      // Only an error if we got nothing at all — a partial load still shows a report.
      if (!cancelled) { setLoadError(!reportOk && !findingsOk); setLoading(false); }
    })();
    void refreshLifetime();
    return () => { cancelled = true; };
  }, [run.id, run.jobId, run.createdAt, lastStatus, reloadTick, refreshLifetime, isModelReview]);

  const handleAction = async (findingId: string, action: string, confirmed?: boolean) => {
    setActionInFlight(findingId);
    try {
      const body: Record<string, unknown> = { action, run_id: run.id, action_source: 'ui_click' };
      if (confirmed) body.confirmed = true;
      const result = await apiFetch<{ action_taken: string; timestamp: string; size_recovered_bytes?: number }>(`/automation/finding/${encodeURIComponent(findingId)}/action`, {
        method: 'POST', body: JSON.stringify(body),
      });
      setFindings(prev => prev.map(f => f.id === findingId ? { ...f, action_taken: result.action_taken, actioned_at: result.timestamp, size_recovered_bytes: result.size_recovered_bytes ?? null } : f));
      // The persisted ledger just changed — keep the all-time total live.
      void refreshLifetime();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error(`[cleanup] Action "${action}" failed for ${findingId}: ${msg}`);
      setFindings(prev => prev.map(f => f.id === findingId ? { ...f, action_taken: 'error', actioned_at: new Date().toISOString(), error_message: msg } : f));
    }
    setActionInFlight(null);
  };

  // Bulk trash — a SINGLE call to the new endpoint (never a client-side loop
  // of single actions, which is the exact shape of the incident this fix
  // closes). Updates local state from `results`; blocked entries are the
  // server's own veto and are surfaced rather than silently dropped.
  const handleBulkAction = async (findingIds: string[], acknowledgments: CategoryAcknowledgments): Promise<BulkSweepSummary> => {
    const response = await bulkTrashFindings(run.id, findingIds, acknowledgments);
    if (response.results.length > 0) {
      const byId = new Map(response.results.map(r => [r.finding_id, r]));
      setFindings(prev => prev.map(f => {
        const r = byId.get(f.id);
        if (!r) return f;
        return {
          ...f,
          action_taken: r.action_taken,
          actioned_at: new Date().toISOString(),
          size_recovered_bytes: response.availability?.find(a => a.finding_id === f.id)?.protection === 'cargo-profile-locks'
            ? (f.size_recovered_bytes ?? 0) + (r.size_recovered_bytes ?? 0) : r.size_recovered_bytes,
          error_message: r.error ?? null,
          category: response.availability?.find(a => a.finding_id === f.id)?.category ?? f.category,
          profiles: response.availability?.find(a => a.finding_id === f.id)?.profiles ?? f.profiles,
          protection: response.availability?.find(a => a.finding_id === f.id)?.protection ?? f.protection,
          availability_reason: r.error ?? undefined,
          available: r.error ? false : f.available,
        };
      }));
      void refreshLifetime();
    }
    return {
      cleaned: response.results.reduce((total, r) => {
        const profiles = response.availability?.find(a => a.finding_id === r.finding_id)?.profiles;
        return total + (profiles?.length ? profiles.reduce((n, p) => n + p.moved, 0) : r.error ? 0 : 1);
      }, 0),
      failed: response.results.reduce((total, r) => {
        const profiles = response.availability?.find(a => a.finding_id === r.finding_id)?.profiles;
        return total + (profiles?.length ? Math.max(r.error ? 1 : 0, profiles.reduce((n, p) => n + p.failed + p.skipped, 0)) : r.error ? 1 : 0);
      }, 0),
      bytes: response.results.reduce((n, r) => n + (r.size_recovered_bytes ?? 0), 0),
      profiles: response.availability?.flatMap(a => a.profiles || []),
    };
  };

  const totalRecovered = sumRunRecovered(findings) + findings
    .filter(f => f.action_taken !== 'trashed' && (f.protection === 'cargo-profile-locks' || /\/(target|cargo-target)$/.test(f.path) || f.path.includes('/.shared-target/')))
    .reduce((sum, f) => sum + (f.size_recovered_bytes ?? 0), 0);
  const allActioned = findings.length > 0 && findings.every(f => f.action_taken !== null);

  return (
    <>
      <div style={{ fontSize: 18, fontWeight: 600, fontFamily: font.display, marginBottom: space.xs }}>{displayName}</div>
      {/* Every other time on this page is relative — "2h ago" in Recent
          Activity, "Last Run 2h ago" in the meta grid — and this one line was
          an absolute `toLocaleString()`. On the detail panel you open FROM one
          of those rows, so the same moment changed vocabulary between the click
          and the panel. It reads the app's one way now, with the exact stamp
          still a hover away. */}
      <div
        data-testid="run-detail-when"
        style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono, marginBottom: space.xl, fontVariantNumeric: 'tabular-nums' }}
      >
        <AsOf asOf={run.createdAt} /> &middot; {run.messageCount} msgs &middot; {run.totalTokens ?? 0} tokens
      </div>
      <Tooltip content="Open this run's full conversation in chat">
        <Button
          colors={colors}
          variant="ghostOn"
          onClick={openConversation}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-border': colors.borderHi,
            '--pa-btn-bg-hover': colors.cyanGlow,
            '--pa-btn-pad': '5px 12px',
            '--pa-btn-radius': `${radius.sm}px`,
            '--pa-btn-weight': 600,
            fontFamily: font.body,
            fontSize: textSize.caption,
            gap: space.sm,
            marginBottom: space.xxxl,
          } as CSSProperties}
        >
          <FiMessageSquare size={13} style={{ verticalAlign: -2 }} />
          {' '}Open conversation
        </Button>
      </Tooltip>
      {!loading && reportNotice && (
        <div role="status" style={{ fontSize: textSize.caption, color: colors.textMuted, marginBottom: space.lg, overflowWrap: 'anywhere' }}>
          {reportNotice}
        </div>
      )}
      {!loading && previousReport && (
        <div style={{ minWidth: 0, maxWidth: '100%', overflow: 'hidden', overflowWrap: 'anywhere', marginBottom: space.xl }}>
          <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginBottom: space.md }}>
            Previous valid report — <time dateTime={previousReport.date}>{previousReport.date}</time>
          </div>
          {previousReport.text && <RenderedReport text={previousReport.text} />}
          {previousReport.findings.map(f => (
            <div key={f.id} style={{ fontSize: textSize.caption, marginBottom: space.sm }}>
              {f.path} — {f.consequence || f.recommendation}
            </div>
          ))}
        </div>
      )}
      {isModelReview ? (
        // Not gated on `findings.length`: that is the storage ledger's count.
        <>
          <ModelReviewFindings />
          {!loading && reportText && <ReportToggle text={reportText} createdAt={run.createdAt} tokens={run.totalTokens} />}
        </>
      ) : loading ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Loading results...</div>
      ) : findings.length > 0 ? (
        <>
          <FindingsPanel runId={run.id} findings={findings} actionInFlight={actionInFlight} onAction={handleAction} onBulkAction={handleBulkAction} totalRecovered={totalRecovered} allActioned={allActioned} lifetime={lifetime} />
          {reportText && <ReportToggle text={reportText} createdAt={run.createdAt} tokens={run.totalTokens} />}
        </>
      ) : reportText ? (
        <>
          <div style={{ maxHeight: 600, overflowY: 'auto' }}><RenderedReport text={reportText} /></div>
        </>
      ) : loadError ? (
        <StateBlock tone="error" title="Couldn't load this run's report." onRetry={() => setReloadTick(t => t + 1)} compact />
      ) : (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '4px 0' }}>
          No report was captured for this run.
        </div>
      )}
    </>
  );
}

/**
 * The automation's UUID, labelled for the one job it does.
 *
 * It used to render under "ID" as a bare 36-character string with nothing
 * saying what a reader would ever do with it. It is the handle to quote when
 * asking about a run that went wrong, so the label says "Reference ID", the
 * hover says what it is for, and it copies in one click — because the
 * alternative is selecting a UUID by hand out of a two-column grid.
 */
function ReferenceIdField({ id }: { id: string }) {
  const { colors } = useTheme();
  const { state, copy } = useCopyToClipboard();
  return (
    <div>
      <div style={{ fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xxs, letterSpacing: '0.05em' }}>
        Reference ID
      </div>
      {/* Same reason as the Recent Activity row: the UUID truncates because it
          is a flex CHILD of this button, and `Button`'s single `pa-btn__label`
          wrapper would make it an inline box, where `overflow: hidden` does
          nothing and a 36-character id overflows its grid column. Keeps the
          element, takes the `.pa-btn` interaction rules. */}
      <Tooltip content="Quote this when asking about a run — click to copy">
        <button
          type="button"
          className="pa-btn pa-btn--composite"
          data-testid="automation-reference-id"
          onClick={() => copy(id)}
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            fontSize: textSize.caption,
            fontFamily: font.mono,
            gap: space.sm,
            justifyContent: 'flex-start',
            textAlign: 'left',
            maxWidth: '100%',
          } as CSSProperties}
        >
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{id}</span>
          <span style={{
            flexShrink: 0, fontSize: 10, fontFamily: font.body,
            color: state === 'failed' ? colors.danger : state === 'copied' ? colors.success : colors.textDim,
          }}>
            {state === 'copied' ? 'copied' : state === 'failed' ? "couldn't copy" : 'copy'}
          </span>
        </button>
      </Tooltip>
    </div>
  );
}

function MetaField({ label, value, mono, hint }: { label: string; value: string; mono?: boolean; hint?: string }) {
  const { colors } = useTheme();
  const body = <div style={{ fontSize: textSize.caption, color: colors.textMuted, fontFamily: mono ? font.mono : font.body }}>{value}</div>;
  return (
    <div>
      <div style={{ fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xxs, letterSpacing: '0.05em' }}>{label}</div>
      {/* A2: the human summary is what's shown; the raw cron (when there is
          one) is one hover/focus away, never printed alongside it. */}
      {hint ? <Tooltip content={hint}><span tabIndex={0} style={{ outline: 'none', display: 'block' }}>{body}</span></Tooltip> : body}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Delete an automation — the two-step, in place
// ═══════════════════════════════════════════════════════════════════════

/**
 * Tier 2 of the destructive-action ruling: destructive, but recoverable with
 * effort — the recipe can be written again, and the runs it already made stay
 * in Recent Activity either way. That tier confirms on the row rather than in
 * a modal, so what is about to go stays readable and the list stays in view;
 * the full-attention interruption is reserved for the unrecoverable (rotating
 * a live drain key), which is the only `ConfirmDialog` in the app.
 *
 * Both places that offer Delete — the recipe card and the detail panel — share
 * this one control, so the question, the sentence and the failure state can't
 * drift apart between them.
 *
 * The failure is stated HERE, on the control that asked, and stays until the
 * user retries or backs out. A delete that failed must never be quiet: for a
 * while it was an OS dialog, which closes the moment you click, so a failed
 * delete looked exactly like a done one.
 */
export function DeleteAutomationControl({ name, onDelete }: {
  name: string;
  /** Performs the delete. Rejecting is how it says it failed. */
  onDelete: () => Promise<void>;
}) {
  const { colors } = useTheme();
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!confirming) {
    return (
      <Button
        colors={colors}
        onClick={() => { setError(null); setConfirming(true); }}
        style={actionVars(colors, 'danger')}
      >
        Delete
      </Button>
    );
  }

  const run = async () => {
    if (deleting) return false;
    setDeleting(true);
    setError(null);
    let ok = false;
    try {
      await onDelete();
      // The row normally goes with the refreshed list; reset anyway so a still
      // mounted control (the detail panel keeps its target) isn't stuck mid-verb.
      setConfirming(false);
      ok = true;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    setDeleting(false);
    // The button primitive reads `false` as "it failed" — a delete the daemon
    // refused must never tick. The error is still stated on the control itself.
    return ok;
  };

  // The sentence and any failure take a whole line of their own (the action
  // rows wrap), so the question is readable rather than squeezed between buttons.
  const line: React.CSSProperties = {
    flexBasis: '100%', fontSize: textSize.micro, fontFamily: font.body, lineHeight: 1.5, marginTop: space.xxs,
  };

  return (
    <>
      <Button
        colors={colors}
        onClick={() => { setConfirming(false); setError(null); }}
        style={actionVars(colors)}
      >
        Cancel
      </Button>
      <Button colors={colors} onClick={run} style={actionVars(colors, 'danger')}>
        {deleting ? 'Deleting…' : 'Delete automation'}
      </Button>
      <div style={{ ...line, color: colors.textMuted }}>
        Delete &ldquo;{name}&rdquo;? Its schedule stops and the automation itself can&apos;t be
        restored &mdash; it has to be created again. Runs it has already made stay in Recent Activity.
      </div>
      {error && (
        <div role="alert" style={{ ...line, color: colors.danger }}>
          Couldn&apos;t delete &ldquo;{name}&rdquo; &mdash; {error}
        </div>
      )}
    </>
  );
}

// The local `Btn` that used to live here is gone.
//
// It was this page's private button primitive, with its own 700ms
// minimum-pending floor and its own success phase. That floor is now
// `MIN_PENDING_MS` in `components/common/Button` — ported from exactly this
// component, which is where it was first written and proved — so a second copy
// here only meant this page's buttons and the rest of the app's could drift.
// Its three resting looks survive as `actionVars()` / `PRIMARY_ACTION_VARS`
// near the top of this file, expressed as `--pa-btn-*` custom properties.

// ═══════════════════════════════════════════════════════════════════════
// Sub-components preserved from original (Findings, Reports, Modal)
// ═══════════════════════════════════════════════════════════════════════

function ReportToggle({ text, createdAt, tokens }: { text: string; createdAt: string; tokens: number | null }) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  return (
    <div style={{ marginTop: space.xxl }}>
      {/* Disclosure: its whole job is to open the report below it, so it keeps
          the element and the aria-expanded that names its state, and takes the
          shared `.pa-btn` interaction rules instead of a primitive whose
          pending floor and success tick have nothing to describe here. */}
      <button
        type="button"
        className="pa-btn"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        style={{
          '--pa-btn-fg': colors.textDim,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '8px 0',
          gap: space.md,
          justifyContent: 'flex-start',
          fontSize: textSize.micro,
          fontFamily: font.body,
        } as CSSProperties}
      >
        <FiChevronRight size={10}
          style={{ transform: open ? 'rotate(90deg)' : 'rotate(0deg)', transition: springTransition('transform') }} />
        View full report &middot; <AsOf asOf={createdAt} /> &middot; {tokens ?? 0} tokens
      </button>
      {open && <div style={{ maxHeight: 400, overflowY: 'auto', marginTop: space.xs }}><RenderedReport text={text} /></div>}
    </div>
  );
}

function RenderedReport({ text }: { text: string }) {
  const { colors } = useTheme();
  const lines = text.split('\n');
  const elements: React.ReactNode[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { i++; continue; }
    if (line.startsWith('# ')) { elements.push(<div key={i} style={{ fontSize: textSize.heading, fontWeight: 700, fontFamily: font.display, color: colors.text, marginTop: space.xxl, marginBottom: space.md }}>{renderInline(line.slice(2), colors)}</div>); i++; continue; }
    if (line.startsWith('## ')) { elements.push(<div key={i} style={{ fontSize: textSize.body, fontWeight: 600, fontFamily: font.display, color: colors.text, marginTop: 14, marginBottom: space.sm }}>{renderInline(line.slice(3), colors)}</div>); i++; continue; }
    if (line.startsWith('### ')) { elements.push(<div key={i} style={{ fontSize: textSize.small, fontWeight: 600, color: colors.cyan, marginTop: space.xl, marginBottom: space.xs }}>{renderInline(line.slice(4), colors)}</div>); i++; continue; }
    if (line.match(/^---+$/)) { elements.push(<hr key={i} style={{ border: 'none', borderTop: `1px solid ${colors.border}`, margin: '12px 0' }} />); i++; continue; }
    if (line.startsWith('```')) { const codeLines: string[] = []; i++; while (i < lines.length && !lines[i].startsWith('```')) { codeLines.push(lines[i]); i++; } i++; elements.push(<pre key={`code-${i}`} style={{ padding: `${space.lg}px ${space.xl}px`, borderRadius: radius.sm, background: colors.bgDeeper, border: `1px solid ${colors.border}`, fontSize: textSize.micro, fontFamily: font.mono, color: colors.textMuted, overflow: 'auto', margin: '6px 0', lineHeight: 1.5 }}>{codeLines.join('\n')}</pre>); continue; }
    if (line.match(/^\s*-\s/)) { const bl: string[] = []; while (i < lines.length && lines[i].match(/^\s*-\s/)) { bl.push(lines[i].replace(/^\s*-\s/, '')); i++; } elements.push(<div key={`b-${i}`} style={{ margin: '4px 0 4px 4px' }}>{bl.map((b, j) => <div key={j} style={{ display: 'flex', gap: space.md, marginBottom: space.xxs, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}><span style={{ color: colors.cyan, flexShrink: 0, marginTop: space.xxs }}>&#8226;</span><span>{renderInline(b, colors)}</span></div>)}</div>); continue; }
    if (line.match(/^\d+\.\s/)) { const nl: string[] = []; while (i < lines.length && lines[i].match(/^\d+\.\s/)) { nl.push(lines[i].replace(/^\d+\.\s/, '')); i++; } elements.push(<div key={`n-${i}`} style={{ margin: '4px 0 4px 4px' }}>{nl.map((n, j) => <div key={j} style={{ display: 'flex', gap: space.md, marginBottom: space.xxs, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}><span style={{ color: colors.textDim, flexShrink: 0, fontFamily: font.mono, fontSize: textSize.micro, minWidth: 16 }}>{j + 1}.</span><span>{renderInline(n, colors)}</span></div>)}</div>); continue; }
    elements.push(<div key={i} style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.6, marginBottom: space.xs }}>{renderInline(line, colors)}</div>); i++;
  }
  return <div>{elements}</div>;
}

function renderInline(text: string, colors: { text: string; cyan: string; border: string }): React.ReactNode {
  const parts: React.ReactNode[] = []; let remaining = text; let key = 0;
  while (remaining.length > 0) {
    const bold = remaining.match(/^(.*?)\*\*(.+?)\*\*(.*)/s);
    if (bold) { if (bold[1]) parts.push(<span key={key++}>{bold[1]}</span>); parts.push(<strong key={key++} style={{ color: colors.text, fontWeight: 600 }}>{bold[2]}</strong>); remaining = bold[3]; continue; }
    const code = remaining.match(/^(.*?)`(.+?)`(.*)/s);
    if (code) { if (code[1]) parts.push(<span key={key++}>{code[1]}</span>); parts.push(<code key={key++} style={{ fontFamily: font.mono, fontSize: textSize.micro, padding: `${space.xxs}px ${space.xs}px`, borderRadius: radius.xs, background: colors.border, color: colors.cyan }}>{code[2]}</code>); remaining = code[3]; continue; }
    parts.push(<span key={key++}>{remaining}</span>); break;
  }
  return parts.length === 1 ? parts[0] : <>{parts}</>;
}

const FINDING_TYPE_LABELS: Record<string, string> = {
  dev_cache: 'Dev Caches',
  app_cache: 'App Caches',
  build_artifact: 'Build Artifacts',
  stale_file: 'Old Downloads',
  large_file: 'Large User Files',
};

function groupFindings(findings: Finding[]): Map<string, Finding[]> {
  const groups = new Map<string, Finding[]>();
  for (const f of findings) {
    // Group by finding_type if available, fall back to path-based grouping
    let group: string;
    const ft = (f as unknown as Record<string, unknown>).type as string | undefined;
    if (ft && FINDING_TYPE_LABELS[ft]) {
      group = FINDING_TYPE_LABELS[ft];
    } else {
      const p = f.path.toLowerCase();
      if (p.includes('/desktop/')) group = 'Desktop';
      else if (p.includes('/downloads/')) group = 'Downloads';
      else if (p.includes('/documents/')) group = 'Documents';
      else if (p.includes('/.cache/') || p.includes('/.npm') || p.includes('/.rustup') || p.includes('/target/')) group = 'Dev Caches';
      else group = 'Other';
    }
    (groups.get(group) || (() => { const l: Finding[] = []; groups.set(group, l); return l; })()).push(f);
  }
  return groups;
}

export function FindingsPanel({
runId, findings, actionInFlight, onAction, onBulkAction, totalRecovered, allActioned, lifetime }: {
  runId: string; findings: Finding[]; actionInFlight: string | null;
  onAction: (findingId: string, action: string, confirmed?: boolean) => Promise<void>;
  onBulkAction: (findingIds: string[], acknowledgments: CategoryAcknowledgments) => Promise<BulkSweepSummary>;
  totalRecovered: number; allActioned: boolean;
  lifetime: RecoveryTotals | null;
}) {
  const { colors } = useTheme();
  const groups = groupFindings(findings);
  const [expandedGroup, setExpandedGroup] = useState<string | null>(null);
  const [previewGroup, setPreviewGroup] = useState<string | null>(null);
  const [includeRegenerable, setIncludeRegenerable] = useState(false);
  const [includeReview, setIncludeReview] = useState(false);
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [observations, setObservations] = useState<Map<string, Finding>>(new Map());
  const checkGeneration = useRef(0);
  useEffect(() => {
    setObservations(new Map());
    ++checkGeneration.current;
    setChecking(false);
  }, [findings, runId]);
  const currentFindings = findings.map(f => ({ ...f, ...observations.get(f.id), action_taken: f.action_taken }));
  const recheck = async () => {
    const generation = ++checkGeneration.current;
    setChecking(true);
    setCheckError(null);
    setCompletedProfiles([]);
    try {
      const response = await previewFindings(runId, findings.filter(f => !f.action_taken).map(f => f.id));
      if (generation !== checkGeneration.current) return;
      const byId = new Map(response.availability.map(a => [a.finding_id, a]));
      setObservations(new Map(response.findings.map(f => {
        const a = byId.get(f.id);
        return [f.id, { ...f, category: a?.category ?? f.category, available: a?.eligible ?? false,
          availability_reason: a?.reason ?? 'Availability was not returned', protection: a?.protection, profiles: a?.profiles }];
      })));
    } catch (error) {
      if (generation === checkGeneration.current) setCheckError(error instanceof Error ? error.message : String(error));
    } finally {
      if (generation === checkGeneration.current) setChecking(false);
    }
  };
  const openCleanup = (group: string) => {
    setCompletedProfiles([]);
    setPreviewGroup(group);
    setIncludeRegenerable(false);
    setIncludeReview(false);
    void recheck();
  };
  const [completedProfiles, setCompletedProfiles] = useState<CargoProfile[]>([]);
  const sweepTargets = useRef<Finding[]>([]);
  const sweepAcknowledgments = useRef<CategoryAcknowledgments>({ include_regenerable: false, include_review: false });

  /**
   * The cleanup sweep, on the app's one long-running-job machine.
   *
   * A single POST that may be moving thousands of files used to be one
   * `cleaning` boolean and the word "Cleaning..." — the same sentence for two
   * files and for two thousand, and a success that was only visible as the
   * dialog disappearing. Now the terminal phases are written out: the success
   * line names what moved to Trash, and a server veto keeps the dialog open
   * with the server's own words in it rather than vanishing like a success.
   *
   * The sweep is one request with no progress channel, so there is no size to
   * report and `<JobProgress>` draws the honest indeterminate band.
   */
  const sweep = useLongRunningJob<BulkSweepSummary>({
    run: async ({ report }) => {
      const items = sweepTargets.current;
      report({ stage: 'trashing', status: `Moving ${items.length} item${items.length === 1 ? '' : 's'} to Trash` });
      try {
        const result = await onBulkAction(items.map(f => f.id), sweepAcknowledgments.current);
        setCompletedProfiles(result.profiles || []);
        return result;
      } catch (err) {
        if (err instanceof BulkActionBlockedError) {
          // The server vetoed something the client thought was eligible (a
          // race with a fresher scan, say) — say so rather than pretend
          // success. The dialog stays open so blocked/excluded stay visible.
          throw new Error(
            `${err.message}${err.blocked.length ? ` (${err.blocked.length} item${err.blocked.length === 1 ? '' : 's'} blocked)` : ''}`,
          );
        }
        throw err;
      }
    },
    summarize: (s) =>
      s.failed > 0
        ? `Moved ${s.cleaned} to Trash · ${s.failed} could not be removed`
        : `Moved ${s.cleaned} to Trash — ${formatBytes(s.bytes)} estimated; space is not freed until Trash is emptied`,
    onSuccess: (s) => {
      // Only a clean sweep closes the dialog; anything the server refused
      // stays on screen next to the list it refused it from.
      if (s.failed === 0 && !s.profiles?.length) { setPreviewGroup(null); setIncludeRegenerable(false); setIncludeReview(false); }
    },
  });

  const closeDialog = () => { ++checkGeneration.current; setChecking(false); setPreviewGroup(null); setIncludeRegenerable(false); setIncludeReview(false); sweep.reset(); };

  const confirmBulk = (eligible: Finding[]) => {
    if (eligible.length === 0 || checking || checkError) return;
    sweepAcknowledgments.current = { include_regenerable: includeRegenerable, include_review: includeReview };
    const profilePaths = Object.fromEntries(eligible.filter(f => f.protection === 'cargo-profile-locks').map(f =>
      [f.id, (f.profiles || []).filter(p => p.status === 'eligible').map(p => p.path)]));
    if (Object.keys(profilePaths).length) sweepAcknowledgments.current.profile_paths = profilePaths;
    sweepTargets.current = eligible;
    void sweep.start();
  };

  const totalPending = findings.filter(f => !f.action_taken).length;
  const totalPendingBytes = sumPendingBytes(findings);
  const defaultBytes = bulkEligible(currentFindings, false).eligible.reduce((n, f) => n + cleanupBytes(f), 0);
  const lifetimeLine = lifetime ? `${formatBytes(lifetime.total_recovered_bytes)} moved to Trash across reports (estimate)` : null;

  return (
    <div style={{ marginTop: space.xl }}>
      <div style={{
        padding: `${space.xxxl}px ${space.huge}px`, borderRadius: radius.lg, marginBottom: space.xxxl,
        background: allActioned ? `linear-gradient(135deg, ${withAlpha(colors.success, 0.12)}, ${withAlpha(colors.success, 0.04)})` : `linear-gradient(135deg, ${withAlpha(colors.cyan, 0.1)}, ${withAlpha(colors.purple, 0.06)})`,
        border: `1px solid ${allActioned ? withAlpha(colors.success, 0.25) : colors.borderHi}`,
      }}>
        <div style={{ fontSize: textSize.heading, fontWeight: 700, fontFamily: font.display, color: allActioned ? colors.success : colors.text }}>
          {allActioned ? `${formatBytes(totalRecovered)} moved to Trash (estimate)` : `${formatBytes(totalPendingBytes)} found (estimate)`}
        </div>
        <div style={{ fontSize: textSize.small, color: colors.textMuted, marginTop: space.xs }}>
          {allActioned ? `All ${findings.length} items reviewed.` : `${totalPending} items across ${groups.size} locations.`}
        </div>
        {lifetimeLine && (
          <div style={{ fontSize: textSize.caption, color: colors.textDim, marginTop: space.sm, fontVariantNumeric: 'tabular-nums' }}>
            {lifetimeLine}
          </div>
        )}
        {totalPending > 0 && (
          <Button
            colors={colors}
            variant="primary"
            onClick={() => openCleanup('__all__')}
            style={{
              '--pa-btn-pad': '12px 32px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 700,
              fontFamily: font.body,
              fontSize: textSize.body,
              marginTop: space.lg,
              maxWidth: '100%',
              whiteSpace: 'normal',
              overflowWrap: 'anywhere',
            } as CSSProperties}
          >
            {defaultBytes > 0 ? `Review cleanup — ${formatBytes(defaultBytes)} selected` : 'Review cleanup options'}
          </Button>
        )}
        {/* The sweep outlives its dialog: a clean run closes the dialog, and
            this is where the run gets to say what it actually did. */}
        {!previewGroup && (
          <JobProgress job={sweep} label="Cleanup sweep" style={{ marginTop: space.xl }} />
        )}
      </div>
      {previewGroup && (() => {
        const pool = (previewGroup === '__all__' ? currentFindings : currentFindings.filter(f => (groups.get(previewGroup) || []).some(g => g.id === f.id))).filter(f => !f.action_taken);
        return (
          <BulkConfirmDialog
            completedProfiles={completedProfiles}
            pending={pool}
            includeRegenerable={includeRegenerable}
            onToggleRegenerable={setIncludeRegenerable}
            includeReview={includeReview}
            onToggleReview={setIncludeReview}
            checking={checking}
            checkError={checkError}
            onRecheck={() => void recheck()}
            onCancel={closeDialog}
            onConfirm={confirmBulk}
            sweep={sweep}
          />
        );
      })()}
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
        {Array.from(groups.entries()).map(([groupName, items]) => {
          const pending = items.filter(f => !f.action_taken);
          const pendingBytes = pending.reduce((s, f) => s + f.size_bytes, 0);
          const groupRecovered = items.filter(f => f.action_taken === 'trashed').reduce((s, f) => s + (f.size_recovered_bytes || 0), 0);
          const isExpanded = expandedGroup === groupName;
          const allDone = pending.length === 0;
          return (
            <div key={groupName} style={{ borderRadius: radius.lg, overflow: 'hidden', border: `1px solid ${allDone ? withAlpha(colors.success, 0.2) : colors.border}`, background: allDone ? withAlpha(colors.success, 0.03) : colors.surface }}>
              <div style={{ padding: `${space.xxl}px ${space.xxxl}px`, display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.lg }}>
                {/* Disclosure: the folder tile opens and closes the item list
                    below it and does nothing else, so it keeps the element and
                    the aria pairing, and takes the `.pa-btn` rules. It had no
                    accessible name at all — an icon in an icon. */}
                <button
                  type="button"
                  className="pa-btn"
                  aria-expanded={isExpanded}
                  aria-label={`${isExpanded ? 'Hide' : 'Show'} the items in ${groupName}`}
                  onClick={() => setExpandedGroup(isExpanded ? null : groupName)}
                  style={{
                    '--pa-btn-pad': '0',
                    '--pa-btn-radius': `${radius.md}px`,
                    '--pa-btn-bg-hover': 'transparent',
                    opacity: allDone ? 0.5 : 1,
                  } as CSSProperties}
                >
                  <div style={{ width: 36, height: 36, borderRadius: radius.md, background: colors.border, display: 'grid', placeItems: 'center' }}>
                    <FiFolder size={18} color={colors.textMuted} />
                  </div>
                </button>
                <div style={{ flex: 1, minWidth: 0, overflowWrap: 'anywhere' }}>
                  <div style={{ fontSize: textSize.body, fontWeight: 600, fontFamily: font.display, color: allDone ? colors.success : colors.text }}>{groupName}</div>
                  <div style={{ fontSize: textSize.caption, color: colors.textDim, marginTop: space.xxs }}>{allDone ? `Moved to Trash — ${formatBytes(groupRecovered)} estimated` : `${pending.length} items · ${formatBytes(pendingBytes)}`}</div>
                </div>
                {allDone ? <div style={{ padding: `${space.md}px ${space.xxl}px`, borderRadius: radius.md, background: withAlpha(colors.success, 0.1), color: colors.success, fontSize: textSize.caption, fontWeight: 600 }}>Done</div> : (
                  <Button
                    colors={colors}
                    variant="primary"
                    onClick={() => openCleanup(groupName)}
                    style={{
                      '--pa-btn-pad': '10px 22px',
                      '--pa-btn-radius': `${radius.md}px`,
                      '--pa-btn-weight': 700,
                      fontFamily: font.body,
                      fontSize: textSize.small,
                    } as CSSProperties}
                  >Review cleanup options</Button>
                )}
              </div>
              {isExpanded && (
                <div style={{ padding: '0 20px 16px', borderTop: `1px solid ${colors.border}` }}>
                  {items.map(f => <FindingRow key={f.id} finding={currentFindings.find(item => item.id === f.id) || f} loading={actionInFlight === f.id || checking} onRecheck={() => openCleanup(groupName)} onAction={(a, confirmed) => onAction(f.id, a, confirmed)} />)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// Small inline badge for a finding's category — shown on every row so the
// category (and not just a free-text "recommendation") is always visible.
function CategoryBadge({ category }: { category: CategoryKey }) {
  const { colors } = useTheme();
  const color = categoryColor(category, colors);
  // R6: a fixed classification, not a live claim — `static`, with the
  // category's own colour (there's no single ChipTone per category).
  return <Chip kind="static" color={color} style={{ flexShrink: 0 }}>{CATEGORY_LABELS[category]}</Chip>;
}

// ═══════════════════════════════════════════════════════════════════════
// Bulk confirmation presents current server observations. Every pending item
// appears once, and opting in never overrides a current server exclusion.

export function BulkConfirmDialog({
  pending, includeRegenerable, onToggleRegenerable, includeReview = false,
  onToggleReview, checking = false, checkError = null, onRecheck,
  onCancel, onConfirm, sweep, completedProfiles = [],
}: {
  pending: Finding[]; includeRegenerable: boolean; completedProfiles?: CargoProfile[];
  onToggleRegenerable: (v: boolean) => void;
  includeReview?: boolean; onToggleReview?: (v: boolean) => void;
  checking?: boolean; checkError?: string | null; onRecheck?: () => void;
  onCancel: () => void; onConfirm: (eligible: Finding[]) => void;
  sweep: LongRunningJob<BulkSweepSummary>;
}) {
  const { colors } = useTheme();
  const { eligible, optional, excluded } = bulkEligible(pending, includeRegenerable, includeReview);
  const rustTargets = pending.filter(f => f.protection?.startsWith('cargo-') || /\/(target|cargo-target)$/.test(f.path));
  const profiles = completedProfiles.length ? completedProfiles : rustTargets.flatMap(f => f.profiles || []);
  const profileBytes = (status: CargoProfile['status']) => profiles.filter(p => p.status === status).reduce((total, p) => total + p.bytes, 0);
  const sum = (items: Finding[]) => items.reduce((n, f) => n + cleanupBytes(f), 0);
  const renderItems = (label: string, items: Finding[]) => (
    <div style={{ padding: space.xl, minWidth: 0, borderBottom: `1px solid ${colors.border}` }}>
      <div style={{ fontSize: textSize.caption, fontWeight: 600 }}>{label} — {items.length} items · {formatBytes(sum(items))} estimated</div>
      {items.map(f => <div key={f.id} style={{ marginTop: space.md, minWidth: 0, overflowWrap: 'anywhere' }}>
        <CategoryBadge category={categoryOf(f)} />
        <div style={{ fontSize: textSize.caption }}>{f.path}</div>
        <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>
          {formatBytes(f.size_bytes)} estimated · {f.availability_reason || f.consequence || f.recommendation}
          {f.protection === 'probe-only' ? ' · probe-only' : ''}
        </div>
      </div>)}
    </div>
  );
  return (
    <div style={{ marginBottom: space.xxl, borderRadius: radius.lg, overflow: 'hidden', minWidth: 0, border: `1px solid ${colors.borderHi}`, background: colors.bgDeeper }}>
      <div style={{ padding: space.xl, fontSize: textSize.body, fontWeight: 600 }}>
        {checking ? 'Checking availability…' : 'Review cleanup options'}
      </div>
      {rustTargets.length > 0 && (
        <div style={{ padding: space.xl, minWidth: 0, overflowWrap: 'anywhere', fontSize: textSize.caption, color: colors.textMuted }}>
          <div>Build outputs are cleaned per profile. Profiles in use are skipped. Profile folders and Cargo lock files remain. Cleaned outputs will need rebuilding.</div>
          <div style={{ marginTop: space.md }}>
            Eligible profile contents — {formatBytes(profileBytes('eligible'))} estimated · Busy profile contents — {formatBytes(profileBytes('busy'))} estimated · Unsupported profile contents — {formatBytes(profileBytes('unsupported'))} estimated
          </div>
          {profiles.map(profile => <div key={profile.path} style={{ marginTop: space.md }}>
            {profile.path} — {profile.reason} · {formatBytes(profile.bytes)} estimated
            {(profile.moved > 0 || profile.skipped > 0 || profile.failed > 0) && <span> · {profile.moved} moved · {profile.skipped} skipped · {profile.failed} failed · {formatBytes(profile.moved_bytes)} moved</span>}
          </div>)}
        </div>
      )}
      {checkError && <div role="alert" style={{ padding: space.xl, fontSize: textSize.caption, color: colors.danger, overflowWrap: 'anywhere' }}>{checkError}</div>}
      {renderItems('Selected', eligible)}
      {renderItems('Optional — rebuildable', optional.filter(f => categoryOf(f) === 'regenerable_costly'))}
      {renderItems('Optional — requiring review', optional.filter(f => categoryOf(f) === 'review_before_removing'))}
      <div style={{ padding: space.xl, display: 'flex', flexDirection: 'column', gap: space.md }}>
        <label style={{ display: 'flex', gap: space.md, fontSize: textSize.caption }}>
          <input type="checkbox" checked={includeRegenerable} disabled={checking || sweep.running} onChange={e => onToggleRegenerable(e.target.checked)} />
          <span style={{ minWidth: 0, overflowWrap: 'anywhere' }}>{REGENERABLE_ACK}</span>
        </label>
        <label style={{ display: 'flex', gap: space.md, fontSize: textSize.caption }}>
          <input type="checkbox" checked={includeReview} disabled={checking || sweep.running || !onToggleReview} onChange={e => onToggleReview?.(e.target.checked)} />
          <span style={{ minWidth: 0, overflowWrap: 'anywhere' }}>{REVIEW_ACK}</span>
        </label>
      </div>
      {renderItems('Excluded — will not be removed', excluded)}
      {onRecheck && <Button colors={colors} disabled={checking || sweep.running} onClick={onRecheck} style={actionVars(colors)}>Recheck availability</Button>}
      {sweep.phase !== 'idle' && <JobProgress job={sweep} label="Cleanup sweep" onRetry={null} />}
      <div style={{ padding: space.xl, display: 'flex', flexWrap: 'wrap', gap: space.md, justifyContent: 'flex-end' }}>
        <Button colors={colors} disabled={sweep.running} onClick={onCancel} style={actionVars(colors)}>Cancel</Button>
        <Button colors={colors} variant="primary" disabled={checking || !!checkError || sweep.running || completedProfiles.length > 0 || eligible.length === 0}
          minPendingMs={0} flashSuccess={false} onClick={() => onConfirm(eligible)}
          style={{ ...PRIMARY_ACTION_VARS, maxWidth: '100%', whiteSpace: 'normal', overflowWrap: 'anywhere' } as CSSProperties}>
          {checking ? 'Checking availability…' : sweep.running ? 'Moving to Trash…' : `Move ${eligible.length} items to Trash — ${formatBytes(sum(eligible))}`}
        </Button>
      </div>
    </div>
  );
}

export function FindingRow({
finding, loading, onAction, onRecheck }: { finding: Finding; loading: boolean; onRecheck?: () => void; onAction: (a: string, confirmed?: boolean) => void }) {
  const { colors } = useTheme();
  const fileName = finding.path.split('/').pop() || finding.path;
  const category = categoryOf(finding);


  if (finding.action_taken === 'trashed') return <div style={{ display: 'flex', alignItems: 'center', gap: space.md, padding: `${space.sm}px ${space.lg}px`, borderRadius: radius.sm, background: withAlpha(colors.success, 0.05), border: `1px solid ${withAlpha(colors.success, 0.12)}` }}><span style={{ fontSize: textSize.caption, color: colors.success }}>Trashed</span><span style={{ fontSize: textSize.micro, color: colors.textDim, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</span>{finding.size_recovered_bytes != null && <span style={{ fontSize: textSize.micro, color: colors.success, fontFamily: font.mono, fontVariantNumeric: 'tabular-nums' }}>{formatBytes(finding.size_recovered_bytes)} estimated</span>}</div>;
  if (finding.action_taken === 'error') return <div style={{ display: 'flex', alignItems: 'center', gap: space.md, padding: `${space.sm}px ${space.lg}px`, borderRadius: radius.sm, background: withAlpha(colors.danger, 0.06), border: `1px solid ${withAlpha(colors.danger, 0.15)}` }}><span style={{ fontSize: textSize.caption, color: colors.danger, fontWeight: 600, flexShrink: 0 }}>Failed</span><Tooltip content={finding.error_message || undefined}><span tabIndex={0} style={{ outline: 'none' }}><span style={{ fontSize: textSize.micro, color: colors.textDim, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{finding.error_message || fileName}</span></span></Tooltip><Button colors={colors} onClick={() => onAction('trash')} style={{ ...actionVars(colors, 'danger'), '--pa-btn-pad': '2px 6px', fontSize: textSize.micro, flexShrink: 0 } as CSSProperties}>Retry</Button></div>;
  if (finding.action_taken) return <div style={{ display: 'flex', alignItems: 'center', gap: space.md, padding: `${space.sm}px ${space.lg}px`, borderRadius: radius.sm, opacity: 0.6 }}><span style={{ fontSize: textSize.caption, color: colors.textMuted }}>Kept</span><span style={{ fontSize: textSize.micro, color: colors.textDim, flex: 1 }}>{fileName}</span></div>;

  const handleTrashClick = () => {
    onAction('trash');
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm, padding: `${space.md}px ${space.lg}px`, borderRadius: radius.sm, background: colors.surface, border: `1px solid ${colors.border}`, marginTop: space.xs }}>
      <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: space.sm }}>
            <div style={{ fontSize: textSize.caption, fontWeight: 500, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</div>
            <CategoryBadge category={category} />
          </div>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono, marginTop: space.xxs, fontVariantNumeric: 'tabular-nums' }}>{formatBytes(finding.size_bytes)}{finding.age_days != null && <> &middot; {finding.age_days}d old</>}</div>
          {(category === 'in_use' || category === 'managed_by_macos') && finding.consequence && (
            <div style={{ fontSize: textSize.micro, color: categoryColor(category, colors), marginTop: space.xxs }}>{finding.consequence}</div>
          )}
        </div>
        <div style={{ display: 'flex', gap: space.xs, flexShrink: 0 }}>
          <Button colors={colors} onClick={category === 'in_use' ? onRecheck : handleTrashClick} disabled={loading || category === 'managed_by_macos' || (category === 'in_use' && !onRecheck)} style={{ ...actionVars(colors, 'danger'), '--pa-btn-pad': '3px 8px', '--pa-btn-weight': 600, fontSize: textSize.micro } as CSSProperties}>{loading ? 'Checking availability…' : category === 'in_use' ? 'Recheck availability' : category === 'managed_by_macos' ? 'Managed by macOS' : 'Trash'}</Button>
          <Button colors={colors} onClick={() => onAction('keep')} disabled={loading} style={{ ...actionVars(colors), '--pa-btn-pad': '3px 8px', fontSize: textSize.micro } as CSSProperties}>Keep</Button>
        </div>
      </div>

    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// New Automation Modal (preserved from original)
// ═══════════════════════════════════════════════════════════════════════

function NewAutomationModal({
onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { colors } = useTheme();
  const pushOverlay = useCommandCenter(s => s.pushBrowserOverlay);
  const popOverlay = useCommandCenter(s => s.popBrowserOverlay);
  useEffect(() => { pushOverlay(); return () => { popOverlay(); }; }, [pushOverlay, popOverlay]);

  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  // 'cron' (recurring) is the default so nothing about the existing flow changes.
  const [kind, setKind] = useState<'cron' | 'once' | 'interval'>('cron');
  const [selectedPreset, setSelectedPreset] = useState(0);
  const [customCron, setCustomCron] = useState('');
  const [atLocal, setAtLocal] = useState('');
  const [intervalValue, setIntervalValue] = useState('1');
  const [intervalUnit, setIntervalUnit] = useState<'minutes' | 'hours' | 'days'>('hours');
  const [tz, setTz] = useState('');
  const [maxRetries, setMaxRetries] = useState('0');

  const cron = selectedPreset < CRON_PRESETS.length - 1 ? CRON_PRESETS[selectedPreset].cron : customCron;
  const unitSecs: Record<typeof intervalUnit, number> = { minutes: 60, hours: 3600, days: 86400 };
  const everySeconds = Math.max(0, Math.round(Number(intervalValue) || 0)) * unitSecs[intervalUnit];

  // Throws on every path that did NOT create an automation. `FormModal` turns
  // that into the sentence above the action row and keeps the modal open — so a
  // rejected form and a refused POST are said the same way, in the same place,
  // and neither can look like a create. There is no local `error` or `saving`
  // state any more: the shell owns both.
  const handleSave = async () => {
    if (!name.trim() || !prompt.trim()) throw new Error('a name and a task are both required.');
    const recipe = { version: '1.0.0', title: name.trim(), description: prompt.trim().slice(0, 120), prompt: prompt.trim() };
    const body: Record<string, unknown> = {
      id: name.trim().replace(/\s+/g, '-').toLowerCase(),
      recipe,
      max_retries: Math.max(0, Math.round(Number(maxRetries) || 0)),
    };
    if (kind === 'cron') {
      if (!cron.trim()) throw new Error('choose or enter a schedule.');
      body.cron = cron;
      if (tz.trim()) body.tz = tz.trim();
    } else if (kind === 'once') {
      if (!atLocal) throw new Error('pick a date and a time.');
      const d = new Date(atLocal);
      if (Number.isNaN(d.getTime())) throw new Error('that date and time did not parse.');
      body.at = d.toISOString();
      if (tz.trim()) body.tz = tz.trim();
    } else {
      if (everySeconds < 1) throw new Error('enter an interval of at least a minute.');
      body.every_seconds = everySeconds;
    }
    await apiFetch<unknown>('/schedule/create', { method: 'POST', body: JSON.stringify(body) });
    onCreated();
  };

  const kindTabs: { id: typeof kind; label: string }[] = [
    { id: 'cron', label: 'Recurring' },
    { id: 'once', label: 'One-time' },
    { id: 'interval', label: 'Interval' },
  ];
  const fieldStyle = {
    width: '100%', padding: `${space.md}px ${space.xl}px`, borderRadius: radius.sm, background: colors.surface,
    border: `1px solid ${colors.border}`, color: colors.text, fontSize: textSize.small, fontFamily: font.body,
    outline: 'none', boxSizing: 'border-box' as const,
  };

  return (
    // The form people type into had the worst floor in the app: no Escape, no
    // close button, no `role="dialog"`, no focus trap, no focus return — and
    // pressing Enter in the name field did nothing at all. `FormModal` is all
    // of that, inherited rather than re-decided here.
    <FormModal
      title="New automation"
      width={480}
      submitLabel="Create automation"
      failureLabel="Couldn't create this automation"
      onSubmit={handleSave}
      onCancel={onClose}
    >
      <>

        <label style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, display: 'block', marginBottom: space.sm }}>What should we call this?</label>
        <input value={name} onChange={e => setName(e.target.value)} placeholder="e.g., Weekly Cleanup" style={{
          width: '100%', padding: `${space.md}px ${space.xl}px`, borderRadius: radius.sm, background: colors.surface,
          border: `1px solid ${colors.border}`, color: colors.text, fontSize: textSize.small, fontFamily: font.body,
          outline: 'none', marginBottom: space.xxl, boxSizing: 'border-box',
        }} />

        <label style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, display: 'block', marginBottom: space.sm }}>What should the agent do?</label>
        <textarea value={prompt} onChange={e => setPrompt(e.target.value)}
          placeholder="Scan my Downloads folder for files older than 30 days..." rows={4} style={{
            width: '100%', padding: `${space.md}px ${space.xl}px`, borderRadius: radius.sm, background: colors.surface,
            border: `1px solid ${colors.border}`, color: colors.text, fontSize: textSize.small, fontFamily: font.body,
            outline: 'none', resize: 'vertical', marginBottom: space.xxl, boxSizing: 'border-box',
          }} />

        <label style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, display: 'block', marginBottom: space.sm }}>When should it run?</label>
        <div style={{ display: 'flex', gap: space.xs, marginBottom: space.xl, background: colors.surface, borderRadius: radius.sm, padding: space.xxs, border: `1px solid ${colors.border}` }}>
          {/* D4: the one legitimately concentric pair on this screen — a tab
              sits flush inside the segmented control's own padding, so its
              radius is DERIVED from the container's, not a second number
              picked by eye. concentric(radius.sm, 3) = 3px. */}
          {kindTabs.map(t => (
            <Button
              key={t.id}
              colors={colors}
              // Inside a `<form>` a button with no type IS a submit button, so
              // picking a schedule kind would have submitted the form.
              type="button"
              variant={kind === t.id ? 'primary' : 'bare'}
              onClick={() => setKind(t.id)}
              style={{
                '--pa-btn-fg': kind === t.id ? colors.textOnCyan : colors.textMuted,
                '--pa-btn-fg-hover': kind === t.id ? colors.textOnCyan : colors.text,
                '--pa-btn-pad': '6px 0',
                '--pa-btn-radius': `${concentric(radius.sm, 3)}px`,
                '--pa-btn-weight': kind === t.id ? 600 : 400,
                fontFamily: font.body,
                fontSize: textSize.caption,
                flex: 1,
              } as CSSProperties}
            >{t.label}</Button>
          ))}
        </div>

        {kind === 'cron' && (
          <>
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.xs, marginBottom: space.xl }}>
              {CRON_PRESETS.map((preset, i) => (
                <label key={i} style={{ display: 'flex', alignItems: 'center', gap: space.md, cursor: 'pointer', padding: '4px 0' }}>
                  <input type="radio" name="cron" checked={selectedPreset === i} onChange={() => setSelectedPreset(i)} style={{ accentColor: colors.cyan }} />
                  <span style={{ fontSize: textSize.small, color: selectedPreset === i ? colors.text : colors.textMuted }}>{preset.label}</span>
                </label>
              ))}
            </div>
            {selectedPreset === CRON_PRESETS.length - 1 && (
              <div style={{ marginBottom: space.xl }}>
                <input value={customCron} onChange={e => setCustomCron(e.target.value)} placeholder="0 9 * * 1-5" style={{ ...fieldStyle, fontFamily: font.mono }} />
                {customCron.trim() && <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs, fontFamily: font.mono }}>Preview: {cronToEnglish(customCron)}</div>}
              </div>
            )}
          </>
        )}

        {kind === 'once' && (
          <div style={{ marginBottom: space.xl }}>
            <input type="datetime-local" value={atLocal} onChange={e => setAtLocal(e.target.value)} style={{ ...fieldStyle, fontFamily: font.mono }} />
          </div>
        )}

        {kind === 'interval' && (
          <div style={{ display: 'flex', gap: space.md, marginBottom: space.xl }}>
            <input type="number" min={1} value={intervalValue} onChange={e => setIntervalValue(e.target.value)} style={{ ...fieldStyle, width: 100 }} />
            <select value={intervalUnit} onChange={e => setIntervalUnit(e.target.value as typeof intervalUnit)} style={{ ...fieldStyle, flex: 1 }}>
              <option value="minutes">minutes</option>
              <option value="hours">hours</option>
              <option value="days">days</option>
            </select>
          </div>
        )}

        {kind !== 'interval' && (
          <div style={{ marginBottom: space.xl }}>
            <label style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, display: 'block', marginBottom: space.sm }}>Timezone (optional)</label>
            <input value={tz} onChange={e => setTz(e.target.value)} placeholder="UTC, +05:30, -08:00" style={{ ...fieldStyle, fontFamily: font.mono }} />
          </div>
        )}

        <label style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, display: 'block', marginBottom: space.sm }}>Retries on failure</label>
        <input type="number" min={0} max={10} value={maxRetries} onChange={e => setMaxRetries(e.target.value)} style={{ ...fieldStyle, marginBottom: space.xs }} />
        <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.xxl }}>0 = no retry. If retries are exhausted, the job is escalated to your Decision Inbox.</div>

      </>
    </FormModal>
  );
}
