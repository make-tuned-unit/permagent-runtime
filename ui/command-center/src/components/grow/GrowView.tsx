// Grow — the tab that follows Build. Build makes the thing; Grow takes it to
// market. Per-project GTM home: the five-pillar strategy canvas, a content
// calendar of social posts, and Henry-driven growth actions. Henry knows the
// project (Brain, people, docs, goals), so Grow drafts and schedules with
// real context. Publishing goes through Postiz (Cloud or self-hosted) as a
// separate HTTP publisher — this repo does not vendor Postiz. Each project
// connects its own Instagram / LinkedIn / X login.

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { ease, font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { api, apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { ViewHeader } from '../common/ViewHeader';
import type { Project } from '../projects/types';
import { FiLoader } from 'react-icons/fi';
import { FunnelPanel } from './FunnelPanel';
import { drainFreshness } from './analyticsFormat';
import {
  fromDatetimeLocalValue,
  groupPostsByDay,
  readMediaMeta,
  readPostMeta,
  toDatetimeLocalValue,
  type PostStatus,
  type SocialCard,
} from './calendarPosts';
import { groupActionsByCategory } from './growActionTabs';
import { codingAgentDirective } from './codingAgentDirective';
import { CODING_AGENTS, codingAgentById } from './codingAgents';
import { GrowResults } from './GrowResults';

// Appended to every Grow prompt that DRAFTS user-facing copy (value props,
// posts, outreach) so the output reads like a sharp human wrote it, not a
// chatbot. The full voice spec lives in the "humanize" builtin skill; this
// names it and inlines the top AI tells so the draft is humanized even before
// the skill loads. Strategy prompts (audience/positioning/channels) deliberately
// omit it — they produce internal analysis, not copy the user will publish.
/** How long the panel takes to fade out before the project actually changes,
 *  and to fade back in after. Long enough to read as a transition, short
 *  enough that switching never feels like waiting. */
const SWAP_FADE_MS = 140;
/** How long the outgoing height stays pinned after the swap, so a panel that
 *  is still fetching cannot collapse the scroll container under the cursor. */
const SWAP_SETTLE_MS = 600;
/** How often the Grow panel re-reads while a review is running on the server.
 *  Only ticks while `generating` is true (see `GrowActions`), so this is a
 *  progress check on a job the user started, not a background poll. */
const GENERATION_POLL_MS = 4000;

const HUMANIZE_VOICE =
  ' Write it the way a sharp person actually writes: lead with the point, stay specific and concrete, keep sentences short, and cut every AI tell (no em-dashes, no hype words like "seamless" or "leverage" or "unlock", no throat-clearing openers). Apply your "humanize" skill for the full voice spec before you hand it back.';

// The five GTM pillars (research: target market · value prop · pricing &
// positioning · channels · integrated marketing) — the strategy spine every
// launch needs. Each is a Henry-assisted prompt seed.
const PILLARS: { key: string; label: string; prompt: (p: string) => string; hint: string }[] = [
  {
    key: 'audience',
    label: 'Audience',
    hint: 'Who is this for, and where do they already gather?',
    prompt: (p) => `For the project "${p}", define the target audience: the specific people who need this, their watering holes (subreddits, communities, hashtags), and the one persona to lead with. Use what you know from the project's Brain, people, and docs.`,
  },
  {
    key: 'value',
    label: 'Value proposition',
    hint: 'The one sentence that makes them care.',
    prompt: (p) => `Draft 3 one-line value propositions for "${p}" — the sharp promise that makes the target audience stop scrolling. Ground them in the project's actual capabilities.${HUMANIZE_VOICE}`,
  },
  {
    key: 'positioning',
    label: 'Positioning & price',
    hint: 'Against what, and for how much?',
    prompt: (p) => `For "${p}", propose positioning against the 2-3 real alternatives people use today, and a pricing hypothesis (free/paid tiers) that fits the audience.`,
  },
  {
    key: 'channels',
    label: 'Channels',
    hint: 'The 2-3 places to show up, not all of them.',
    prompt: (p) => `Recommend the 2-3 highest-leverage launch channels for "${p}" (e.g. a specific subreddit, X, a newsletter, a directory) and why each fits this audience — not a generic list.`,
  },
  {
    key: 'workback',
    label: 'Workback schedule',
    hint: 'Milestones counting back from launch day.',
    prompt: (p) => `Build a workback schedule for "${p}" from its launch date: the dated milestones between now and launch, working backwards.`,
  },
  {
    key: 'content',
    label: 'Content & launch',
    hint: 'The hub piece and the posts that orbit it.',
    prompt: (p) => `For "${p}", outline the launch content: one substantial hub piece (a guide/thread that establishes authority) and a week of social posts that link back to it. Draft the first post so I can schedule it.${HUMANIZE_VOICE}`,
  },
];

// ── Saved strategy (metadata_json.strategy — #13) ────────────────────────────
export interface SavedPillar {
  content: string;
  updated_at?: string;
  /** Labeled bullets [{label, detail}] — rendered as the card's rich body. */
  points?: Array<{ label: string; detail: string }>;
  /** Stat chips [{label, value}] — rendered as a metric row. */
  metrics?: Array<{ label: string; value: string }>;
}

/** Tolerant read of a saved pillar from the project's metadata bag. */
export function readStrategy(project: Project, key: string): SavedPillar | null {
  const strategy = (project.metadataJson as { strategy?: Record<string, unknown> } | null)?.strategy;
  const raw = strategy?.[key] as { content?: unknown; updated_at?: unknown } | undefined;
  if (!raw || typeof raw.content !== 'string' || !raw.content.trim()) return null;
  const pairs = (v: unknown, a: string, b: string) =>
    Array.isArray(v)
      ? (v as Array<Record<string, unknown>>)
          .filter(item => typeof item?.[a] === 'string' && typeof item?.[b] === 'string')
          .map(item => ({ [a]: item[a] as string, [b]: item[b] as string }))
      : undefined;
  const rawAny = raw as Record<string, unknown>;
  return {
    content: raw.content,
    updated_at: typeof raw.updated_at === 'string' ? raw.updated_at : undefined,
    points: pairs(rawAny.points, 'label', 'detail') as SavedPillar['points'],
    metrics: pairs(rawAny.metrics, 'label', 'value') as SavedPillar['metrics'],
  };
}

async function saveStrategy(projectId: string, pillar: string, content: string): Promise<void> {
  await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/strategy/${encodeURIComponent(pillar)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
}

/** Run-all: one turn where Henry produces and SAVES every pillar. */
function runAllPrompt(projectName: string): string {
  return `Build the complete go-to-market strategy for "${projectName}" using everything you know about the project (Brain, people, docs, goals). Work through all five pillars — audience, value, positioning, channels, content — and for EACH one, save your result with the set_project_strategy tool (project: "${projectName}", pillar: "<key>"): content = a 2-3 sentence summary, points = [{label, detail}] labeled specifics (personas with watering holes, channels with fit reasons, alternatives with your counter-positioning), metrics = [{label, value}] stat chips (price hypothesis, audience size, post cadence). The Strategy cards render this as rich content, so fill all three fields. Also save the "workback" pillar: the launch workback schedule — points = [{label: "<date or week>", detail: "<milestone>"}] counting back from launch day. THEN save this project's brand with set_project_brand (voice, origin story of why it was built, palette from its real product if you know it, donts). THEN turn the workback into real to-dos: create a Kanban card on this project's board for each concrete milestone with the card_create tool (title = the milestone, description = why it matters and its target week). Finish with a one-paragraph summary.${HUMANIZE_VOICE}`;
}

function draftPostPrompt(projectName: string): string {
  return `For "${projectName}", call social_content_brief first and draft from THAT project's brief only — a top-performing page, a newly completed goal/feature, or the saved origin story. Create it as a social_post with card_create: title = the hook, description = the post body, post_status = "draft", harvest_kind set, format and channel that fit. Omit scheduled_for so the daemon picks the send time. A still matching this post generates automatically; do not set scheduled yourself.${HUMANIZE_VOICE}`;
}

function brandPrompt(projectName: string): string {
  return `For "${projectName}", save this project's brand kit with set_project_brand: voice (how it writes), origin (why it was built, quoted from this project), bg/fg/accent as #RRGGBB from its real site or product UI if you know them, and donts for generated media. Use only this project — do not copy another project's kit.`;
}

interface ProjectBrand {
  voice: string;
  origin: string;
  bg: string;
  fg: string;
  accent: string;
}

function readBrand(project: Project): ProjectBrand {
  const raw = (project.metadataJson as { brand?: Record<string, unknown> } | null)?.brand;
  const s = (k: string) => (typeof raw?.[k] === 'string' ? (raw[k] as string) : '');
  return { voice: s('voice'), origin: s('origin'), bg: s('bg'), fg: s('fg'), accent: s('accent') };
}

async function saveBrand(projectId: string, brand: ProjectBrand): Promise<void> {
  await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/brand`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(brand),
  });
}

// The deterministic growth inbox (backend GET /api/projects/:id/growth-inbox).
// Ranked with NO LLM from the project's real signals — matches the Rust
// response (camelCase). See crates/goose-server/src/routes/grow.rs.
type MovePriority = 'high' | 'medium' | 'low';
interface GrowthMove {
  title: string;
  why: string;
  priority: MovePriority;
  evidenceCount: number;
}
interface GrowthWin {
  title: string;
  why: string;
}
interface GrowthInboxData {
  moves: GrowthMove[];
  wins: GrowthWin[];
  signal: { posts: number; shipped: number; activeGoals: number; daysSinceLastPost: number | null };
}

// ── Analytics connection (backend routes/grow_analytics.rs) ──────────────────
// Ruled decision (2026-07-20): the analytics lens is an API CLIENT to an
// existing web-analytics account (read-only stats fetch), not a self-hosted
// collector. Wire types mirror the Rust responses (camelCase).
type AnalyticsProviderId = 'plausible' | 'plausible_v2' | 'goatcounter';
const PROVIDER_LABELS: Record<AnalyticsProviderId, string> = {
  plausible: 'Plausible (v1 · CE)',
  plausible_v2: 'Plausible Cloud (v2)',
  goatcounter: 'GoatCounter',
};
interface AnalyticsConnectionStatus {
  connected: boolean;
  provider: AnalyticsProviderId | null;
  baseUrl: string | null;
  siteId: string | null;
  /** Whether a key is stored server-side — the key itself is never sent back. */
  hasApiKey: boolean;
}
interface AnalyticsStatsData {
  connected: boolean;
  provider: AnalyticsProviderId | null;
  periodDays: number | null;
  visitors: number | null;
  pageviews: number | null;
  /** Fetch failures (provider down, bad key, sovereign mode) arrive here — honest, never faked. */
  error: string | null;
}
interface AnalyticsTestResult {
  ok: boolean;
  visitors: number | null;
  pageviews: number | null;
  error: string | null;
}

// First-party analytics (#23) — the daemon is the collector; no third party.
// Backend: routes/first_party_analytics.rs.
interface FirstPartySetup {
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
interface VerifyCheck {
  id: string;
  label: string;
  passed: boolean;
  detail: string;
}

interface VerifyResponse {
  verified: boolean;
  checks: VerifyCheck[];
  summary: string;
}

interface FirstPartyStats {
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

type GrowLens = 'actions' | 'results' | 'strategy' | 'calendar' | 'analytics';
// Async lifecycle for data-backed sections — loading / ready / error are
// distinct so a fetch failure never masquerades as an empty result.
type LoadState = 'loading' | 'ready' | 'error';

// Actions leads: the point of collecting analytics is deciding what to do.
// Results sits next to it so "what I did" is as reachable as "what to do".
const LENSES: GrowLens[] = ['actions', 'results', 'strategy', 'calendar', 'analytics'];
const LENS_LABELS: Record<GrowLens, string> = {
  actions: 'Actions',
  results: 'Results',
  strategy: 'Strategy',
  calendar: 'Calendar',
  analytics: 'Analytics',
};

export function GrowView() {
  const { colors, gradient, reduceMotion } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectsState, setProjectsState] = useState<LoadState>('loading');
  const [activeId, setActiveId] = useState<string | null>(null);
  const [posts, setPosts] = useState<SocialCard[]>([]);
  const [postsState, setPostsState] = useState<LoadState>('loading');
  const [postsMutationError, setPostsMutationError] = useState<string | null>(null);
  const [lens, setLens] = useState<GrowLens>('actions');
  const [ctx, setCtx] = useState<{ people: number; goals: number } | null>(null);
  const [focusLens, setFocusLens] = useState<GrowLens | null>(null);
  const postsRequestGeneration = useRef(0);
  // Project switching. Every panel refetches at once, so a bare `setActiveId`
  // is a hard cut: the whole column drops to its loading states in one frame
  // and springs back when the slowest request lands. Fading out BEFORE the
  // switch — which we can do because we own the trigger — means the swap and
  // the loading states happen while nothing is visible, and the new project
  // arrives as one smooth rise instead of a flash.
  const [swapping, setSwapping] = useState(false);
  // What the user has chosen but the panel has not caught up to yet. The
  // dropdown is bound to this, not to `activeId` — a control that springs back
  // to the old value for the length of the fade reads as the app arguing with
  // the click, which is worse than the flash we came here to remove.
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [pinnedHeight, setPinnedHeight] = useState<number | undefined>(undefined);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const swapTimer = useRef<ReturnType<typeof setTimeout>>();
  const setActivePanel = useCommandCenter((st) => st.setActivePanel);
  const sendMessage = useCommandCenter((st) => st.sendMessage);
  const openChatDock = useCommandCenter((st) => st.openChatDock);
  const openGrowForProject = useCommandCenter((st) => st.openGrowForProject);
  const setOpenGrowForProject = useCommandCenter((st) => st.setOpenGrowForProject);
  const openGrowLens = useCommandCenter((st) => st.openGrowLens);
  const setOpenGrowLens = useCommandCenter((st) => st.setOpenGrowLens);
  const setPendingProjectNavigation = useCommandCenter((st) => st.setPendingProjectNavigation);

  const loadProjects = useCallback(() => {
    setProjectsState('loading');
    apiFetch<Project[]>('/api/projects')
      .then((ps) => {
        const real = ps.filter((p) => p.status !== 'archived');
        setProjects(real);
        setActiveId((cur) => cur ?? real[0]?.id ?? null);
        setProjectsState('ready');
      })
      .catch(() => setProjectsState('error'));
  }, []);

  // projectsRev bumps on project_changed — a strategy save (from the UI or
  // Henry's set_project_strategy tool) refreshes the cards live.
  const projectsRev = useCommandCenter((st) => st.projectsRev);
  // The primary agent's configured display name — identity is config, never a literal (#986).
  const agentName = useCommandCenter((st) => st.agentName);
  useEffect(() => { loadProjects(); }, [loadProjects, projectsRev]);

  // Content calendar = social_post cards on this project (reserved card type
  // already exists; empty until Henry/the user create them).
  const loadPosts = useCallback((id: string, opts?: { silent?: boolean }) => {
    const generation = ++postsRequestGeneration.current;
    // Background refreshes keep the current list on screen (no loading flash);
    // only user-visible (re)loads show the loading state.
    if (!opts?.silent) setPostsState('loading');
    apiFetch<SocialCard[]>(`/api/projects/${encodeURIComponent(id)}/cards?card_type=social_post`)
      .then((p) => {
        if (generation !== postsRequestGeneration.current) return;
        setPosts(p);
        setPostsState('ready');
      })
      .catch(() => {
        if (generation !== postsRequestGeneration.current) return;
        if (opts?.silent) {
          // Keep a list that's already on screen. If this was racing the
          // first load, do not leave the calendar stuck on "Loading posts…".
          setPostsState((s) => (s === 'loading' ? 'error' : s));
          return;
        }
        setPosts([]);
        setPostsState('error');
      });
  }, []);

  useEffect(() => {
    if (!activeId) return;
    loadPosts(activeId);
    return () => { ++postsRequestGeneration.current; };
  }, [activeId, loadPosts]);

  // project_changed (brand/strategy save, media job finished) refreshes
  // the calendar without a loading flash. Skip the initial stamp — the
  // activeId effect already loaded.
  const seenPostsRev = useRef(projectsRev);
  useEffect(() => {
    if (!activeId) return;
    if (seenPostsRev.current === projectsRev) return;
    seenPostsRev.current = projectsRev;
    loadPosts(activeId, { silent: true });
  }, [projectsRev, activeId, loadPosts]);

  // PATCH/DELETE /api/projects/:id/cards/:cardId — confirmed paths in routes/cards.rs
  // (patch + delete on the same resource; there is no PUT /api/cards/:id).
  const mutatePost = useCallback(async (
    projectId: string,
    post: SocialCard,
    body: Record<string, unknown> | null,
  ) => {
    setPostsMutationError(null);
    const path = `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}`;
    try {
      if (body === null) {
        await apiFetch(path, { method: 'DELETE' });
      } else {
        await apiFetch(path, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });
      }
      loadPosts(projectId, { silent: true });
    } catch (e) {
      const msg = e instanceof Error ? e.message : 'Could not update the post.';
      setPostsMutationError(msg);
      throw e;
    }
  }, [loadPosts]);

  // Keep the Content calendar live while it's on screen: "+ Draft a post with
  // Henry" hands off to chat, and before this poll the drafted social_post
  // card never appeared until the user switched projects and back (2026-07
  // wiring audit — persist-but-no-readback). Same 15s stale-while-revalidate
  // cadence the dashboard uses.
  useEffect(() => {
    if (!activeId || lens !== 'calendar') return;
    const t = setInterval(() => loadPosts(activeId, { silent: true }), 15_000);
    return () => clearInterval(t);
  }, [activeId, lens, loadPosts]);

  const active = projects.find((p) => p.id === activeId) ?? null;

  /** Every project change goes through here — the dropdown and the cross-tab
   *  deep link alike, so one of them can never feel different from the other. */
  const switchProject = useCallback((id: string) => {
    if (!id || id === activeId) return;
    clearTimeout(swapTimer.current);
    if (reduceMotion) { setActiveId(id); return; }
    // Hold the height we are leaving so the scroll container cannot lurch
    // while the new panel is still empty.
    setPinnedHeight(panelRef.current?.offsetHeight);
    setPendingId(id);
    setSwapping(true);
    swapTimer.current = setTimeout(() => {
      setActiveId(id);
      setPendingId(null);
      setSwapping(false);
      // Release the pin once the new content has had time to lay out.
      swapTimer.current = setTimeout(() => setPinnedHeight(undefined), SWAP_SETTLE_MS);
    }, SWAP_FADE_MS);
  }, [activeId, reduceMotion]);

  useEffect(() => () => clearTimeout(swapTimer.current), []);

  // Honor a cross-tab deep link (Projects → Grow this project), then CLEAR it
  // (the pendingProjectNavigation consume-then-clear pattern). Without the
  // clear, one agent-driven grow open stuck in the store forever: every later
  // manual Grow visit re-selected that project on remount, and a repeat
  // open for the same project was a silent no-op (same value → no re-render).
  useEffect(() => {
    if (openGrowForProject) {
      switchProject(openGrowForProject);
      setOpenGrowForProject(null);
    }
  }, [openGrowForProject, setOpenGrowForProject, switchProject]);

  useEffect(() => {
    if (!openGrowLens) return;
    setLens(openGrowLens);
    setOpenGrowLens(null);
  }, [openGrowLens, setOpenGrowLens]);

  // Real project context — Grow feels connected because it shows the project's
  // actual state (people, shipped work), not a blank canvas.
  useEffect(() => {
    if (!activeId) { setCtx(null); return; }
    let alive = true;
    (async () => {
      const [people, cards] = await Promise.all([
        apiFetch<unknown[]>(`/api/projects/${encodeURIComponent(activeId)}/people`).catch(() => []),
        apiFetch<{ card_type: string }[]>(`/api/projects/${encodeURIComponent(activeId)}/cards`).catch(() => []),
      ]);
      if (!alive) return;
      // Count of goal cards in ANY state — labeled "goals", not "shipped"
      // (2026-07 wiring audit: the old "N shipped" label counted in-progress
      // and triage cards as shipped work).
      const goals = cards.filter((c) => c.card_type === 'goal').length;
      setCtx({ people: people.length, goals });
    })();
    return () => { alive = false; };
  }, [activeId]);

  // One-click hand-off: surface chat and send the GTM prompt directly to Henry,
  // grounded in the selected project (the Discuss-with-Henry pattern). No
  // clipboard, no tab hunting.
  const send = (prompt: string) => {
    // Open the chat dock explicitly. setActivePanel('chat') only dismisses any
    // overlay — since chat went dock-first it does NOT surface Henry, so these
    // cards looked dead: the prompt was sent to a chat nobody could see.
    setActivePanel('chat');
    openChatDock();
    void sendMessage(prompt);
  };

  // Close the one-way door: Projects → Grow deep-links in (openGrowForProject),
  // but Grow had no way back. Return to this project in the Projects tab, reusing
  // the pendingProjectNavigation seam ProjectsView consumes (mirrors BrainView's
  // "Open project"). No new store seam.
  const openInProjects = useCallback(() => {
    if (!activeId) return;
    setPendingProjectNavigation(activeId);
    navigateToTool('projects');
  }, [activeId, setPendingProjectNavigation]);

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      {/* Header + project switcher. The title/subtitle come from ViewHeader so
          Grow wears the same header as Home, Projects, Automate and Build —
          this view used to hand-roll a 16px title against the ramp's 20px
          `type.title`, which read as a visibly smaller heading. The wrapper
          exists only to carry Grow's brand ribbon, which ViewHeader has no
          slot for; it must stay `position: relative` for the ribbon to anchor. */}
      <div style={{ position: 'relative', flexShrink: 0 }}>
        <div style={{ position: 'absolute', left: 0, right: 0, bottom: -1, height: 2, background: `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`, opacity: 0.5, zIndex: 1 }} />
        <ViewHeader
          title="Grow"
          subtitle={
            <span style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
              <span>Take {active ? active.name : 'your project'} to market — {agentName} drafts with the project's real context.</span>
              {active?.siteUrl && (
                <a href={active.siteUrl} target="_blank" rel="noreferrer" style={{ color: colors.cyan, textDecoration: 'none' }}>site ↗</a>
              )}
              {active?.repoUrl && (
                <a href={active.repoUrl} target="_blank" rel="noreferrer" style={{ color: colors.cyan, textDecoration: 'none' }}>repo ↗</a>
              )}
              {active && (
                <button
                  type="button"
                  onClick={openInProjects}
                  title={`Open ${active.name} in Projects`}
                  style={{
                    color: colors.cyan, background: 'none', border: 'none', padding: 0,
                    cursor: 'pointer', font: 'inherit',
                  }}
                >open project ↗</button>
              )}
              {/* Always rendered so the count fades in rather than popping the
                  header line around on every project change. */}
              <span style={{
                color: colors.textDim,
                opacity: ctx ? 1 : 0,
                transition: reduceMotion ? undefined : `opacity 220ms ${ease.out}`,
              }}>
                {ctx && `${ctx.goals} ${ctx.goals === 1 ? 'goal' : 'goals'} · ${ctx.people} ${ctx.people === 1 ? 'person' : 'people'}`}
              </span>
            </span>
          }
          actions={<>
        {/* VIEW axis — segmented tab toggle (mirrors the Kanban/overview toggle) */}
        <div role="tablist" aria-label="Grow view" style={{ display: 'flex', gap: 2, background: colors.bgDeeper, borderRadius: radius.md, padding: 2 }}>
          {LENSES.map((l) => {
            const selected = lens === l;
            return (
              <button
                key={l}
                role="tab"
                aria-selected={selected}
                tabIndex={0}
                onClick={() => setLens(l)}
                onFocus={() => setFocusLens(l)}
                onBlur={() => setFocusLens(null)}
                style={{
                  fontSize: 12, fontFamily: font.body,
                  padding: '5px 12px', borderRadius: radius.sm, cursor: 'pointer', border: 'none',
                  background: selected ? colors.cyanSoft : 'transparent',
                  color: selected ? colors.cyan : colors.textMuted,
                  fontWeight: selected ? 600 : 500,
                  outline: 'none',
                  boxShadow: focusLens === l ? `0 0 0 2px ${colors.borderHi}` : 'none',
                  transition: reduceMotion ? 'none' : 'background 150ms ease, color 150ms ease',
                }}
              >{LENS_LABELS[l]}</button>
            );
          })}
        </div>
        <select
          value={pendingId ?? activeId ?? ''}
          onChange={(e) => switchProject(e.target.value)}
          aria-label="Select project"
          style={{
            background: colors.bgDeeper, color: colors.text, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: '6px 10px', fontSize: 13, fontFamily: font.body,
          }}
        >
          {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
          </>}
        />
      </div>

      {projectsState === 'error' ? (
        <ErrorState
          colors={colors}
          message="Couldn't load your projects."
          onRetry={loadProjects}
        />
      ) : projectsState === 'loading' && projects.length === 0 ? (
        <LoadingState colors={colors} label="Loading projects…" />
      ) : active ? (
        <div
          ref={panelRef}
          role="tabpanel"
          aria-label={`${lens} view`}
          aria-busy={swapping}
          style={{
            flex: 1, overflowY: 'auto', padding: '20px 24px',
            display: 'flex', flexDirection: 'column', gap: 20,
            minHeight: pinnedHeight,
            opacity: swapping ? 0 : 1,
            transition: reduceMotion ? undefined : `opacity ${SWAP_FADE_MS}ms ${ease.out}`,
          }}
        >
          {/* Keyed for the same reason the analytics panels are: the load
              effect refetches on project.id but never clears local state, so
              without a remount project A's verify results and outcomes stay on
              screen over project B's cards for as long as the refetch is in
              flight. That leak was a reported bug on the analytics panels
              (2026-08-04) — see analyticsPanelScope.test.ts. */}
          {lens === 'actions' && <GrowActions key={active.id} project={active} colors={colors} />}
          {lens === 'results' && <GrowResults key={`${active.id}-results`} project={active} colors={colors} />}
          {lens === 'analytics' && <GrowAnalytics project={active} posts={posts} colors={colors} />}
          {lens === 'strategy' && (
          <section>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', margin: '0 0 12px' }}>
              <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>Go-to-market strategy</h3>
              <button
                onClick={() => send(runAllPrompt(active.name))}
                title={`${agentName} researches every pillar and fills these cards with the results`}
                style={{
                  fontSize: 12, fontFamily: font.body, fontWeight: 600,
                  color: colors.cyan, background: colors.cyanSoft,
                  border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
                  padding: '6px 14px', cursor: 'pointer',
                }}
              >✦ Generate</button>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 12 }}>
              {PILLARS.map((pillar) => (
                <PillarCard
                  key={pillar.key}
                  pillarKey={pillar.key}
                  label={pillar.label}
                  hint={pillar.hint}
                  colors={colors}
                  saved={readStrategy(active, pillar.key)}
                  onSave={(content) => saveStrategy(active.id, pillar.key, content)}
                />
              ))}
              <BrandCard
                colors={colors}
                brand={readBrand(active)}
                onAsk={() => send(brandPrompt(active.name))}
                onSave={(next) => saveBrand(active.id, next)}
                agentName={agentName}
              />
            </div>
          </section>
          )}

          {lens === 'calendar' && (
          <section>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, margin: '0 0 12px' }}>
              <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>Content calendar</h3>
              <span style={{ fontSize: 10, color: colors.textDim, background: colors.bgDeeper, padding: '1px 6px', borderRadius: radius.pill, fontVariantNumeric: 'tabular-nums' }}>{posts.length}</span>
              <div style={{ flex: 1 }} />
              <button
                onClick={() => send(draftPostPrompt(active.name))}
                style={{
                  fontSize: 11, fontFamily: font.body, color: colors.text,
                  background: 'transparent', border: `1px solid ${colors.border}`,
                  borderRadius: radius.md, padding: '5px 12px', cursor: 'pointer',
                }}
              >+ Draft a post with {agentName}</button>
            </div>
            <HiggsfieldConnect colors={colors} />
            <PostizConnect colors={colors} />
            <ProjectChannels projectId={active.id} colors={colors} />
            {postsMutationError && (
              <div role="alert" style={{
                fontSize: 12, color: colors.danger, marginBottom: 10,
                background: colors.bgDeeper, border: `1px solid ${colors.border}`,
                borderRadius: radius.md, padding: '8px 10px',
              }}>
                Couldn&apos;t save changes: {postsMutationError}
              </div>
            )}
            {postsState === 'error' ? (
              <ErrorState
                colors={colors}
                inline
                message="Couldn't load the content calendar."
                onRetry={() => loadPosts(active.id)}
              />
            ) : postsState === 'loading' ? (
              <LoadingState colors={colors} inline label="Loading posts…" />
            ) : posts.length === 0 ? (
              <div style={{
                border: `1px dashed ${colors.border}`, borderRadius: radius.lg, padding: 28,
                textAlign: 'center', fontSize: 12, color: colors.textDim,
              }}>
                No posts yet. Draft one with {agentName} above — it is written in this project's voice, a still is generated, and Approve schedules it on this project's connected accounts when you are ready.
              </div>
            ) : (
              <CalendarLens
                projectId={active.id}
                posts={posts}
                colors={colors}
                onMutate={mutatePost}
                onReload={() => loadPosts(active.id, { silent: true })}
              />
            )}
          </section>
          )}
        </div>
      ) : (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: colors.textDim, fontSize: 13 }}>
          Create a project in the Projects tab, then grow it here.
        </div>
      )}
    </div>
  );
}

// ── Strategy pillar card ─────────────────────────────────────────────────────
// The whole card is the interactive surface (mirrors DecisionsCard): clickable,
// keyboard-operable (Enter/Space), with hover + focus affordances. The "Ask
// Henry" chip is a visual cue, not a nested control.
// Feather-style icon per pillar — the card's identity at a glance.
const PILLAR_ICONS: Record<string, string> = {
  audience: 'M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M9 11a4 4 0 100-8 4 4 0 000 8zM23 21v-2a4 4 0 00-3-3.87M16 3.13a4 4 0 010 7.75',
  value: 'M13 2L3 14h9l-1 8 10-12h-9l1-8z',
  positioning: 'M12 22a10 10 0 100-20 10 10 0 000 20zM12 18a6 6 0 100-12 6 6 0 000 12zM12 14a2 2 0 100-4 2 2 0 000 4z',
  channels: 'M18 8a3 3 0 100-6 3 3 0 000 6zM6 15a3 3 0 100-6 3 3 0 000 6zM18 22a3 3 0 100-6 3 3 0 000 6zM8.6 13.5l6.8 3.5M15.4 6.5l-6.8 3.5',
  content: 'M12 20h9M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4L16.5 3.5z',
  workback: 'M8 2v4M16 2v4M3 10h18M5 4h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V6a2 2 0 012-2z',
};

/** Strategy pillar card — display + edit only (#22). Generation is the single
 *  ✦ Generate button on the lens header; per-card Ask-Henry chips are gone.
 *  A saved pillar renders rich: summary, labeled points, stat chips. */
function PillarCard({
  pillarKey, label, hint, colors, saved, onSave,
}: {
  pillarKey: string;
  label: string;
  hint: string;
  colors: ThemeColors;
  /** Persisted strategy for this pillar (metadata_json.strategy), if any. */
  saved: SavedPillar | null;
  onSave: (content: string) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);

  const commit = async () => {
    const content = draft.trim();
    if (!content) { setEditing(false); return; }
    setSaving(true);
    try {
      await onSave(content); // project_changed → projectsRev → cards refresh
      setEditing(false);
    } catch {
      setSaveError(true);
    } finally {
      setSaving(false);
    }
  };

  const shell: CSSProperties = {
    background: colors.surface, backdropFilter: 'blur(24px) saturate(140%)',
    border: `1px solid ${saved ? colors.borderHi : colors.border}`,
    borderRadius: radius.lg, padding: 16,
    display: 'flex', flexDirection: 'column', gap: 10, minHeight: 120,
  };

  if (editing) {
    return (
      <div style={shell}>
        <div style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text }}>{label}</div>
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          autoFocus
          rows={6}
          style={{
            width: '100%', resize: 'vertical', fontSize: 12, lineHeight: 1.5,
            fontFamily: font.body, color: colors.text, background: 'transparent',
            border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: 8,
            outline: 'none',
          }}
        />
        {saveError && <span style={{ fontSize: 11, color: colors.danger }}>Couldn't save — try again.</span>}
        <div style={{ display: 'flex', gap: 8 }}>
          <button onClick={() => void commit()} disabled={saving} style={{
            fontSize: 11, fontFamily: font.body, fontWeight: 600, color: colors.cyan,
            background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
            borderRadius: radius.md, padding: '5px 12px', cursor: 'pointer', opacity: saving ? 0.6 : 1,
          }}>{saving ? 'Saving…' : 'Save'}</button>
          <button onClick={() => setEditing(false)} style={{
            fontSize: 11, fontFamily: font.body, color: colors.textMuted,
            background: 'transparent', border: 'none', cursor: 'pointer',
          }}>Cancel</button>
        </div>
      </div>
    );
  }

  return (
    <div style={shell}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0 }}
          stroke={saved ? colors.cyan : colors.textMuted} strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round">
          <path d={PILLAR_ICONS[pillarKey] ?? PILLAR_ICONS.value} />
        </svg>
        <span style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text, flex: 1 }}>{label}</span>
        {saved && (
          <button
            onClick={() => { setDraft(saved.content); setSaveError(false); setEditing(true); }}
            title={saved.updated_at ? `Saved ${new Date(saved.updated_at).toLocaleString()}` : 'Edit'}
            style={{
              fontSize: 10, fontFamily: font.body, color: colors.textMuted,
              background: 'transparent', border: 'none', cursor: 'pointer', padding: 0,
            }}
          >Edit</button>
        )}
      </div>

      {saved ? (
        <>
          <div style={{
            fontSize: 12, color: colors.text, lineHeight: 1.55,
            whiteSpace: 'pre-wrap', overflowWrap: 'break-word',
          }}>{saved.content}</div>

          {saved.points && saved.points.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
              {saved.points.map((pt, i) => (
                <div key={i} style={{ display: 'flex', gap: 7, fontSize: 11.5, lineHeight: 1.45 }}>
                  <span style={{ color: colors.cyan, flexShrink: 0 }}>▸</span>
                  <span style={{ color: colors.textMuted, overflowWrap: 'break-word', minWidth: 0 }}>
                    <span style={{ color: colors.text, fontWeight: 600 }}>{pt.label}</span>
                    {' — '}{pt.detail}
                  </span>
                </div>
              ))}
            </div>
          )}

          {saved.metrics && saved.metrics.length > 0 && (
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 'auto' }}>
              {saved.metrics.map((m, i) => (
                <span key={i} title={m.label} style={{
                  // Chips must never exceed the card: wrap long label·value
                  // pairs inside the pill instead of bleeding across the grid.
                  fontSize: 10.5, fontFamily: font.mono, lineHeight: 1.4,
                  maxWidth: '100%', overflowWrap: 'anywhere',
                  color: colors.cyan, background: colors.cyanSoft,
                  border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
                  padding: '3px 8px',
                }}>
                  <span style={{ color: colors.textMuted }}>{m.label} · </span>{m.value}
                </span>
              ))}
            </div>
          )}
        </>
      ) : (
        <>
          <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5, flex: 1 }}>{hint}</div>
          <span style={{ fontSize: 11, color: colors.textDim, fontFamily: font.body }}>
            ✦ Generate fills this in
          </span>
        </>
      )}
    </div>
  );
}

function BrandCard({
  colors, brand, onAsk, onSave, agentName,
}: {
  colors: ThemeColors;
  brand: ProjectBrand;
  onAsk: () => void;
  onSave: (brand: ProjectBrand) => Promise<void>;
  agentName: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(brand);
  const [saving, setSaving] = useState(false);
  const filled = !!(brand.voice || brand.origin || brand.bg);
  const shell: CSSProperties = {
    background: colors.surface, border: `1px solid ${filled ? colors.borderHi : colors.border}`,
    borderRadius: radius.lg, padding: 16, display: 'flex', flexDirection: 'column', gap: 10, minHeight: 120,
  };
  const field: CSSProperties = {
    width: '100%', fontSize: 12, fontFamily: font.body, color: colors.text,
    background: colors.bgDeeper, border: `1px solid ${colors.border}`, borderRadius: radius.sm,
    padding: '6px 8px', boxSizing: 'border-box',
  };
  if (editing) {
    return (
      <div style={shell}>
        <div style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text }}>Brand</div>
        <textarea value={draft.voice} onChange={(e) => setDraft({ ...draft, voice: e.target.value })} placeholder="Voice" rows={3} style={field} />
        <textarea value={draft.origin} onChange={(e) => setDraft({ ...draft, origin: e.target.value })} placeholder="Why this was built" rows={3} style={field} />
        <div style={{ display: 'flex', gap: 6 }}>
          <input value={draft.bg} onChange={(e) => setDraft({ ...draft, bg: e.target.value })} placeholder="#bg" aria-label="Background hex" style={field} />
          <input value={draft.fg} onChange={(e) => setDraft({ ...draft, fg: e.target.value })} placeholder="#fg" aria-label="Foreground hex" style={field} />
          <input value={draft.accent} onChange={(e) => setDraft({ ...draft, accent: e.target.value })} placeholder="#accent" aria-label="Accent hex" style={field} />
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <button
            type="button"
            disabled={saving}
            onClick={() => {
              setSaving(true);
              void onSave(draft).then(() => setEditing(false)).finally(() => setSaving(false));
            }}
            style={{ fontSize: 11, fontFamily: font.body, fontWeight: 600, color: colors.cyan, background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`, borderRadius: radius.md, padding: '5px 12px', cursor: 'pointer' }}
          >{saving ? 'Saving…' : 'Save'}</button>
          <button type="button" onClick={() => setEditing(false)} style={{ fontSize: 11, fontFamily: font.body, color: colors.textMuted, background: 'transparent', border: 'none', cursor: 'pointer' }}>Cancel</button>
        </div>
      </div>
    );
  }
  return (
    <div style={shell}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <div style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text }}>Brand</div>
        <div style={{ flex: 1 }} />
        <button type="button" onClick={onAsk} style={{ fontSize: 11, fontFamily: font.body, color: colors.text, background: 'transparent', border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '4px 10px', cursor: 'pointer' }}>Ask {agentName}</button>
        <button type="button" onClick={() => { setDraft(brand); setEditing(true); }} style={{ fontSize: 11, fontFamily: font.body, color: colors.text, background: 'transparent', border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '4px 10px', cursor: 'pointer' }}>Edit</button>
      </div>
      {filled ? (
        <>
          {brand.voice && <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>{brand.voice}</div>}
          {brand.origin && <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>{brand.origin}</div>}
          <div style={{ display: 'flex', gap: 6 }}>
            {[['bg', brand.bg], ['fg', brand.fg], ['accent', brand.accent]].filter(([, v]) => v).map(([k, v]) => (
              <span key={k} style={{ fontSize: 10, fontFamily: font.mono, color: colors.textDim }}>{k} {v}</span>
            ))}
          </div>
        </>
      ) : (
        <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>
          Voice, palette, and why this project was built. Empty until you save a kit for this project — nothing is shared across projects.
        </div>
      )}
    </div>
  );
}

type ChannelBinding = {
  integrationId?: string;
  identifier?: string;
  name?: string;
  profile?: string;
};

type PublisherSnap = {
  configured: boolean;
  baseUrl?: string;
  channels?: Record<string, ChannelBinding>;
  pending?: { channel: string } | null;
};

const NETWORKS: { id: string; label: string }[] = [
  { id: 'ig', label: 'Instagram' },
  { id: 'li', label: 'LinkedIn' },
  { id: 'x', label: 'X' },
];

function PostizConnect({ colors }: { colors: ThemeColors }) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [open, setOpen] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    apiFetch<PublisherSnap>('/api/grow/postiz')
      .then((s) => setConfigured(!!s && !Array.isArray(s) && s.configured))
      .catch(() => setConfigured(false));
  }, []);
  const field: CSSProperties = {
    fontSize: 11, fontFamily: font.mono, color: colors.text, background: colors.bgDeeper,
    border: `1px solid ${colors.border}`, borderRadius: radius.sm, padding: '4px 6px',
  };
  const save = async () => {
    setBusy(true);
    try {
      const s = await apiFetch<PublisherSnap>('/api/grow/postiz', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ apiKey, baseUrl: baseUrl.trim() || undefined }),
      });
      setConfigured(s.configured);
      setOpen(false);
      setApiKey('');
    } finally { setBusy(false); }
  };
  return (
    <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 8 }}>
      Posting uses your Postiz account (Cloud by default). {configured ? 'API key saved.' : 'Not connected — Approve stays on this calendar until you save a key and log in to a network for this project.'}
      {' '}
      <button type="button" onClick={() => setOpen((v) => !v)} style={{ fontSize: 11, fontFamily: font.body, color: colors.text, background: 'transparent', border: 'none', textDecoration: 'underline', cursor: 'pointer', padding: 0 }}>
        {configured ? 'Replace key' : 'Save API key'}
      </button>
      {open && (
        <div style={{ display: 'flex', gap: 6, marginTop: 8, flexWrap: 'wrap', alignItems: 'center' }}>
          <input value={apiKey} onChange={(e) => setApiKey(e.target.value)} placeholder="Postiz API key" type="password" aria-label="Postiz API key" style={field} />
          <input value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://api.postiz.com/public/v1" aria-label="Postiz base URL" style={{ ...field, minWidth: 220 }} />
          <button type="button" disabled={busy || !apiKey.trim()} onClick={() => void save()} style={{ fontSize: 11, fontFamily: font.body, color: colors.cyan, background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`, borderRadius: radius.md, padding: '4px 10px', cursor: 'pointer' }}>Save</button>
        </div>
      )}
    </div>
  );
}

function ProjectChannels({ projectId, colors }: { projectId: string; colors: ThemeColors }) {
  const [snap, setSnap] = useState<PublisherSnap | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loginUrl, setLoginUrl] = useState<string | null>(null);
  const load = useCallback(() => {
    apiFetch<PublisherSnap>(`/api/projects/${encodeURIComponent(projectId)}/publisher`)
      .then((s) => {
        if (!s || Array.isArray(s)) return;
        setSnap(s);
        if (!s.pending) setLoginUrl(null);
      })
      .catch(() => setSnap({ configured: false, channels: {}, pending: null }));
  }, [projectId]);
  useEffect(() => { load(); }, [load]);
  useEffect(() => {
    if (!snap?.pending) return;
    const t = window.setInterval(load, 2000);
    return () => window.clearInterval(t);
  }, [snap?.pending, load]);

  const connect = async (channel: string) => {
    setBusy(channel);
    setError(null);
    try {
      const start = await apiFetch<{ url: string; channel: string; label: string }>(
        `/api/projects/${encodeURIComponent(projectId)}/publisher/connect`,
        { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ channel }) },
      );
      setLoginUrl(start.url);
      load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setBusy(null); }
  };
  const disconnect = async (channel: string) => {
    setBusy(channel);
    setError(null);
    try {
      const next = await apiFetch<PublisherSnap>(
        `/api/projects/${encodeURIComponent(projectId)}/publisher/${encodeURIComponent(channel)}`,
        { method: 'DELETE' },
      );
      setSnap(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setBusy(null); }
  };

  const channels = snap?.channels ?? {};
  const pending = snap?.pending?.channel;
  const configured = !!snap?.configured;
  const anyBound = NETWORKS.some((n) => channels[n.id]?.integrationId);

  return (
    <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 12 }}>
      <div style={{ marginBottom: 8 }}>
        {anyBound
          ? 'Approve schedules this post on the connected account for that channel.'
          : configured
            ? 'Connect Instagram, LinkedIn, or X for this project. A login window opens; after you sign in, that account is ready to post to.'
            : 'Approve parks a draft on this calendar until this project has a connected account.'}
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
        {NETWORKS.map((n) => {
          const bound = channels[n.id];
          const waiting = pending === n.id;
          const label = bound?.name || bound?.profile
            ? `${n.label} · ${bound.name || bound.profile}`
            : n.label;
          return (
            <div key={n.id} style={{ display: 'flex', alignItems: 'center', gap: 6, border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '4px 8px' }}>
              <span style={{ color: bound ? colors.text : colors.textDim }}>{waiting ? `Waiting for ${n.label} login…` : label}</span>
              {bound ? (
                <button type="button" disabled={busy === n.id} onClick={() => void disconnect(n.id)} style={{ fontSize: 11, fontFamily: font.body, color: colors.text, background: 'transparent', border: 'none', textDecoration: 'underline', cursor: 'pointer', padding: 0 }}>
                  Disconnect
                </button>
              ) : (
                <button type="button" disabled={!configured || busy === n.id} onClick={() => void connect(n.id)} style={{ fontSize: 11, fontFamily: font.body, color: colors.cyan, background: 'transparent', border: 'none', textDecoration: 'underline', cursor: 'pointer', padding: 0 }}>
                  {waiting ? 'Open login again' : `Connect ${n.label}`}
                </button>
              )}
            </div>
          );
        })}
      </div>
      {loginUrl && (
        <div style={{ marginTop: 6 }}>
          If a browser window did not open,{' '}
          <a href={loginUrl} target="_blank" rel="noreferrer" style={{ color: colors.cyan }}>open the {pending ? NETWORKS.find((n) => n.id === pending)?.label ?? '' : ''} login</a>.
        </div>
      )}
      {error && <div role="alert" style={{ color: colors.danger, marginTop: 6 }}>{error}</div>}
    </div>
  );
}

function HiggsfieldConnect({ colors }: { colors: ThemeColors }) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [open, setOpen] = useState(false);
  const [keyId, setKeyId] = useState('');
  const [secret, setSecret] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    apiFetch<{ configured: boolean }>('/api/grow/higgsfield')
      .then((s) => setConfigured(s.configured))
      .catch(() => setConfigured(false));
  }, []);
  const field: CSSProperties = {
    fontSize: 11, fontFamily: font.mono, color: colors.text, background: colors.bgDeeper,
    border: `1px solid ${colors.border}`, borderRadius: radius.sm, padding: '4px 6px',
  };
  const save = async () => {
    setBusy(true);
    try {
      const s = await apiFetch<{ configured: boolean }>('/api/grow/higgsfield', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ keyId, secret }),
      });
      setConfigured(s.configured);
      setOpen(false);
      setSecret('');
    } finally { setBusy(false); }
  };
  return (
    <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 12 }}>
      Reels use your Higgsfield account. {configured ? 'Connected.' : 'Not connected — stills still generate locally.'}
      {' '}
      <button type="button" onClick={() => setOpen((v) => !v)} style={{ fontSize: 11, fontFamily: font.body, color: colors.text, background: 'transparent', border: 'none', textDecoration: 'underline', cursor: 'pointer', padding: 0 }}>
        {configured ? 'Replace keys' : 'Connect'}
      </button>
      {open && (
        <div style={{ display: 'flex', gap: 6, marginTop: 8, flexWrap: 'wrap' }}>
          <input value={keyId} onChange={(e) => setKeyId(e.target.value)} placeholder="Key ID" aria-label="Higgsfield key id" style={field} />
          <input value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="Secret" type="password" aria-label="Higgsfield secret" style={field} />
          <button type="button" disabled={busy} onClick={() => void save()} style={{ fontSize: 11, fontFamily: font.body, color: colors.cyan, background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`, borderRadius: radius.md, padding: '4px 10px', cursor: 'pointer' }}>Save</button>
        </div>
      )}
    </div>
  );
}

// ── Shared async-state blocks ────────────────────────────────────────────────

/**
 * Placeholder cards that occupy roughly the space the real ones will.
 *
 * A one-line "Loading…" where a stack of cards is about to appear collapses
 * the column and then springs it back open — the jolt reads as a flash even
 * when the fetch is fast. Holding the shape costs nothing and the arrival
 * becomes a fill rather than a jump.
 */
function SkeletonCards({ colors, count = 2, height = 76 }: { colors: ThemeColors; count?: number; height?: number }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }} aria-hidden>
      <style>{'@keyframes pa-skeleton { 0%,100% { opacity: 0.5; } 50% { opacity: 0.85; } }'}</style>
      {Array.from({ length: count }, (_, i) => (
        <div
          key={i}
          className="pa-skeleton"
          style={{
            height, borderRadius: radius.lg,
            background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            animation: `pa-skeleton 1.6s ${ease.out} ${i * 0.12}s infinite`,
          }}
        />
      ))}
    </div>
  );
}

function CalendarLens({
  projectId, posts, colors, onMutate, onReload,
}: {
  projectId: string;
  posts: SocialCard[];
  colors: ThemeColors;
  onMutate: (projectId: string, post: SocialCard, body: Record<string, unknown> | null) => Promise<void>;
  onReload: () => void;
}) {
  const groups = groupPostsByDay(posts);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {groups.map((group) => (
        <div key={group.day}>
          <div style={{
            fontFamily: font.mono, fontSize: 11, color: colors.textDim,
            textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 8,
          }}>{group.label}</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {group.posts.map((post) => (
              <CalendarPostRow
                key={post.id}
                projectId={projectId}
                post={post}
                colors={colors}
                onMutate={onMutate}
                onReload={onReload}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function CalendarPostRow({
  projectId, post, colors, onMutate, onReload,
}: {
  projectId: string;
  post: SocialCard;
  colors: ThemeColors;
  onMutate: (projectId: string, post: SocialCard, body: Record<string, unknown> | null) => Promise<void>;
  onReload: () => void;
}) {
  const meta = readPostMeta(post);
  const media = readMediaMeta(post);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(post.title);
  const [body, setBody] = useState(post.description ?? '');
  const [when, setWhen] = useState(toDatetimeLocalValue(meta.scheduledFor));
  const [status, setStatus] = useState<PostStatus>(meta.status);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState(media.mediaFeedback);

  useEffect(() => {
    if (editing) return;
    setTitle(post.title);
    setBody(post.description ?? '');
    setWhen(toDatetimeLocalValue(meta.scheduledFor));
    setStatus(meta.status);
    setFeedback(media.mediaFeedback);
  }, [post.title, post.description, meta.scheduledFor, meta.status, media.mediaFeedback, editing]);

  const btn: CSSProperties = {
    fontSize: 11, fontFamily: font.body, color: colors.text,
    background: 'transparent', border: `1px solid ${colors.border}`,
    borderRadius: radius.md, padding: '4px 10px', cursor: busy ? 'wait' : 'pointer',
  };
  const chip: CSSProperties = {
    fontSize: 10, fontFamily: font.mono, color: colors.textDim,
    background: colors.bgDeeper, padding: '1px 6px', borderRadius: radius.pill,
    textTransform: 'uppercase', letterSpacing: '0.04em',
  };

  const saveEdit = async () => {
    setBusy(true);
    try {
      await onMutate(projectId, post, { title, description: body });
      setEditing(false);
    } catch { /* surfaced by parent */ }
    finally { setBusy(false); }
  };

  const saveSchedule = async (nextWhen: string, nextStatus: PostStatus) => {
    setBusy(true);
    setWhen(nextWhen);
    setStatus(nextStatus);
    const scheduledFor = fromDatetimeLocalValue(nextWhen);
    const metadataJson = {
      ...(post.metadataJson ?? {}),
      postStatus: nextStatus,
      ...(scheduledFor ? { scheduledFor } : {}),
    };
    if (!scheduledFor) delete (metadataJson as Record<string, unknown>).scheduledFor;
    try {
      await onMutate(projectId, post, { metadataJson });
    } catch { /* surfaced by parent */ }
    finally { setBusy(false); }
  };

  const approve = async () => {
    setBusy(true);
    try {
      await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}/approve`, {
        method: 'POST',
      });
      onReload();
    } finally { setBusy(false); }
  };

  const retryMedia = async () => {
    setBusy(true);
    try {
      await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}/media/retry`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ feedback: feedback.trim() || undefined }),
      });
      onReload();
    } catch { /* parent */ }
    finally { setBusy(false); }
  };

  const remove = async () => {
    setBusy(true);
    try { await onMutate(projectId, post, null); }
    catch { /* surfaced by parent */ }
    finally { setBusy(false); }
  };

  const canApprove = status === 'draft' && media.mediaStatus === 'ready';
  const canRetryStill = status === 'draft' && media.mediaStatus !== 'generating';

  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.md, padding: '12px 14px',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
        <span style={chip}>{status}</span>
        <span style={chip}>{media.mediaStatus}</span>
        {media.channel && <span style={chip}>{media.channel}</span>}
        {media.format && <span style={chip}>{media.format}</span>}
        <div style={{ flex: 1 }} />
        {!editing ? (
          <>
            <button type="button" style={btn} disabled={busy} onClick={() => setEditing(true)}>Edit</button>
            <button type="button" style={btn} disabled={busy} onClick={() => void remove()}>Delete</button>
          </>
        ) : (
          <>
            <button type="button" style={btn} disabled={busy} onClick={() => void saveEdit()}>Save</button>
            <button type="button" style={btn} disabled={busy} onClick={() => setEditing(false)}>Cancel</button>
          </>
        )}
      </div>
      <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
        {media.stillFile && (
          <PostStill
            projectId={projectId}
            cardId={post.id}
            filename={media.stillFile}
            cacheKey={media.mediaStatus}
            colors={colors}
          />
        )}
        <div style={{ flex: 1, minWidth: 0 }}>
      {editing ? (
        <>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            aria-label="Post title"
            style={{
              width: '100%', fontSize: 13, fontWeight: 600, fontFamily: font.body,
              color: colors.text, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '6px 8px', marginBottom: 6, boxSizing: 'border-box',
            }}
          />
          <textarea
            value={body}
            onChange={(e) => setBody(e.target.value)}
            aria-label="Post body"
            rows={3}
            style={{
              width: '100%', fontSize: 12, fontFamily: font.body, lineHeight: 1.5,
              color: colors.textMuted, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '6px 8px', resize: 'vertical', boxSizing: 'border-box',
            }}
          />
        </>
      ) : (
        <>
          <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{post.title}</div>
          {post.description && (
            <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 4, lineHeight: 1.5 }}>{post.description}</div>
          )}
        </>
      )}
        </div>
      </div>
      {media.mediaError && (
        <div style={{ fontSize: 11, color: colors.textDim, marginTop: 8 }}>{media.mediaError}</div>
      )}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, marginTop: 10, alignItems: 'center' }}>
        <label style={{ fontSize: 11, color: colors.textDim, display: 'flex', alignItems: 'center', gap: 6 }}>
          Schedule
          <input
            type="datetime-local"
            aria-label="Reschedule post"
            value={when}
            disabled={busy}
            onChange={(e) => setWhen(e.target.value)}
            onBlur={() => {
              if (when === toDatetimeLocalValue(meta.scheduledFor)) return;
              void saveSchedule(when, status);
            }}
            style={{
              fontSize: 11, fontFamily: font.body, color: colors.text,
              background: colors.bgDeeper, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '4px 6px',
            }}
          />
        </label>
        {status !== 'draft' && (
        <label style={{ fontSize: 11, color: colors.textDim, display: 'flex', alignItems: 'center', gap: 6 }}>
          Status
          <select
            aria-label="Post status"
            value={status}
            disabled={busy}
            onChange={(e) => void saveSchedule(when, e.target.value as PostStatus)}
            style={{
              fontSize: 11, fontFamily: font.body, color: colors.text,
              background: colors.bgDeeper, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '4px 6px',
            }}
          >
            <option value="scheduled">scheduled</option>
            <option value="posted">posted</option>
          </select>
        </label>
        )}
        {canApprove && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void approve()}
            style={{
              ...btn,
              color: colors.cyan,
              borderColor: colors.borderHi,
              fontWeight: 600,
            }}
          >Approve</button>
        )}
      </div>
      {canRetryStill && (
        <div style={{ display: 'flex', gap: 8, marginTop: 10, alignItems: 'flex-start' }}>
          <textarea
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            disabled={busy}
            aria-label="Still taste notes"
            placeholder="Taste notes for a new still — copy stays"
            rows={2}
            style={{
              flex: 1, fontSize: 11, fontFamily: font.body, color: colors.text,
              background: colors.bgDeeper, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '6px 8px', resize: 'vertical',
              boxSizing: 'border-box',
            }}
          />
          <button
            type="button"
            style={btn}
            disabled={busy}
            onClick={() => void retryMedia()}
          >Regenerate still</button>
        </div>
      )}
    </div>
  );
}

function PostStill({
  projectId, cardId, filename, cacheKey, colors,
}: {
  projectId: string;
  cardId: string;
  filename: string;
  cacheKey?: string;
  colors: ThemeColors;
}) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    let objectUrl: string | null = null;
    api.fetchGrowMediaBlob(projectId, cardId, filename)
      .then((blob) => {
        if (!live) return;
        objectUrl = URL.createObjectURL(blob);
        setUrl(objectUrl);
      })
      .catch(() => { /* still generating or missing */ });
    return () => {
      live = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [projectId, cardId, filename, cacheKey]);
  if (!url) {
    return (
      <div style={{
        width: 72, height: 90, borderRadius: radius.sm,
        background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      }} />
    );
  }
  return (
    <img
      src={url}
      alt=""
      style={{
        width: 72, height: 90, objectFit: 'cover', borderRadius: radius.sm,
        border: `1px solid ${colors.border}`,
      }}
    />
  );
}

function LoadingState({ colors, label, inline }: { colors: ThemeColors; label: string; inline?: boolean }) {
  const body = (
    <div style={{ fontSize: 12, color: colors.textDim }}>{label}</div>
  );
  if (inline) {
    return (
      <div style={{ border: `1px dashed ${colors.border}`, borderRadius: radius.lg, padding: 28, textAlign: 'center' }}>{body}</div>
    );
  }
  return (
    <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{body}</div>
  );
}

function ErrorState({ colors, message, onRetry, inline }: { colors: ThemeColors; message: string; onRetry: () => void; inline?: boolean }) {
  const body = (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: 13, color: colors.text, marginBottom: 4 }}>{message}</div>
      <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 12 }}>Something went wrong reaching the server.</div>
      <button
        onClick={onRetry}
        style={{
          fontSize: 12, fontFamily: font.body, color: colors.cyan, background: colors.cyanSoft,
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.md, padding: '6px 14px', cursor: 'pointer',
        }}
      >Retry</button>
    </div>
  );
  if (inline) {
    return (
      <div style={{ border: `1px solid ${colors.border}`, borderRadius: radius.lg, padding: 28 }}>{body}</div>
    );
  }
  return (
    <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{body}</div>
  );
}

// ── Analytics lens — growth funnel + metric tiles ────────────────────────────
//
// Shows REAL, derivable signal (content published, goals shipped) plus live
// visitors/pageviews once an analytics account is connected below. Ruled
// decision (2026-07-20): connect to an EXISTING analytics account via its
// stats API — Plausible (v1 Stats API, CE-compatible), Plausible Cloud (v2),
// or GoatCounter — read-only, provider-pluggable. This supersedes the earlier
// "self-hosted PostHog / native event bridge" plan. Metrics no provider
// exposes without goal config (signups, retention) keep their honest "no
// source" hints rather than faking a number.

/** Another action verified inside the same comparison window. */
interface Confounder {
  id: string;
  title: string;
}

/** One judged window. Mirrors `OutcomeView` in routes/growth_actions.rs:49. */
interface ActionOutcome {
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
interface ActionIdentity {
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
  outcomes: ActionOutcome[];
  /** The reading frozen at verification — what every window is compared
   *  against. Absent for an action that was never verified, and for one whose
   *  stored baseline no longer parses (the backend sends nothing rather than a
   *  zero, which would read as "there was no traffic before the change"). */
  baseline?: BaselineView | null;
}

/** One window of the frozen baseline. Mirrors `BaselineWindow` in
 *  routes/growth_actions.rs. */
interface BaselineWindow {
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
interface BaselineView {
  /** pageviews | sessions | aeo_visits | bounce_rate */
  metric: string;
  /** up | down */
  dir: string;
  /** First fully-post-change UTC day, `YYYY-MM-DD`. */
  pivot: string;
  takenAt: string;
  windows: BaselineWindow[];
}

interface GrowthVerifyResponse {
  verified: boolean;
  identity: ActionIdentity | null;
  /** Every strategy that was tried, passed or not, so a card can say why it
   *  could not confirm rather than reading as "not done". */
  checks: VerifyCheck[];
  reason: string | null;
}

/** One measured result from another project, named so the claim can be audited. */
interface TransferExample {
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
interface TransferNote {
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

interface GrowthAction {
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

interface GrowthActionsData {
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

/** The closed set an action may pre-register against (metrics.rs:41-47). Kept
 *  in the same order and spelling the backend parses, so a select can only ever
 *  produce a value `TargetMetric::parse` accepts. */
const TARGET_METRICS: { value: string; label: string }[] = [
  { value: 'pageviews', label: 'pageviews' },
  { value: 'sessions', label: 'sessions' },
  { value: 'aeo_visits', label: 'answer-engine visits' },
  { value: 'bounce_rate', label: 'bounce rate' },
];

/**
 * Verdict → label and colour.
 *
 * `inconclusive` is the one case that must NOT be tinted like a problem. The
 * proposal makes it the expected outcome at this traffic — "≲100 views/week —
 * per-project verdicts stay `inconclusive` essentially always" — and states the
 * rule directly: "It must be the visually neutral default, not a sad grey
 * state, or there is pressure to manufacture verdicts"
 * (docs/proposals/grow-action-outcome-loop.md:46-48, :173-175). So it borrows
 * the same `textMuted` the body copy uses rather than `danger` or a dimmer grey
 * than the settled `no_effect`.
 */
function verdictMeta(verdict: string, colors: ThemeColors): { label: string; color: string } {
  switch (verdict) {
    case 'helped': return { label: 'Helped', color: colors.success };
    case 'hindered': return { label: 'Hindered', color: colors.danger };
    case 'no_effect': return { label: 'No detectable change', color: colors.textDim };
    case 'confounded': return { label: 'Overlapped another change', color: colors.textDim };
    default: return { label: 'Not enough data to say', color: colors.textMuted };
  }
}

/**
 * How the change was confirmed, in words that say what was actually checked.
 *
 * The proposal's requirement, and the reason `verified_by` is a column at all:
 * "'Verified from a commit' and 'you told me so' are different claims and must
 * not look identical" (proposal:107-109). `checked` drives the styling apart as
 * well as the wording — self-attestation gets a dashed rule and the warning
 * tint, so the two are distinguishable at a glance and not only on a careful
 * read.
 */
function verifiedByMeta(how: string | null | undefined): { label: string; checked: boolean } {
  switch (how) {
    case 'git': return { label: 'Verified from a commit in this project’s repo', checked: true };
    case 'content': return { label: 'Verified on the live page', checked: true };
    case 'event': return { label: 'Verified from a traffic source that was not there before', checked: true };
    case 'self': return { label: 'You told me it landed — your word, not a check', checked: false };
    default: return { label: 'Not verified', checked: false };
  }
}

/**
 * When a window can first be judged.
 *
 * The pivot is the day AFTER verification, and the window completes once `days`
 * have fully elapsed from it (`pivot_date` at metrics.rs:156-157,
 * `window_is_complete` at :191-192). Rendering the date is what stops an empty
 * outcome list reading as "it found nothing" when the truth is "it is not due
 * yet".
 */
function windowDueAt(verifiedAt: string | null, days: number): Date | null {
  if (!verifiedAt) return null;
  const at = new Date(verifiedAt);
  if (Number.isNaN(at.getTime())) return null;
  const due = new Date(at);
  due.setUTCDate(due.getUTCDate() + 1 + days);
  return due;
}

/** The measurement windows, in the order and spelling the sweep uses
 *  (`metrics.rs` WINDOW_DAYS). The Tracking view walks all three; anything that
 *  shows only the first tells the user a 28-day verdict is not coming. */
const WINDOW_DAYS = [7, 14, 28];
/** The shortest window is 7 days (metrics.rs WINDOW_DAYS), the longest 28. */
const FIRST_WINDOW_DAYS = WINDOW_DAYS[0];
const FINAL_WINDOW_DAYS = WINDOW_DAYS[WINDOW_DAYS.length - 1];

/** Where one measurement window has got to.
 *
 *  `judged` — the sweep has written an outcome for it.
 *  `due`    — the window has fully elapsed but no outcome exists yet (the sweep
 *             runs nightly, so this is a real and honest state, not an error).
 *  `open`   — still accumulating; `dueAt` says when it closes.
 */
type WindowState = 'judged' | 'due' | 'open';

interface WindowProgress {
  days: number;
  state: WindowState;
  dueAt: Date | null;
  outcome: ActionOutcome | null;
}

/**
 * How far through the 7/14/28-day windows an action is.
 *
 * Derived here rather than sent by the server because every input is already on
 * the wire — `verifiedAt` and the outcomes — and a second source for the same
 * fact is a second thing that can disagree with the sweep. The boundary is the
 * same one `metrics::window_is_complete` uses: the pivot is the day AFTER
 * verification, and the window closes once `days` have fully elapsed from it.
 */
function windowProgress(
  identity: ActionIdentity,
  now: Date = new Date(),
): WindowProgress[] {
  return WINDOW_DAYS.map((days) => {
    const outcome = identity.outcomes.find((o) => o.windowDays === days) ?? null;
    const dueAt = windowDueAt(identity.verifiedAt, days);
    const state: WindowState = outcome
      ? 'judged'
      : dueAt && dueAt.getTime() <= now.getTime()
        ? 'due'
        : 'open';
    return { days, state, dueAt, outcome };
  });
}

/** A metric value in the units the metric is actually in. A bounce rate is a
 *  proportion in [0,1] and rendering it as "0.99" beside a pageview count reads
 *  as a broken number rather than as 99%. */
function metricValue(metric: string, value: number): string {
  if (metric === 'bounce_rate') return `${(value * 100).toFixed(0)}%`;
  return value.toLocaleString(undefined, { maximumFractionDigits: 1 });
}

/**
 * "Verify change" plus everything the verdict has to say — the honest half of
 * the card.
 *
 * Its own component, with its own state, for the reason
 * `analyticsPanelScope.test.ts` exists: a verify result that outlives the thing
 * it was about is the most damaging thing this surface can show. Keyed on the
 * action id by the caller, it is remounted whenever the action it describes
 * changes, so no stale verdict can be inherited.
 */
function ActionVerify({
  projectId, action, colors, readOnly = false,
}: {
  projectId: string;
  action: GrowthAction;
  colors: ThemeColors;
  /**
   * The archived shelf. Every CONTROL disappears — a filed action is a record,
   * not a thing still asking to be done — but the verdict does not. Suppressing
   * the whole component instead would hide the measured outcome of exactly the
   * actions the archive exists to keep as data points, which is the opposite of
   * what filing one away is supposed to mean.
   */
  readOnly?: boolean;
}) {
  const [result, setResult] = useState<GrowthVerifyResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [metric, setMetric] = useState('');
  const [dir, setDir] = useState('');

  const identity = result?.identity ?? action.identity ?? null;

  const verify = useCallback((body: Record<string, unknown>) => {
    if (!identity) return;
    setBusy(true);
    apiFetch<GrowthVerifyResponse>(
      `/api/projects/${encodeURIComponent(projectId)}/growth-actions/`
      + `${encodeURIComponent(identity.id)}/verify`,
      { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) },
    )
      .then(setResult)
      // A thrown fetch becomes a rendered, honest result rather than a dead
      // button — the rule the first-party install check already follows
      // (runVerify's catch, this file). A verify control that silently does
      // nothing is worse than one that says it failed.
      .catch((e) => setResult({
        verified: false,
        identity: null,
        checks: [],
        reason: `Could not run the check: ${e instanceof Error ? e.message : String(e)}`,
      }))
      .finally(() => setBusy(false));
  }, [projectId, identity]);

  const rule: CSSProperties = {
    marginTop: 10, paddingTop: 8, borderTop: `1px solid ${colors.border}`,
    display: 'flex', flexDirection: 'column', gap: 8,
  };
  const label: CSSProperties = {
    fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
    textTransform: 'uppercase', color: colors.textDim,
  };
  const button: CSSProperties = {
    background: colors.surface, border: `1px solid ${colors.border}`,
    borderRadius: radius.sm, padding: '3px 10px', cursor: busy ? 'default' : 'pointer',
    color: colors.text, fontFamily: font.body, fontSize: 11, opacity: busy ? 0.6 : 1,
  };
  const select: CSSProperties = {
    background: colors.bgDeeper, border: `1px solid ${colors.border}`,
    borderRadius: radius.sm, padding: '3px 6px', color: colors.text,
    fontFamily: font.body, fontSize: 11,
  };

  // On the shelf there is nothing to say unless a check actually confirmed
  // something: the controls are gone, so "this can't be verified" would be a
  // prompt to do something the card no longer offers.
  if (readOnly && (!identity || !identity.verifiedBy)) return null;

  // No row, no identity, nothing to attach a verdict to. Said out loud rather
  // than rendered as a missing button, which is indistinguishable from a
  // feature that was never built.
  if (!identity) {
    return (
      <div style={rule}>
        <span style={{ fontSize: 11, color: colors.textDim }}>
          This action has no saved record yet, so it can’t be verified. Run “Review again” to
          save it.
        </span>
      </div>
    );
  }

  const provenance = verifiedByMeta(identity.verifiedBy);
  const target = identity.targetMetric
    ? TARGET_METRICS.find((m) => m.value === identity.targetMetric)?.label ?? identity.targetMetric
    : null;

  // The agent predicted this only when BOTH halves are present. A metric with
  // no direction is not a prediction — "bounce rate moves" is true either way —
  // so a half-filled pair falls back to asking rather than guessing "up".
  const predicted = !!identity.targetMetric && !!identity.targetDir;
  const predictedLabel = target ?? identity.targetMetric;

  // The claim every verify call must carry. The row's own pre-registration wins
  // whenever it exists; the selects below only ever fill in for a row that has
  // none. Without this, the self-attest and re-check buttons — which now render
  // for a predicted action too — would post `targetMetric: ''` and take a 400
  // from `parse_target`, which reads on screen as "the check is broken".
  const targetBody = (): Record<string, unknown> => {
    if (identity.targetMetric && identity.targetDir) {
      return { targetMetric: identity.targetMetric, targetDir: identity.targetDir };
    }
    return metric && dir ? { targetMetric: metric, targetDir: dir } : {};
  };

  return (
    <div style={rule}>
      {identity.verifiedBy ? (
        <>
          {/* HOW it was checked, never just THAT it was. A commit and a
              self-report are different claims, so they get different colour,
              different border and different words. */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 6, alignSelf: 'flex-start',
            border: `1px ${provenance.checked ? 'solid' : 'dashed'} ${provenance.checked ? colors.success : colors.warning}`,
            borderRadius: radius.sm, padding: '2px 8px',
            color: provenance.checked ? colors.success : colors.warning,
            ...label,
          }}>
            <span>{provenance.checked ? '✓' : '✎'}</span>
            <span>{provenance.label}</span>
          </div>
          {target && identity.targetDir && (
            <span style={{ fontSize: 11, color: colors.textDim }}>
              Pre-registered before the baseline was frozen: {target} should go{' '}
              {identity.targetDir}
              {identity.verifiedAt && ` · verified ${new Date(identity.verifiedAt).toLocaleDateString()}`}
            </span>
          )}

          {identity.outcomes.map((o) => {
            const meta = verdictMeta(o.verdict, colors);
            // A percentage next to "not enough data to say" is the exact
            // failure the proposal names — "'this helped, +12%' off 40
            // pageviews is not measuring; it is pattern-matching noise and
            // presenting it as evidence" (proposal:35-39). So the number only
            // appears where a verdict actually rests on it.
            const showsDelta = (o.verdict === 'helped' || o.verdict === 'hindered')
              && o.deltaPct !== null;
            return (
              <div key={o.windowDays} style={{
                background: colors.bgDeeper, border: `1px solid ${colors.border}`,
                borderRadius: radius.md, padding: 10,
                display: 'flex', flexDirection: 'column', gap: 4,
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  <span style={{
                    ...label, color: meta.color, border: `1px solid ${meta.color}`,
                    borderRadius: 999, padding: '1px 7px',
                  }}>{meta.label}</span>
                  <span style={{ ...label }}>{o.windowDays}-day window</span>
                  {o.windowDays < FINAL_WINDOW_DAYS && (
                    // Proposal open decision 2: early windows are read, but
                    // labelled provisional rather than presented as settled.
                    <span style={{ ...label, color: colors.textDim }}>provisional</span>
                  )}
                  {showsDelta && (
                    <span style={{ fontFamily: font.mono, fontSize: 11, color: meta.color }}>
                      {o.deltaPct! > 0 ? '+' : ''}{(o.deltaPct! * 100).toFixed(0)}%
                    </span>
                  )}
                </div>
                {/* The rationale is body text, always. It carries the numbers
                    the verdict rests on, and a verdict whose reasoning is
                    hidden in a tooltip cannot be argued with. */}
                <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>
                  {o.rationale}
                </div>
                {o.confounders.length > 0 && (
                  <div style={{ fontSize: 11, color: colors.textDim }}>
                    Overlapping changes: {o.confounders.map((c) => c.title).join(', ')}
                  </div>
                )}
              </div>
            );
          })}

          {identity.outcomes.length === 0 && (() => {
            // Empty here means "not due yet", not "nothing found". Saying which
            // is the difference between a feature that is working and one that
            // looks broken.
            const due = windowDueAt(identity.verifiedAt, FIRST_WINDOW_DAYS);
            return (
              <span style={{ fontSize: 11, color: colors.textDim }}>
                Measuring. The first {FIRST_WINDOW_DAYS}-day reading is due
                {due ? ` ${due.toLocaleDateString()}` : ''}, then {14} and {FINAL_WINDOW_DAYS} days.
              </span>
            );
          })()}

          {/* The evidence a check found is NOT persisted — there is no
              `verified_detail` column — so a reload leaves the badge above with
              nothing behind it. This recovers it by re-running the checks.

              It is safe because `verify_mode` (growth_actions.rs) returns
              `Recheck` once `verified_at` is set and the handler then skips BOTH
              writes, returning the stored identity and the frozen baseline. It
              is NOT safe "by construction": only `baseline_json` coalesces.
              `record_verification` also writes `status = 'verified'` and
              `verified_at = now`, and `verified_at` is the pivot
              `metrics::pivot_date` measures every comparison window from — so
              without that guard this button would slide the after-windows
              forward against a baseline frozen days earlier and drag a judged
              action back into measurement. Do not delete one half without the
              other. */}
          {!readOnly && (
            <button
              onClick={() => verify(targetBody())}
              disabled={busy}
              style={{ ...button, alignSelf: 'flex-start' }}
            >{busy ? 'Re-checking…' : 'Re-check'}</button>
          )}
        </>
      ) : (
        <>
          {/* Pre-registration is a gate, not a form field: the backend refuses a
              verify without a target (growth_actions.rs) so a metric cannot be
              chosen once the result is visible.

              WHO fills it in is the point. The agent recommended this action, so
              the agent states what it expects to move — that claim is what the
              7/14/28-day sweep grades it against. Asking the user to supply it
              inverted the loop: they would be answering the question they came
              here to be advised on, and there would be no prediction of the
              agent's left to be right or wrong. The selects below are now the
              FALLBACK, for an action whose agent declined to predict (or one
              suggested before predictions existed). */}
          {predicted ? (
            <>
              <span style={{ fontSize: 11, color: colors.textDim }}>
                I expect this to move{' '}
                <strong style={{ color: colors.text }}>{predictedLabel}</strong>{' '}
                <strong style={{ color: colors.text }}>
                  {identity.targetDir === 'down' ? 'down' : 'up'}
                </strong>. I’ll check at 7, 14 and 28 days and record whether I was right.
              </span>
              {/* The one control here on purpose. There used to be a
                  "Measure something else" button beside it that revealed the
                  selects below and let the user replace the agent's target.
                  It is gone: the target is the AGENT's prediction and this loop
                  exists to grade the agent, so measuring a claim the agent
                  never made produces a verdict about nobody — the exact
                  unfalsifiability the pre-registration gate was built to stop.
                  The selects survive only for a row that genuinely carries no
                  prediction. */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                <button
                  onClick={() => verify(targetBody())}
                  disabled={busy}
                  style={{ ...button, opacity: busy ? 0.5 : 1 }}
                >{busy ? 'Checking…' : 'I did this — start measuring'}</button>
              </div>
            </>
          ) : (
            <>
              <span style={{ fontSize: 11, color: colors.textDim }}>
                {identity.targetMetric || identity.targetDir
                  ? 'I couldn’t say what this should move, so pick the metric before checking it.'
                  : 'Say what this should move before checking it — a metric picked after the result is known can’t be wrong.'}
              </span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                <select
                  aria-label="Target metric"
                  value={metric}
                  onChange={(e) => setMetric(e.target.value)}
                  style={select}
                >
                  <option value="">what should move…</option>
                  {TARGET_METRICS.map((m) => (
                    <option key={m.value} value={m.value}>{m.label}</option>
                  ))}
                </select>
                <select
                  aria-label="Target direction"
                  value={dir}
                  onChange={(e) => setDir(e.target.value)}
                  style={select}
                >
                  <option value="">which way…</option>
                  <option value="up">should go up</option>
                  <option value="down">should go down</option>
                </select>
                <button
                  onClick={() => verify({ targetMetric: metric, targetDir: dir })}
                  disabled={busy || !metric || !dir}
                  style={{ ...button, opacity: busy || !metric || !dir ? 0.5 : 1 }}
                >{busy ? 'Checking…' : 'Verify change'}</button>
              </div>
            </>
          )}
        </>
      )}

      {/* OUTSIDE the verified/not-verified ternary on purpose. This used to
          render only under `result && !result.verified`, so a PASS showed a
          bare badge and threw away the one thing the user could audit — which
          commit, which path, which string on which page. A pass that also says
          "live page does not contain it" teaches something the badge hides. */}
      {result && (
        <div style={{
          background: colors.bgDeeper, border: `1px solid ${colors.border}`,
          borderRadius: radius.md, padding: 10,
          display: 'flex', flexDirection: 'column', gap: 6,
        }}>
          {/* "Could not confirm" is not "not done", and the checks say which
              one it was (growth_verify.rs:9-11). */}
          {/* "What confirmed it" only when something below actually did. A
              re-check of a self-attested action re-runs the real strategies and
              they can all come back empty — the action IS verified, on the
              user's word, and heading that list with "what confirmed it" would
              dress four failed checks up as corroboration. */}
          <span style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>
            {result.verified
              ? ((result.checks ?? []).some((c) => c.passed)
                ? 'What confirmed it'
                : 'What the checks found')
              : result.reason ?? 'Nothing could confirm the change landed.'}
          </span>
          {/* `?? []` because this block now renders on a PASS too, and a
              payload from an older daemon — or a truncated one — carries no
              `checks`. A missing list must cost the evidence line, not take the
              whole Grow tab down with a TypeError. */}
          {(result.checks ?? []).map((c) => (
            <div key={c.id} style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
              <span style={{ color: c.passed ? colors.success : colors.textDim }}>
                {c.passed ? '✓' : '·'}
              </span>{' '}
              {c.label} — {c.detail}
            </div>
          ))}
          {!result.verified && (
            <button
              onClick={() => verify({ ...targetBody(), selfAttested: true })}
              disabled={busy}
              style={{ ...button, alignSelf: 'flex-start' }}
            >It did land — record my word</button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Category → tint, at module scope so the archived shelf and the active board
 * colour the same category identically. Two copies drifted apart is the kind of
 * difference a reader would read as meaning.
 */
function categoryColor(category: string, colors: ThemeColors): string {
  const map: Record<string, string> = {
    conversion: colors.cyan,
    retention: colors.success,
    churn: colors.danger,
    ux: colors.purple,
    acquisition: colors.warning ?? colors.cyan,
    measurement: colors.textMuted,
    content: colors.cyan,
    seo: colors.success,
    aeo: colors.purple,
  };
  return map[category] ?? colors.textDim;
}

/**
 * The states an action may be filed away from.
 *
 * `suggested` is absent deliberately, and `reject_pointless_archive`
 * (growth_actions.rs) refuses it on the server for the same reason: archiving
 * is what releases an action's text for re-proposal, so filing away something
 * that was never acted on would hand the identical advice straight back on the
 * next review. Dismissal is the control for advice the user does not want, and
 * it keeps the text off the board.
 */
const ARCHIVABLE = ['done', 'verified', 'measuring', 'judged', 'dismissed'];

/** Which list a card is in. See `ActionCard`'s `lane` prop. */
type ActionLane = 'actions' | 'tracking' | 'shelf';

/**
 * The measurement rail on a Tracking card: the frozen baseline, and how far
 * through the 7/14/28-day windows this action is.
 *
 * `ActionVerify` already renders the verdicts themselves, so this deliberately
 * does not repeat them — it renders what the card was missing, which is the
 * "before" every verdict is computed against and the windows that have not
 * reported yet. An empty outcome list with no rail reads as "the measurement
 * found nothing"; the truth is almost always "it is not due until the 26th".
 */
function TrackingRail({ identity, colors }: { identity: ActionIdentity; colors: ThemeColors }) {
  const metric = identity.targetMetric ?? identity.baseline?.metric ?? null;
  const progress = windowProgress(identity);
  const baselineByWindow = new Map(
    (identity.baseline?.windows ?? []).map((w) => [w.windowDays, w]),
  );

  const label: CSSProperties = {
    fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
    textTransform: 'uppercase', color: colors.textDim,
  };

  return (
    <div style={{
      marginTop: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      borderRadius: radius.md, padding: 10,
      display: 'flex', flexDirection: 'column', gap: 8,
    }}>
      <span style={label}>Measuring against</span>
      {identity.baseline ? (
        <span style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
          Baseline frozen {new Date(identity.baseline.takenAt).toLocaleDateString()}. Windows
          start {identity.baseline.pivot} — the change day itself is in neither half.
        </span>
      ) : (
        // Never a zero. A baseline of nought would render as "there was no
        // traffic before the change", which is a claim nothing here can make.
        <span style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
          No baseline was frozen for this action, so its windows cannot be compared.
        </span>
      )}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {progress.map((w) => {
          const before = baselineByWindow.get(w.days) ?? null;
          return (
            <div key={w.days} style={{
              display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap',
              fontSize: 11, color: colors.textDim,
            }}>
              <span style={{ ...label, minWidth: 52 }}>{w.days}-day</span>
              <span style={{
                color: w.state === 'judged' ? colors.text : colors.textDim,
              }}>
                {w.state === 'judged'
                  ? `read ${new Date(w.outcome!.judgedAt).toLocaleDateString()} — ${verdictMeta(w.outcome!.verdict, colors).label.toLowerCase()}`
                  : w.state === 'due'
                    // The sweep is nightly, so "due" is a real state and saying
                    // so is honest. Silence here reads as a stuck experiment.
                    ? 'window closed — the next nightly sweep will read it'
                    : `closes ${w.dueAt ? w.dueAt.toLocaleDateString() : 'once verified'}`}
              </span>
              {before && metric && (
                <span style={{ fontFamily: font.mono, color: colors.textDim }}>
                  before: {metricValue(metric, before.value)}
                  {before.denominator > 0 && metric === 'bounce_rate'
                    ? ` of ${before.denominator.toLocaleString()} sessions`
                    : ''}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/**
 * One action, on the board or on the archived shelf.
 *
 * Its own top-level component rather than a block inside `GrowActions`' map
 * because the archived list renders the same card read-only, and because the
 * copy-confirmation flag belongs to a card rather than to an index into a list
 * that reorders on every review.
 */
function ActionCard({
  project, action, colors, lane, onChanged, showCategory = true,
}: {
  project: Project;
  action: GrowthAction;
  colors: ThemeColors;
  /** Which list this card is in — it decides which exits the card offers.
   *
   *  `actions`  work still asking for a decision. Dismiss is offered whatever
   *             the status, because this is the list the user is trying to
   *             shorten and a row here with no control is the defect.
   *  `tracking` work being measured. Archive is the exit: it files the card
   *             away and KEEPS measuring it, which is what filing away
   *             in-flight work has to mean. Dismiss is not offered — it would
   *             drop a live experiment into the refused pile.
   *  `shelf`    archived or dismissed. A record: no controls at all. */
  lane: ActionLane;
  /** Refetch the board. Archiving moves a card between two lists, so the
   *  parent has to re-read rather than this card patching itself. */
  onChanged: () => void;
  /** False inside a category tab — the tab already names the category, and a
   *  chip that repeats it is how the old long tagged list read. */
  showCategory?: boolean;
}) {
  const readOnly = lane === 'shelf';
  const [copied, setCopied] = useState(false);
  const [moving, setMoving] = useState<string | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);
  const [agentId, setAgentId] = useState<string>(CODING_AGENTS[0].id);
  const [sending, setSending] = useState(false);
  const setPendingTerminalLaunch = useCommandCenter((s) => s.setPendingTerminalLaunch);

  const directive = codingAgentDirective({
    projectName: project.name,
    projectRoot: project.rootPath,
    action,
  });

  const copyArtifact = useCallback((text: string) => {
    navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    });
  }, []);

  const identity = action.identity ?? null;
  const actionId = identity?.id ?? null;

  /** One lifecycle route, two exits, and they are not interchangeable.
   *
   *  Archiving RELEASES an action's text for re-proposal (`board` excludes
   *  archived rows and `restates` is checked against `board`), so it is the
   *  wrong exit for advice the user is done with — and the server refuses it
   *  outright on a `suggested` row for that reason. Dismissal keeps the text on
   *  the generator's board where it can never be proposed again, which is why
   *  it is the exit offered on every card in the Actions list whatever its
   *  status. Without it nothing the user could press ever shortened the panel,
   *  so it could only grow. */
  const move = useCallback((status: string) => {
    if (!actionId) return;
    setMoving(status);
    setMoveError(null);
    apiFetch(
      `/api/projects/${encodeURIComponent(project.id)}/growth-actions/`
      + `${encodeURIComponent(actionId)}/status`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ status }),
      },
    )
      .then(() => onChanged())
      // A refused move says why. The server's refusals are written to be read
      // ("Nothing has happened to this action yet…"), and swallowing one would
      // look like a dead button.
      .catch((e) => setMoveError(e instanceof Error ? e.message : String(e)))
      .finally(() => setMoving(null));
  }, [project.id, actionId, onChanged]);

  const sendToAgent = useCallback(() => {
    const agent = codingAgentById(agentId);
    if (!agent || !project.rootPath) return;
    setSending(true);
    const display = agent.command.split(' ')[0] || agent.label;
    // Queue the launch before switching workspaces: if Build mounts in the
    // same tick, it must already see the pending payload.
    setPendingTerminalLaunch({
      rootPath: project.rootPath,
      label: `${project.slug} · ${display} · grow`,
      command: agent.command,
      followUpInput: directive,
      growthAction: actionId ? { projectId: project.id, actionId } : undefined,
    });
    const opened = navigateToTool('build');
    if (!opened) {
      setMoveError('Open the Build workspace to send this to a coding agent.');
    }
    setSending(false);
  }, [agentId, project.rootPath, project.slug, project.id, directive, actionId, setPendingTerminalLaunch]);

  const tint = categoryColor(action.category, colors);
  const transfer = action.transfer ?? null;
  const canArchive = !readOnly && !!identity && ARCHIVABLE.includes(identity.status);
  /** Keyed on the DURABLE ROW, not on a status allowlist and not on the prose
   *  cache.
   *
   *  This was `identity.status === 'suggested'`, which is a claim about
   *  lifecycle where the user's need is about the list: they can see the row,
   *  they have already done it (or never will), and they want it gone. On this
   *  project four actions have been on the board since 2026-08-14 with no entry
   *  left in the prose cache; every control the panel offers hangs off the
   *  identity, so the rule now is simply "the board can reach this row" — which
   *  is true for every card the board renders, because `render_board` builds
   *  the list FROM the rows. `done` in particular had no dismissal at all, and
   *  its only other exit — Archive — releases the text for re-proposal, so
   *  filing away stale advice handed the identical advice back on the next
   *  review. That is exactly what happened here: the 2026-08-19 review restated
   *  the 2026-08-14 funnel action. */
  const canDismiss = lane === 'actions' && !!actionId;

  const smallButton: CSSProperties = {
    background: colors.surface, border: `1px solid ${colors.border}`,
    borderRadius: radius.sm, padding: '3px 10px', cursor: 'pointer',
    color: colors.text, fontFamily: font.body, fontSize: 11,
  };

  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: 14,
      // Filed work reads as a record, not as something still asking to be done.
      opacity: readOnly ? 0.75 : 1,
    }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 6 }}>
        {showCategory && (
          <span style={{
            fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
            textTransform: 'uppercase', color: tint,
            border: `1px solid ${tint}`,
            borderRadius: 999, padding: '1px 7px', flexShrink: 0,
          }}>{action.category}</span>
        )}
        <span style={{ fontFamily: font.display, fontSize: 14, fontWeight: 600, color: colors.text }}>
          {action.title}
        </span>
        <div style={{ flex: 1 }} />
        {/* Only when the prose cache still holds both. "medium impact · medium
            confidence" invented for a card whose cache entry was pruned is the
            same fabrication the backend refuses when it declines to default a
            target. */}
        {action.impact && action.confidence && (
          <span style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, flexShrink: 0 }}>
            {action.impact} impact · {action.confidence} confidence
          </span>
        )}
      </div>

      {/* What this CATEGORY has measurably done elsewhere — derived from
          `growth_action_outcomes` on the user's other active projects, never
          asserted by the model. The provenance disclosure is mandatory: a card
          that appears because something worked elsewhere and will not say where
          is not auditable, and is indistinguishable from the model flattering
          its own suggestion. */}
      {transfer && (
        <div style={{ marginBottom: 6, display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span style={{ fontSize: 11, color: colors.textDim }}>
            {transfer.helped > 0
              ? `Worked on ${transfer.helped} of ${transfer.projects} other project(s)`
                + ` — on projects like this one, ${transfer.segmentHelped} of`
                + ` ${transfer.segmentProjects} (${transfer.segmentLabel})`
              : transfer.hindered > 0
                ? `Hindered on ${transfer.hindered} of ${transfer.projects} other project(s)`
                : `Tried on ${transfer.projects} other project(s), with no detectable change`}
          </span>
          {transfer.examples.length > 0 && (
            <details>
              <summary style={{
                fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                textTransform: 'uppercase', color: colors.textDim, cursor: 'pointer',
              }}>Where that comes from</summary>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 4 }}>
                {transfer.examples.map((ex, xi) => (
                  <div key={`${ex.projectName}-${xi}`} style={{ fontSize: 11, color: colors.textDim }}>
                    &ldquo;{ex.title}&rdquo; on {ex.projectName} —{' '}
                    {verdictMeta(ex.verdict, colors).label}
                    {ex.deltaPct !== null && (
                      `, ${ex.deltaPct > 0 ? '+' : ''}${(ex.deltaPct * 100).toFixed(0)}%`
                    )}
                  </div>
                ))}
              </div>
            </details>
          )}
        </div>
      )}

      {/* Evidence first: the number is what makes this checkable. Rendered only
          when there is one — an empty rail beside a card whose prose the cache
          no longer holds reads as a missing figure rather than as no figure. */}
      {action.evidence && (
        <div style={{
          fontSize: 11, color: colors.textDim, fontFamily: font.mono,
          borderLeft: `2px solid ${colors.border}`, paddingLeft: 8, marginBottom: 6,
        }}>{action.evidence}</div>
      )}
      <div style={{ fontSize: 13, color: colors.textMuted, lineHeight: 1.5 }}>
        {action.recommendation}
      </div>

      {/* Ordered steps: an action nobody knows how to start is an observation
          wearing an action's clothes. */}
      {action.steps?.length > 0 && (
        <ol style={{ margin: '8px 0 0', paddingLeft: 18, fontSize: 12, color: colors.textMuted, lineHeight: 1.6 }}>
          {action.steps.map((step, si) => <li key={si}>{step}</li>)}
        </ol>
      )}

      {/* Always a coding-agent prompt, even when the generator stored a bare
          post. Copying the raw artifact is how SEO work landed in chat as a
          blog post with no path and no instruction. */}
      {lane === 'actions' && (
        <div style={{ marginTop: 10 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4, flexWrap: 'wrap' }}>
            <span style={{ fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textDim }}>
              Prompt for your coding agent
            </span>
            <div style={{ flex: 1 }} />
            <button
              onClick={() => copyArtifact(directive)}
              style={smallButton}
            >{copied ? 'Copied ✓' : 'Copy'}</button>
            <select
              aria-label="Coding agent"
              value={agentId}
              onChange={(e) => setAgentId(e.target.value)}
              style={{
                ...smallButton, cursor: 'pointer',
                opacity: project.rootPath ? 1 : 0.5,
              }}
            >
              {CODING_AGENTS.map((a) => (
                <option key={a.id} value={a.id}>{a.label}</option>
              ))}
            </select>
            <button
              onClick={sendToAgent}
              disabled={sending || !project.rootPath}
              title={!project.rootPath
                ? 'Add a root path to this project to launch a coding agent here.'
                : `Open ${codingAgentById(agentId)?.label ?? 'the agent'} in Build with this prompt`}
              style={{ ...smallButton, opacity: (sending || !project.rootPath) ? 0.5 : 1 }}
            >{sending ? 'Sending…' : 'Send'}</button>
          </div>
          <pre style={{
            margin: 0, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: 10, fontSize: 11, fontFamily: font.mono,
            color: colors.textMuted, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            maxHeight: 200, overflowY: 'auto',
          }}>{directive}</pre>
        </div>
      )}

      {/* OUTSIDE the artifact block on purpose. Verification applies to every
          action including artifactKind "none" — that is the `self` fallback row
          of the proposal's table (proposal:105) — and putting this beside Copy
          would hide it for exactly the actions with no deliverable to copy. */}
      <ActionVerify
        key={actionId ?? action.title}
        projectId={project.id}
        action={action}
        colors={colors}
        readOnly={readOnly}
      />

      {/* Only on the Tracking lane. On the Actions lane there is nothing to
          measure yet, and on the shelf the card is a record. */}
      {lane === 'tracking' && identity && (
        <TrackingRail identity={identity} colors={colors} />
      )}

      {canArchive && (
        <div style={{ marginTop: 8, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          {/* Not deletion, and the wording must not promise permanence: an
              archived action keeps being measured while it still owes a window,
              keeps feeding the agent's learning, and releases its text back for
              re-proposal. */}
          <button
            onClick={() => move('archived')}
            disabled={!!moving}
            style={{ ...smallButton, opacity: moving ? 0.5 : 1 }}
          >{moving === 'archived' ? 'Filing…' : 'Archive'}</button>
          <span style={{ fontSize: 10, color: colors.textDim }}>
            Files it away. It keeps being measured and keeps teaching the agent.
          </span>
        </div>
      )}
      {canDismiss && (
        <div style={{ marginTop: 8, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <button
            onClick={() => move('dismissed')}
            disabled={!!moving}
            style={{ ...smallButton, opacity: moving ? 0.5 : 1 }}
          >{moving === 'dismissed' ? 'Dismissing…' : 'Not interested'}</button>
          {/* The distinction matters and is the whole reason dismissal is not
              archiving: a dismissed action stays ON the board, so the generator
              still sees it and cannot propose it again. Archiving releases the
              text. */}
          <span style={{ fontSize: 10, color: colors.textDim }}>
            Takes it off the list. The agent keeps it in view, so it will not suggest it again.
          </span>
        </div>
      )}
      {moveError && (
        <div style={{ marginTop: 6, fontSize: 11, color: colors.danger }}>{moveError}</div>
      )}
    </div>
  );
}

/**
 * Actions — what to DO about the data. The Analytics lens answers "what
 * happened"; this answers "so what".
 *
 * Two sources, deliberately distinguished so the user can weigh them
 * differently: the deterministic growth inbox (computed server-side from real
 * signals, NO model involved) and the agent's read of the analytics. The
 * second is labelled as a model's reading and every item carries the figure it
 * was drawn from, because an ungrounded suggestion that looks like analysis is
 * worse than no suggestion.
 */
export function GrowActions({ project, colors }: { project: Project; colors: ThemeColors }) {
  const [inbox, setInbox] = useState<GrowthInboxData | null>(null);
  const [inboxState, setInboxState] = useState<LoadState>('loading');
  const inboxGen = useRef(0);

  const loadInbox = useCallback((id: string) => {
    const generation = ++inboxGen.current;
    setInboxState('loading');
    apiFetch<GrowthInboxData>(`/api/projects/${encodeURIComponent(id)}/growth-inbox`)
      .then((d) => {
        if (generation !== inboxGen.current) return;
        setInbox(d);
        setInboxState('ready');
      })
      .catch(() => {
        if (generation !== inboxGen.current) return;
        setInbox(null);
        setInboxState('error');
      });
  }, []);

  const [actions, setActions] = useState<GrowthActionsData | null>(null);
  const [actionsState, setActionsState] = useState<LoadState>('loading');
  const actionsGen = useRef(0);

  /**
   * The review is running ON THE SERVER.
   *
   * This replaced a `useState(false)` set by the click handler. That flag was
   * component-local, and this component unmounts when the user leaves the tab
   * (`lens === 'actions' && <GrowActions …>`) or switches project — so the flag
   * was destroyed while the review carried on, and coming back showed an idle
   * button over a review that was still running. The result then landed in the
   * database with nothing on screen to say it had.
   *
   * The truth now lives where the work does. Every GET reports it, so a remount
   * reconciles instead of guessing.
   */
  const serverGenerating = actions?.generating ?? false;
  /**
   * The click, before the server has answered.
   *
   * Only bridges the round trip between pressing the button and the POST's
   * reply — the spinner must be on screen from the moment the click is
   * registered, and without this it would appear a request later. It is NOT
   * the source of truth and never outlives the request: if this component
   * unmounts mid-flight, the server's flag is what the next mount reads.
   */
  const [pending, setPending] = useState(false);
  const busyGenerating = pending || serverGenerating;

  /** `silent` keeps the current cards on screen while re-reading. The poll
   *  below runs every few seconds during a review; dropping the board to
   *  skeletons each time would make the panel flash for as long as the review
   *  takes. */
  const loadActions = useCallback((id: string, opts?: { silent?: boolean }) => {
    const generation = ++actionsGen.current;
    if (!opts?.silent) setActionsState('loading');
    apiFetch<GrowthActionsData>(`/api/projects/${encodeURIComponent(id)}/growth-actions`)
      .then((d) => {
        if (generation !== actionsGen.current) return;
        setActions(d);
        setActionsState('ready');
      })
      .catch(() => {
        if (generation !== actionsGen.current) return;
        if (!opts?.silent) setActionsState('error');
      });
  }, []);

  // Regeneration is explicit. It spends a model call, and actions that
  // reshuffle on every render cannot be acted on.
  //
  // The POST now returns as soon as the review has been STARTED — the work runs
  // in a task on the daemon that no longer belongs to this request — so the
  // reply carries the board as it stands with `generating: true`. `pending` is
  // set first and synchronously so the spinner is on screen from the moment the
  // click is registered rather than one round trip later; the server's flag
  // takes over from it as soon as the reply lands.
  const generate = useCallback((id: string) => {
    if (busyGenerating) return;
    setPending(true);
    apiFetch<GrowthActionsData>(
      `/api/projects/${encodeURIComponent(id)}/growth-actions/generate`,
      { method: 'POST' },
    )
      .then((d) => { setActions(d); setActionsState('ready'); })
      .catch(() => setActionsState('error'))
      .finally(() => setPending(false));
  }, [busyGenerating]);

  useEffect(() => {
    loadInbox(project.id);
    loadActions(project.id);
    return () => { ++inboxGen.current; ++actionsGen.current; };
  }, [project.id, loadInbox, loadActions]);

  // While a review is running, keep asking. This is what makes returning to the
  // tab mid-run show it still running and, when it lands, show the new actions
  // with nothing to press: the flag is on the server, so a remount reads it
  // from the GET above and this poll carries it to completion.
  //
  // It runs ONLY while `generating` is true — it is not a background poll of
  // the panel, and it stops the moment the review does.
  useEffect(() => {
    if (!serverGenerating) return;
    const t = setInterval(() => loadActions(project.id, { silent: true }), GENERATION_POLL_MS);
    return () => clearInterval(t);
  }, [serverGenerating, project.id, loadActions]);

  // Multi-client liveness (#629): the daemon emits `project_changed` when a
  // review finishes, which `livenessSync` turns into a `projectsRev` bump. This
  // is the fast path — the poll above is the belt that still works if the
  // socket is down. Skipped on the first render so it does not double the load
  // the mount effect already did.
  const projectsRev = useCommandCenter((st) => st.projectsRev);
  const seenRev = useRef(projectsRev);
  useEffect(() => {
    if (seenRev.current === projectsRev) return;
    seenRev.current = projectsRev;
    loadActions(project.id, { silent: true });
  }, [projectsRev, project.id, loadActions]);

  const hasActions = (actions?.actions?.length ?? 0) > 0;
  const tracking = actions?.tracking ?? [];
  const archived = actions?.archived ?? [];
  const dismissed = actions?.dismissed ?? [];
  const droppedRestated = actions?.droppedAsRestatement ?? 0;
  const droppedNoTarget = actions?.droppedForNoTarget ?? 0;
  const droppedPresent = actions?.droppedAsAlreadyPresent ?? 0;
  const onChanged = useCallback(() => loadActions(project.id), [loadActions, project.id]);

  const actionGroups = groupActionsByCategory(actions?.actions ?? []);
  const [categoryTab, setCategoryTab] = useState<string | null>(null);
  const [focusCategory, setFocusCategory] = useState<string | null>(null);
  const groupKeys = actionGroups.map((g) => g.key).join(',');
  useEffect(() => {
    if (!groupKeys) {
      setCategoryTab(null);
      return;
    }
    const keys = groupKeys.split(',');
    if (!categoryTab || !keys.includes(categoryTab)) {
      setCategoryTab(keys[0]);
    }
  }, [groupKeys, categoryTab]);
  const selectedGroup = actionGroups.find((g) => g.key === categoryTab) ?? actionGroups[0] ?? null;

  return (
    <>
      {/* Deterministic moves — no model in the loop. */}
      <GrowthInboxSection
        colors={colors}
        state={inboxState}
        inbox={inbox}
        onRetry={() => loadInbox(project.id)}
      />

      <section style={{ marginTop: 8 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 10 }}>
          <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>
            From your analytics
          </h3>
          <div style={{ flex: 1 }} />
          {actions?.generatedAt && (
            <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>
              {new Date(actions.generatedAt).toLocaleString()}
            </span>
          )}
          {/* Two clicks cannot start two reviews: the button is disabled for as
              long as either half of `busyGenerating` holds, and the daemon
              refuses a second review for a project that already has one running
              (`begin_review`). The disabled attribute is the courtesy; the
              server is the rule. */}
          <button
            onClick={() => generate(project.id)}
            disabled={busyGenerating}
            aria-busy={busyGenerating}
            style={{
              background: colors.surface, border: `1px solid ${colors.border}`,
              borderRadius: radius.md, padding: '5px 12px',
              cursor: busyGenerating ? 'default' : 'pointer',
              color: colors.text, fontFamily: font.body, fontSize: 12,
              opacity: busyGenerating ? 0.7 : 1,
              display: 'inline-flex', alignItems: 'center', gap: 6,
            }}
          >
            {/* A MOVING affordance, not a text swap. "Reviewing…" is
                indistinguishable from a stuck button, and this one can be on
                screen for the length of a model call. `pa-spin` is the app's
                spinner utility (index.css) and honours prefers-reduced-motion
                through the global block there. */}
            {busyGenerating && <FiLoader size={12} className="pa-spin" aria-hidden />}
            {busyGenerating
              ? 'Reviewing your analytics…'
              : hasActions ? 'Review again' : 'Review my analytics'}
          </button>
        </div>

        {/* Said out loud, because the one thing the user could not tell before
            was whether anything was still happening. The review runs on the
            daemon, so this is true whether or not this tab is open. */}
        {busyGenerating && (
          <div style={{
            fontSize: 11, color: colors.textDim, marginBottom: 10,
            display: 'flex', alignItems: 'center', gap: 6,
          }}>
            <FiLoader size={11} className="pa-spin" aria-hidden />
            <span>
              Your agent is reading the last {actions?.periodDays ?? 30} days. This keeps running
              if you leave the tab — come back and the new actions will be here.
            </span>
          </div>
        )}

        {actionsState === 'loading' && <SkeletonCards colors={colors} count={2} height={92} />}
        {actionsState === 'error' && (
          <div style={{ fontSize: 12, color: colors.danger }}>Couldn&rsquo;t load actions.</div>
        )}

        {/* An empty list ALWAYS explains itself — silence is indistinguishable
            from breakage, and this panel is allowed to have nothing to say. */}
        {actionsState === 'ready' && !hasActions && !busyGenerating && (
          <div style={{
            fontSize: 12, color: colors.textMuted, background: colors.bgDeeper,
            border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: 12,
          }}>
            {/* An empty Actions list with a full Tracking list is not "nothing
                to say" — it is "everything you were offered is now being
                measured", and saying the wrong one of those reads as data
                loss. */}
            {tracking.length > 0
              ? 'Nothing is waiting on you. Everything you took on is being measured below.'
              : actions?.reason ?? 'No review yet — run one to see what your data suggests.'}
          </div>
        )}

        {actionGroups.length > 0 && (
          <div
            role="tablist"
            aria-label="Action category"
            style={{
              display: 'flex', gap: 2, flexWrap: 'wrap',
              background: colors.bgDeeper, borderRadius: radius.md, padding: 2,
              marginBottom: 10,
            }}
          >
            {actionGroups.map((group) => {
              const selected = selectedGroup?.key === group.key;
              return (
                <button
                  key={group.key}
                  role="tab"
                  aria-selected={selected}
                  tabIndex={0}
                  onClick={() => setCategoryTab(group.key)}
                  onFocus={() => setFocusCategory(group.key)}
                  onBlur={() => setFocusCategory(null)}
                  style={{
                    fontSize: 12, fontFamily: font.body,
                    padding: '5px 12px', borderRadius: radius.sm, cursor: 'pointer', border: 'none',
                    background: selected ? colors.cyanSoft : 'transparent',
                    color: selected ? colors.cyan : colors.textMuted,
                    fontWeight: selected ? 600 : 500,
                    outline: 'none',
                    boxShadow: focusCategory === group.key ? `0 0 0 2px ${colors.borderHi}` : 'none',
                  }}
                >
                  {group.label} ({group.actions.length})
                </button>
              );
            })}
          </div>
        )}

        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {(selectedGroup?.actions ?? []).map((a, i) => (
            // Identity first: the durable id survives a regeneration that
            // rewords the title, so an in-flight verify stays attached to the
            // card it was started from rather than jumping to a neighbour.
            <ActionCard
              key={a.identity?.id ?? `${a.title}-${i}`}
              project={project}
              action={a}
              colors={colors}
              lane="actions"
              onChanged={onChanged}
              showCategory={false}
            />
          ))}
        </div>

        {/* Tracking — what we changed, and whether it worked.

            Not collapsed and not the archive: this is live work with verdicts
            still to come, and the user asked for it precisely so a verified
            action would leave the decision list without leaving their sight.
            #1053 deliberately kept these rows on the active board so nothing
            in flight could silently vanish; that guarantee is honoured by
            MOVING them here, where every one is still rendered with its
            evidence, its prediction, its baseline and its windows. */}
        {tracking.length > 0 && (
          <section style={{ marginTop: 18 }}>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, marginBottom: 10 }}>
              <h3 style={{
                fontFamily: font.mono, fontSize: 11, color: colors.textDim,
                textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0,
              }}>Tracking ({tracking.length})</h3>
              <span style={{ fontSize: 10, color: colors.textDim }}>
                Changes you made, measured at {WINDOW_DAYS.join(', ')} days against the
                traffic before them.
              </span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              {tracking.map((a, i) => (
                <ActionCard
                  key={a.identity?.id ?? `tracking-${a.title}-${i}`}
                  project={project}
                  action={a}
                  colors={colors}
                  lane="tracking"
                  onChanged={onChanged}
                />
              ))}
            </div>
          </section>
        )}

        {/* The shelf. Collapsed, because filed work is a record the user goes
            looking for rather than something competing with the board — but
            present, because an archive you cannot open is a delete. */}
        {archived.length > 0 && (
          <details style={{ marginTop: 12 }}>
            <summary style={{
              fontFamily: font.mono, fontSize: 10, letterSpacing: '0.08em',
              textTransform: 'uppercase', color: colors.textDim, cursor: 'pointer',
            }}>Archived ({archived.length})</summary>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginTop: 10 }}>
              {archived.map((a, i) => (
                <ActionCard
                  key={a.identity?.id ?? `archived-${a.title}-${i}`}
                  project={project}
                  action={a}
                  colors={colors}
                  lane="shelf"
                  onChanged={onChanged}
                />
              ))}
            </div>
          </details>
        )}

        {/* Advice the user turned down. Its own section rather than the archive
            because the two are opposites to the agent: dismissed text stays on
            the board and can never be proposed again, archived text is
            released. Collapsed for the same reason as the archive — a refusal
            is a record, not work. */}
        {dismissed.length > 0 && (
          <details style={{ marginTop: 12 }}>
            <summary style={{
              fontFamily: font.mono, fontSize: 10, letterSpacing: '0.08em',
              textTransform: 'uppercase', color: colors.textDim, cursor: 'pointer',
            }}>Dismissed ({dismissed.length})</summary>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginTop: 10 }}>
              {dismissed.map((a, i) => (
                <ActionCard
                  key={a.identity?.id ?? `dismissed-${a.title}-${i}`}
                  project={project}
                  action={a}
                  colors={colors}
                  lane="shelf"
                  onChanged={onChanged}
                />
              ))}
            </div>
          </details>
        )}

        {hasActions && (
          <div style={{ fontSize: 10, color: colors.textDim, marginTop: 10 }}>
            Read from your own analytics by your agent, over the last{' '}
            {actions?.periodDays ?? 30} days. Each item cites the figure it came from — check it
            before acting.
          </div>
        )}

        {/* Both guards can silently withhold advice — the reword guard drops a
            suggestion the user never sees, and an untargeted action is discarded
            outright. Counting them out loud is what keeps that auditable; a
            drop nobody is told about is indistinguishable from a model that had
            less to say. */}
        {(droppedRestated > 0 || droppedNoTarget > 0 || droppedPresent > 0) && (
          <div style={{ fontSize: 10, color: colors.textDim, marginTop: 6 }}>
            Last review dropped {droppedRestated + droppedNoTarget + droppedPresent} suggestion(s):{' '}
            {[
              droppedRestated > 0
                ? `${droppedRestated} restated something already on your board`
                : null,
              droppedNoTarget > 0
                ? `${droppedNoTarget} made no measurable prediction`
                : null,
              droppedPresent > 0
                ? `${droppedPresent} already in the repo`
                : null,
            ].filter(Boolean).join(', ')}.
          </div>
        )}
      </section>
    </>
  );
}

function GrowAnalytics({
  project, posts, colors,
}: {
  project: Project;
  posts: SocialCard[];
  colors: ThemeColors;
}) {
  // The growth inbox moved to the Actions lens — this lens is now purely
  // "what happened", with every "so what" living one tab over.
  const connectionRequestGeneration = useRef(0);
  const statsRequestGeneration = useRef(0);

  // Analytics connection + live stats. The connection status loads first;
  // stats only fetch once a provider is connected (no pointless round-trip on
  // the empty state).
  const [conn, setConn] = useState<AnalyticsConnectionStatus | null>(null);
  const [connState, setConnState] = useState<LoadState>('loading');
  const [stats, setStats] = useState<AnalyticsStatsData | null>(null);
  const [statsState, setStatsState] = useState<LoadState>('ready');

  const loadStats = useCallback((id: string) => {
    const generation = ++statsRequestGeneration.current;
    setStatsState('loading');
    apiFetch<AnalyticsStatsData>(`/api/projects/${encodeURIComponent(id)}/analytics/stats?period=30d`)
      .then((s) => {
        if (generation !== statsRequestGeneration.current) return;
        setStats(s);
        setStatsState('ready');
      })
      .catch(() => {
        if (generation !== statsRequestGeneration.current) return;
        setStats(null);
        setStatsState('error');
      });
  }, []);

  const loadConnection = useCallback((id: string) => {
    const generation = ++connectionRequestGeneration.current;
    ++statsRequestGeneration.current;
    setConnState('loading');
    setStats(null);
    apiFetch<AnalyticsConnectionStatus>(`/api/projects/${encodeURIComponent(id)}/analytics/connection`)
      .then((c) => {
        if (generation !== connectionRequestGeneration.current) return;
        setConn(c);
        setConnState('ready');
        if (c.connected) loadStats(id);
      })
      .catch(() => {
        if (generation !== connectionRequestGeneration.current) return;
        setConn(null);
        setConnState('error');
      });
  }, [loadStats]);

  useEffect(() => {
    loadConnection(project.id);
    return () => {
      ++connectionRequestGeneration.current;
      ++statsRequestGeneration.current;
    };
  }, [project.id, loadConnection]);

  // First-party (self-hosted) analytics — preferred path; the connector stays
  // for people who already have a provider account.
  const [fpStats, setFpStats] = useState<FirstPartyStats | null>(null);
  const fpRequestGeneration = useRef(0);
  const loadFpStats = useCallback((id: string) => {
    const generation = ++fpRequestGeneration.current;
    apiFetch<FirstPartyStats>(`/api/projects/${encodeURIComponent(id)}/analytics/first_party/stats`)
      .then((s) => {
        if (generation !== fpRequestGeneration.current) return;
        setFpStats(s);
      })
      .catch(() => {
        if (generation !== fpRequestGeneration.current) return;
        setFpStats(null);
      });
  }, []);
  useEffect(() => {
    loadFpStats(project.id);
    return () => { ++fpRequestGeneration.current; };
  }, [project.id, loadFpStats]);

  const connected = conn?.connected ?? false;
  const providerLabel = conn?.provider ? PROVIDER_LABELS[conn.provider] : null;
  const fpLive = !!fpStats?.receiving;
  // First-party numbers win when the collector is receiving; the third-party
  // provider fills in otherwise.
  // First-party counts DEVICE SIGNATURES, not people: the hash collapses
  // everyone sharing a browser build, OS version and language into one value,
  // which on mobile merges many real people. It systematically undercounts, so
  // the label changes with the source rather than presenting both as "Visitors".
  const visitors = fpLive ? fpStats!.deviceSignatures : connected ? stats?.visitors ?? null : null;
  const pageviews = fpLive ? fpStats!.pageviews : connected ? stats?.pageviews ?? null : null;
  const fetchFailed = connected && (statsState === 'error' || !!stats?.error);

  // Hint for a connected-but-valueless metric slot: fetching, failed, or the
  // provider genuinely doesn't expose it (e.g. GoatCounter has no site-wide
  // pageview aggregate) — each state named honestly.
  const liveHint = (notExposed: string, awaiting: string): string => {
    if (!connected) return awaiting;
    if (statsState === 'loading') return 'Fetching…';
    if (fetchFailed) return 'Fetch failed — see the connection panel';
    return notExposed;
  };

  // The classic growth funnel (research: awareness → interest → action →
  // retention). Awareness/reach comes from published content; Visitors is
  // live once analytics is connected; signups/retention need provider goal
  // events (flagged follow-up).
  const funnel = [
    { stage: 'Content live', value: posts.length, source: true, hint: 'Published social posts' },
    { stage: 'Reach', value: null as number | null, source: false, hint: 'Impressions — connect a channel' },
    {
      stage: fpLive ? 'Devices' : 'Visitors',
      value: visitors,
      source: visitors != null,
      hint: fpLive
        ? 'Distinct device signatures, not people — browsers sharing a build, OS and language merge into one, so this undercounts'
        : liveHint(`Not exposed by ${providerLabel}`, 'Site sessions — connect analytics below'),
    },
    {
      stage: 'Signups',
      value: null as number | null,
      source: false,
      hint: connected ? `Needs goal events in ${providerLabel} — follow-up` : 'Conversions — connect analytics below',
    },
    {
      stage: 'Retained',
      value: null as number | null,
      source: false,
      hint: connected ? `Needs goal events in ${providerLabel} — follow-up` : 'Return users — connect analytics below',
    },
  ];
  const maxV = Math.max(1, ...funnel.map((f) => f.value ?? 0));

  const tiles = [
    { label: 'POSTS PUBLISHED', value: String(posts.length), sub: 'this project' },
    { label: 'ACTIVE CHANNELS', value: '0', sub: 'connect in the epic' },
    {
      label: 'REACH (30D)',
      value: pageviews != null ? pageviews.toLocaleString() : '—',
      sub: pageviews != null
        ? `pageviews · ${fpLive ? 'self-hosted' : providerLabel}`
        : liveHint(`not exposed by ${providerLabel}`, 'awaiting analytics'),
    },
    {
      label: 'CONVERSIONS',
      value: '—',
      sub: connected ? 'needs provider goals — follow-up' : 'awaiting analytics',
    },
  ];

  // Third-party providers are an OPT-IN alternative, not a co-equal choice.
  // Permagent's own collector is the encouraged path: it keeps the data on the
  // user's infrastructure, so putting a vendor connection at the same visual
  // weight would steer people away from that for no reason. Collapsed unless
  // already connected, or deliberately opened.
  const [showProviders, setShowProviders] = useState(false);
  const providersOpen = showProviders || !!conn?.provider;

  return (
    <>
      <div style={{
        fontSize: 11, color: colors.textDim, background: colors.bgDeeper,
        border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '8px 12px', marginBottom: 4,
      }}>
        Analytics for <strong style={{ color: colors.text }}>{project.name}</strong>, collected by
        Permagent onto your own infrastructure — nothing here is faked or shared. What to DO about
        it lives in <strong style={{ color: colors.text }}>Actions</strong>.
      </div>

      {/* Self-hosted analytics (#23) — the daemon is the collector.
          KEYED ON THE PROJECT. `loadSetup` refetches on a projectId change and
          guards stale responses with a generation counter, but the panel's
          other state does not reset — so after verifying Evntally and switching
          to GetLadle, the previous project's PASS was still on screen, telling
          the user analytics was installed here when it was not (reported
          2026-08-04). In a surface where every failure is silent, a false
          "verified" is the worst thing this panel can say. Keying remounts it,
          which clears the whole class rather than the one field that leaked. */}
      <FirstPartyAnalyticsPanel
        key={project.id}
        colors={colors}
        projectId={project.id}
        stats={fpStats}
        onRefresh={() => loadFpStats(project.id)}
      />

      {/* Conversion funnel over first-party events — only once the collector
          is live; an empty form on a project with no data is noise. Keyed on
          the project so saved steps and results never leak across a switch
          (the same class of bug as the verify PASS leak above). */}
      {fpLive && <FunnelPanel key={`funnel-${project.id}`} projectId={project.id} colors={colors} />}

      {/* Third-party connection, deliberately understated. */}
      {!providersOpen ? (
        <button
          onClick={() => setShowProviders(true)}
          style={{
            alignSelf: 'flex-start', background: 'transparent', border: 'none',
            color: colors.textDim, fontFamily: font.body, fontSize: 11,
            cursor: 'pointer', padding: '2px 0', textDecoration: 'underline',
          }}
        >
          Already use Plausible, Fathom or GA? Connect it read-only instead
        </button>
      ) : (
        <AnalyticsConnectionPanel
          key={project.id}
          colors={colors}
          projectId={project.id}
          conn={conn}
          connState={connState}
          stats={stats}
          statsState={statsState}
          onReload={() => loadConnection(project.id)}
          onRefreshStats={() => loadStats(project.id)}
        />
      )}

      {/* Metric tiles */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))', gap: 12 }}>
        {tiles.map((t) => (
          <div key={t.label} style={{
            background: colors.surface, border: `1px solid ${colors.border}`,
            borderRadius: radius.lg, padding: 16,
          }}>
            <div style={{ fontFamily: font.display, fontSize: 26, fontWeight: 700, color: colors.text, fontVariantNumeric: 'tabular-nums' }}>{t.value}</div>
            <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, letterSpacing: '0.08em', marginTop: 4 }}>{t.label}</div>
            <div style={{ fontSize: 10, color: colors.textDim, marginTop: 2 }}>{t.sub}</div>
          </div>
        ))}
      </div>

      {/* Funnel */}
      <section style={{ marginTop: 8 }}>
        <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 12px' }}>Growth funnel</h3>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {funnel.map((f) => (
            <div key={f.stage} style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <div style={{ width: 96, fontSize: 12, color: colors.textMuted, textAlign: 'right', flexShrink: 0 }}>{f.stage}</div>
              <div style={{ flex: 1, height: 26, background: colors.bgDeeper, borderRadius: radius.sm, overflow: 'hidden', position: 'relative' }}>
                {f.source ? (
                  <div style={{
                    width: `${Math.max(6, ((f.value ?? 0) / maxV) * 100)}%`, height: '100%',
                    background: `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`,
                    borderRadius: radius.sm,
                  }} />
                ) : (
                  <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', paddingLeft: 10 }}>
                    <span style={{ fontSize: 10, color: colors.textDim, fontStyle: 'italic' }}>{f.hint}</span>
                  </div>
                )}
              </div>
              <div style={{
                minWidth: 40, textAlign: 'right', flexShrink: 0, fontFamily: font.mono, fontSize: 12,
                color: colors.text, fontVariantNumeric: 'tabular-nums',
              }}>{f.source ? f.value?.toLocaleString() : ''}</div>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}

// ── First-party analytics panel (#23) ────────────────────────────────────────
// The self-hosted path: enable → copy the agent prompt (a coding agent adds
// the snippet to the site) → come back and the panel flips live on the first
// beacon. No third-party dependency; the daemon is the collector.

function FirstPartyAnalyticsPanel({
  colors, projectId, stats, onRefresh,
}: {
  colors: ThemeColors;
  projectId: string;
  stats: FirstPartyStats | null;
  onRefresh: () => void;
}) {
  const [setup, setSetup] = useState<FirstPartySetup | null>(null);
  const [setupState, setSetupState] = useState<LoadState>('loading');
  const [ingestBase, setIngestBase] = useState('');
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState<'snippet' | 'prompt' | null>(null);
  const generation = useRef(0);

  const loadSetup = useCallback(() => {
    const gen = ++generation.current;
    setSetupState('loading');
    apiFetch<FirstPartySetup>(`/api/projects/${encodeURIComponent(projectId)}/analytics/first_party`)
      .then((s) => {
        if (gen !== generation.current) return;
        setSetup(s);
        setIngestBase(s.ingestBase ?? '');
        setSetupState('ready');
      })
      .catch(() => {
        if (gen !== generation.current) return;
        setSetupState('error');
      });
  }, [projectId]);

  useEffect(() => {
    loadSetup();
    return () => { ++generation.current; };
  }, [loadSetup]);

  // While enabled but not yet receiving, poll so "come back and it's flowing"
  // needs no manual refresh. 10s is plenty; stops once live.
  useEffect(() => {
    if (!setup?.enabled || stats?.receiving) return;
    const interval = setInterval(onRefresh, 10_000);
    return () => clearInterval(interval);
  }, [setup?.enabled, stats?.receiving, onRefresh]);

  const enable = useCallback((base?: string) => {
    setSaving(true);
    apiFetch<FirstPartySetup>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/enable`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(base !== undefined ? { ingestBase: base } : {}),
      },
    )
      .then((s) => {
        setSetup(s);
        setIngestBase(s.ingestBase ?? '');
        setSetupState('ready');
        onRefresh();
      })
      .catch(() => setSetupState('error'))
      .finally(() => setSaving(false));
  }, [projectId, onRefresh]);

  // Point the daemon at the site's drain endpoint — the URL the coding agent
  // reports back after installing the relay.
  const setDrain = useCallback((url: string) => {
    setSaving(true);
    apiFetch<FirstPartySetup>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/drain`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ drainUrl: url }),
      },
    )
      .then((s) => {
        setSetup(s);
        setSetupState('ready');
        onRefresh();
      })
      .catch(() => setSetupState('error'))
      .finally(() => setSaving(false));
  }, [projectId, onRefresh]);

  // Install verification — the loud failure signal analytics otherwise lacks.
  // Every failure mode here is silent (202s, empty catch blocks, a 401 that
  // looks like a wrong key), so this runs the assertions against the DEPLOYED
  // origin rather than trusting the coding agent's report.
  const [verifying, setVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<VerifyResponse | null>(null);
  const runVerify = useCallback((origin: string) => {
    if (!origin) return;
    setVerifying(true);
    setVerifyResult(null);
    apiFetch<VerifyResponse>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/verify`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ origin, secondRoute: '/about' }),
      },
    )
      .then(setVerifyResult)
      .catch((e) => setVerifyResult({
        verified: false,
        checks: [],
        summary: `Could not run verification: ${e instanceof Error ? e.message : String(e)}`,
      }))
      .finally(() => setVerifying(false));
  }, [projectId]);

  // Rotate the drain secret. It ships inside the install brief, so it lands in
  // the coding agent's transcript and tool logs — a credential that has passed
  // through a third-party model's context should be replaceable without
  // rebuilding the install. Rotating 401s the deployed site until the new value
  // is set on the app service and it redeploys.
  const [rotating, setRotating] = useState(false);
  const rotateSecret = useCallback(() => {
    if (!window.confirm(
      'Mint a new drain key?\n\nIngestion will fail with 401 until you set the new value on '
      + 'your app service and redeploy. Copy the fresh brief afterwards.',
    )) return;
    setRotating(true);
    apiFetch<FirstPartySetup>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/rotate`,
      { method: 'POST' },
    )
      .then((s) => { setSetup(s); onRefresh(); })
      .catch(() => setSetupState('error'))
      .finally(() => setRotating(false));
  }, [projectId, onRefresh]);

  const copy = useCallback((kind: 'snippet' | 'prompt', text: string | null | undefined) => {
    if (!text) return;
    navigator.clipboard?.writeText(text).then(() => {
      setCopied(kind);
      setTimeout(() => setCopied((c) => (c === kind ? null : c)), 1600);
    });
  }, []);

  const shell: React.CSSProperties = {
    background: colors.surface, border: `1px solid ${colors.border}`,
    borderRadius: radius.lg, padding: 16, display: 'flex', flexDirection: 'column', gap: 10,
  };
  const buttonStyle: React.CSSProperties = {
    background: colors.bgDeeper, color: colors.text, border: `1px solid ${colors.border}`,
    borderRadius: radius.md, padding: '6px 12px', fontSize: 12, cursor: 'pointer',
  };

  if (setupState === 'error') {
    return (
      <div style={shell}>
        <ErrorState colors={colors} inline message="Couldn't load self-hosted analytics." onRetry={loadSetup} />
      </div>
    );
  }

  // Not yet enabled: the offer.
  if (!setup?.enabled) {
    return (
      <div style={shell}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div>
            <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>Self-hosted analytics</div>
            <div style={{ fontSize: 11, color: colors.textDim, marginTop: 2 }}>
              Your daemon collects pageviews directly — no third-party account, your data stays here.
            </div>
          </div>
          <button
            style={{ ...buttonStyle, opacity: setupState === 'loading' || saving ? 0.6 : 1 }}
            disabled={setupState === 'loading' || saving}
            onClick={() => enable()}
          >{saving ? 'Enabling…' : 'Enable'}</button>
        </div>
      </div>
    );
  }

  const receiving = !!stats?.receiving;

  return (
    <div style={shell}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>
          Self-hosted analytics
          {receiving && (
            <span style={{ marginLeft: 8, fontSize: 10, color: colors.cyan, fontFamily: font.mono }}>
              ● live{stats && stats.eventsLast5m > 0 ? ` · ${stats.eventsLast5m} events / 5m` : ''}
            </span>
          )}
        </div>
        <button style={buttonStyle} onClick={onRefresh}>Refresh</button>
      </div>

      {!receiving && (
        <>
          <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
            <b style={{ color: colors.text }}>Step 1.</b> Copy the install brief below and give it to
            a coding agent inside this project's repo. It builds the relay: visitors beacon
            same-origin to your own app, which buffers events in your own database.
            <br />
            <b style={{ color: colors.text }}>Step 2.</b> Paste the drain URL it reports back. This
            Mac then pulls events outbound every couple of minutes — nothing here is ever exposed to
            the internet, and events survive while it sleeps.
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button style={buttonStyle} onClick={() => copy('prompt', setup.agentPrompt)}>
              {copied === 'prompt' ? 'Copied ✓' : 'Copy install brief'}
            </button>
            <button style={buttonStyle} onClick={() => copy('snippet', setup.snippet)}>
              {copied === 'snippet' ? 'Copied ✓' : 'Copy snippet only'}
            </button>
            <button
              style={{ ...buttonStyle, opacity: rotating ? 0.6 : 1 }}
              disabled={rotating}
              title="Mint a new drain key — the old one stops working immediately"
              onClick={rotateSecret}
            >{rotating ? 'Rotating…' : 'Rotate key'}</button>
          </div>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
            <input
              value={ingestBase}
              onChange={(e) => setIngestBase(e.target.value)}
              placeholder="https://yoursite.com/api/permagent-analytics/drain"
              style={{
                flex: '1 1 260px', background: colors.bgDeeper, color: colors.text,
                border: `1px solid ${colors.border}`, borderRadius: radius.md,
                padding: '6px 10px', fontSize: 12, fontFamily: font.mono,
              }}
            />
            <button
              style={{ ...buttonStyle, opacity: saving ? 0.6 : 1 }}
              disabled={saving}
              onClick={() => setDrain(ingestBase.trim())}
            >{saving ? 'Saving…' : 'Start ingesting'}</button>
            <button
              style={{ ...buttonStyle, opacity: verifying ? 0.6 : 1 }}
              disabled={verifying}
              title="Fetch the deployed site and assert the install actually works"
              onClick={() => {
                // Derive the origin from the drain URL the agent reported.
                const url = ingestBase.trim() || setup.drainUrl || '';
                try { runVerify(new URL(url).origin); } catch { /* not a URL yet */ }
              }}
            >{verifying ? 'Verifying…' : 'Verify install'}</button>
          </div>
          {verifyResult && (
            <div style={{
              fontSize: 11, fontFamily: font.mono, whiteSpace: 'pre-wrap',
              background: colors.bgDeeper, borderRadius: radius.md, padding: 10,
              border: `1px solid ${verifyResult.verified ? colors.border : colors.danger}`,
              color: verifyResult.verified ? colors.textMuted : colors.text,
              maxHeight: 260, overflowY: 'auto',
            }}>{verifyResult.summary}</div>
          )}
          {setup.lastError && (
            <div style={{ fontSize: 10, color: colors.danger, fontFamily: font.mono }}>
              Last drain failed: {setup.lastError}
            </div>
          )}
          <div style={{ fontSize: 10, color: colors.textDim }}>
            {setup.drainUrl
              ? `Draining from ${setup.drainUrl}${setup.lastDrainAt ? ` · last checked ${new Date(setup.lastDrainAt).toLocaleTimeString()}` : ' · waiting for the first pass…'}`
              : 'Waiting for a drain URL.'}
          </div>
        </>
      )}
      {receiving && (() => {
        if (setup.lastError) {
          return (
            <div style={{ fontSize: 10, color: colors.danger, fontFamily: font.mono }}>
              Drain failing: {setup.lastError}
            </div>
          );
        }
        // Drain health, subtly, but honest: freshness comes from stats (it
        // refreshes with the panel; setup only loads once), and a drain that
        // has gone quiet for over an hour — or a relay holding events we have
        // not pulled — gets the warning tint. A stale figure must never read
        // as a quiet day (the botsExcluded rule).
        const fresh = drainFreshness(stats?.lastDrainAt ?? setup.lastDrainAt);
        if (!fresh) return null;
        const lag = stats?.drainLagEvents ?? 0;
        return (
          <div style={{
            fontSize: 10, fontFamily: font.mono,
            color: fresh.stale || lag > 0 ? colors.warning : colors.textDim,
          }}>
            {fresh.label}
            {lag > 0 && <> · {lag.toLocaleString()} event{lag === 1 ? '' : 's'} behind</>}
            {fresh.stale && <> · figures may be behind</>}
          </div>
        );
      })()}

      {receiving && stats && (
        <>
          {/* Daily pageviews across the WHOLE window.
              byDay only returns days that have traffic, so plotting it directly
              gave one full-width bar per active day, every one at 100% height —
              a solid colour block that carried no information at all. Padding
              the window with zero-days turns it back into a real shape: two
              busy days out of thirty should look like two spikes, not a wall. */}
          {(() => {
            const byDay = new Map(stats.byDay.map((d) => [d.day, d]));
            const days: { day: string; pageviews: number; visitors: number }[] = [];
            for (let i = stats.periodDays - 1; i >= 0; i--) {
              const dt = new Date();
              dt.setDate(dt.getDate() - i);
              const key = dt.toISOString().slice(0, 10);
              days.push(byDay.get(key) ?? { day: key, pageviews: 0, visitors: 0 });
            }
            const max = Math.max(1, ...days.map((d) => d.pageviews));
            return (
              <div style={{ display: 'flex', alignItems: 'flex-end', gap: 1, height: 48 }}>
                {days.map((d) => (
                  <div
                    key={d.day}
                    title={d.pageviews > 0
                      ? `${d.day}: ${d.pageviews} pageviews · ${d.visitors} devices`
                      : `${d.day}: no traffic`}
                    style={{
                      flex: 1, minWidth: 2,
                      // A zero day is a hairline, not a bar — visibly empty
                      // rather than a misleading minimum-height stub.
                      height: d.pageviews > 0 ? `${Math.max(8, (d.pageviews / max) * 100)}%` : '1px',
                      background: d.pageviews > 0
                        ? `linear-gradient(180deg, ${colors.cyan}, ${colors.purple})`
                        : colors.border,
                      borderRadius: 1,
                      opacity: d.pageviews > 0 ? 0.9 : 1,
                    }}
                  />
                ))}
              </div>
            );
          })()}
          {/* Headline figures, each labelled for what it actually is. */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(110px, 1fr))', gap: 10 }}>
            {([
              ['Pageviews', stats.pageviews.toLocaleString(), `last ${stats.periodDays} days`],
              // NOT "visitors": the hash merges people sharing a browser build,
              // OS and language, so it undercounts — badly on mobile.
              ['Devices', stats.deviceSignatures.toLocaleString(), 'distinct signatures, undercounts'],
              ['Sessions', stats.sessions > 0 ? stats.sessions.toLocaleString() : '—',
                stats.sessions > 0 ? 'first-party, no cookie' : 'relay predates sessions'],
              ['Bounce', stats.bounceRate != null ? `${Math.round(stats.bounceRate * 100)}%` : '—',
                stats.bounceRate != null ? 'one-page sessions' : 'needs sessions'],
              ['Pages / session', stats.pagesPerSession != null ? stats.pagesPerSession.toFixed(1) : '—',
                stats.pagesPerSession != null ? 'depth' : 'needs sessions'],
            ] as const).map(([label, value, sub]) => (
              <div key={label} style={{
                background: colors.bgDeeper, border: `1px solid ${colors.border}`,
                borderRadius: radius.md, padding: '8px 10px',
              }}>
                <div style={{ fontFamily: font.display, fontSize: 20, fontWeight: 700, color: colors.text, fontVariantNumeric: 'tabular-nums' }}>{value}</div>
                <div style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, letterSpacing: '0.06em', textTransform: 'uppercase', marginTop: 2 }}>{label}</div>
                <div style={{ fontSize: 9, color: colors.textDim, marginTop: 1 }}>{sub}</div>
              </div>
            ))}
          </div>
          <div style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>
            {stats.pageviews.toLocaleString()} pageviews · {stats.deviceSignatures.toLocaleString()} devices
            {stats.sessions > 0 && <> · {stats.sessions.toLocaleString()} sessions</>}
            {stats.bounceRate != null && <> · {Math.round(stats.bounceRate * 100)}% bounce</>}
            {' '}· last {stats.periodDays}d
            {/* A filtered figure must never read as a quiet day. */}
            {stats.botsExcluded > 0 && !stats.includingBots && (
              <> · {stats.botsExcluded.toLocaleString()} bot hits excluded</>
            )}
          </div>
          {(stats.aeoVisits ?? 0) > 0 && (
            <div style={{ fontSize: 11, color: colors.textMuted, marginBottom: 8 }}>
              <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, letterSpacing: '0.08em', textTransform: 'uppercase' }}>AEO</span>
              {' '}{(stats.aeoVisits ?? 0).toLocaleString()} answer-engine visit{(stats.aeoVisits === 1) ? '' : 's'}
            </div>
          )}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 12 }}>
            {([
              ['Top pages', stats.topPages],
              ['Sources', stats.topSources],
              ['Referrers', stats.topReferrers],
              ['Campaigns', stats.topCampaigns],
              ['Entry pages', stats.topEntryPages],
              ['Events', stats.topEvents],
            ] as const).map(([label, rows]) => (
              <div key={label}>
                <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, letterSpacing: '0.08em', textTransform: 'uppercase', marginBottom: 4 }}>{label}</div>
                {rows.length === 0 && <div style={{ fontSize: 11, color: colors.textDim }}>—</div>}
                {rows.slice(0, 5).map((r) => (
                  <div key={r.name} style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 11, color: colors.textMuted, padding: '2px 0' }}>
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{r.name}</span>
                    <span style={{ fontFamily: font.mono, color: colors.text }}>{r.count.toLocaleString()}</span>
                  </div>
                ))}
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

// ── Analytics connection panel ───────────────────────────────────────────────
// The "connect analytics" settings surface on the analytics lens. Every
// control hits a real endpoint (save / test / stats / disconnect) — no dead
// UI. The API key is write-only: sent on save, never read back.

function AnalyticsConnectionPanel({
  colors, projectId, conn, connState, stats, statsState, onReload, onRefreshStats,
}: {
  colors: ThemeColors;
  projectId: string;
  conn: AnalyticsConnectionStatus | null;
  connState: LoadState;
  stats: AnalyticsStatsData | null;
  statsState: LoadState;
  onReload: () => void;
  onRefreshStats: () => void;
}) {
  const [showForm, setShowForm] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [disconnecting, setDisconnecting] = useState(false);

  // Transient panel state must not leak across a project switch.
  useEffect(() => { setShowForm(false); setTestResult(null); }, [projectId]);

  const runTest = () => {
    setTesting(true);
    setTestResult(null);
    apiFetch<AnalyticsTestResult>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/connection/test`,
      { method: 'POST' },
    )
      .then((r) => setTestResult(r.ok
        ? {
          ok: true,
          message: `Connection OK — ${(r.visitors ?? 0).toLocaleString()} visitors in the last 7 days.`,
        }
        : { ok: false, message: r.error ?? 'Test failed.' }))
      .catch((e: Error) => setTestResult({ ok: false, message: e.message }))
      .finally(() => setTesting(false));
  };

  const disconnect = () => {
    setDisconnecting(true);
    apiFetch(`/api/projects/${encodeURIComponent(projectId)}/analytics/connection`, { method: 'DELETE' })
      .then(() => { setTestResult(null); setShowForm(false); onReload(); })
      .catch((e: Error) => setTestResult({ ok: false, message: e.message }))
      .finally(() => setDisconnecting(false));
  };

  const btnStyle: CSSProperties = {
    fontSize: 11, fontFamily: font.body, color: colors.text,
    background: 'transparent', border: `1px solid ${colors.border}`,
    borderRadius: radius.md, padding: '4px 10px', cursor: 'pointer',
  };

  if (connState === 'error') {
    return <ErrorState colors={colors} inline message="Couldn't load the analytics connection." onRetry={onReload} />;
  }
  if (connState === 'loading') {
    return <LoadingState colors={colors} inline label="Checking analytics connection…" />;
  }

  if (showForm) {
    return (
      <AnalyticsConnectForm
        colors={colors}
        projectId={projectId}
        conn={conn}
        onCancel={() => setShowForm(false)}
        onSaved={() => { setShowForm(false); setTestResult(null); onReload(); }}
      />
    );
  }

  if (!conn?.connected) {
    return (
      <div style={{
        border: `1px dashed ${colors.border}`, borderRadius: radius.lg, padding: '16px 18px',
        display: 'flex', alignItems: 'center', gap: 14, flexWrap: 'wrap',
      }}>
        <div style={{ flex: 1, minWidth: 220 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>Connect analytics</div>
          <div style={{ fontSize: 11, color: colors.textDim, marginTop: 3, lineHeight: 1.5 }}>
            Point the funnel at your existing Plausible or GoatCounter account — a read-only stats
            fetch, your data stays where it is.
          </div>
        </div>
        <button
          onClick={() => setShowForm(true)}
          style={{
            fontSize: 12, fontFamily: font.body, color: colors.cyan, background: colors.cyanSoft,
            border: `1px solid ${colors.borderHi}`, borderRadius: radius.md, padding: '7px 14px', cursor: 'pointer',
          }}
        >Connect analytics</button>
      </div>
    );
  }

  const providerLabel = conn.provider ? PROVIDER_LABELS[conn.provider] : conn.provider;
  const statsLine = statsState === 'loading'
    ? 'Fetching stats…'
    : statsState === 'error'
      ? 'Stats fetch failed — the daemon may be unreachable.'
      : stats?.error
        ? stats.error
        : stats
          ? [
            stats.visitors != null ? `${stats.visitors.toLocaleString()} visitors` : null,
            stats.pageviews != null ? `${stats.pageviews.toLocaleString()} pageviews` : null,
          ].filter(Boolean).join(' · ') + ` (last ${stats.periodDays ?? 30}d)`
          : '';
  const statsFailed = statsState === 'error' || !!stats?.error;

  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: '12px 14px',
      display: 'flex', flexDirection: 'column', gap: 8,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span aria-hidden style={{
          width: 8, height: 8, borderRadius: '50%', flexShrink: 0,
          background: statsFailed ? colors.warning : colors.success,
        }} />
        <span style={{ fontSize: 12, fontWeight: 600, color: colors.text }}>{providerLabel}</span>
        <span style={{ fontSize: 11, color: colors.textMuted, fontFamily: font.mono }}>
          {conn.baseUrl}{conn.siteId ? ` · ${conn.siteId}` : ''}
        </span>
        <div style={{ flex: 1 }} />
        <button onClick={onRefreshStats} disabled={statsState === 'loading'} style={btnStyle}>Refresh</button>
        <button onClick={runTest} disabled={testing} style={btnStyle}>{testing ? 'Testing…' : 'Test connection'}</button>
        <button onClick={() => { setTestResult(null); setShowForm(true); }} style={btnStyle}>Edit</button>
        <button
          onClick={disconnect}
          disabled={disconnecting}
          style={{ ...btnStyle, color: colors.warning }}
        >{disconnecting ? 'Disconnecting…' : 'Disconnect'}</button>
      </div>
      {statsLine && (
        <div style={{ fontSize: 11, color: statsFailed ? colors.warning : colors.textMuted }}>{statsLine}</div>
      )}
      {testResult && (
        <div style={{ fontSize: 11, color: testResult.ok ? colors.success : colors.warning }}>{testResult.message}</div>
      )}
    </div>
  );
}

// The connect/edit form. Provider, base URL, site id, API key — saved via
// PUT /analytics/connection. The key field is write-only: when one is already
// stored, leaving it blank keeps it.
function AnalyticsConnectForm({
  colors, projectId, conn, onSaved, onCancel,
}: {
  colors: ThemeColors;
  projectId: string;
  conn: AnalyticsConnectionStatus | null;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const [provider, setProvider] = useState<AnalyticsProviderId>(conn?.provider ?? 'plausible');
  const [baseUrl, setBaseUrl] = useState(conn?.baseUrl ?? '');
  const [siteId, setSiteId] = useState(conn?.siteId ?? '');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const needsSiteId = provider !== 'goatcounter';
  const hasStoredKey = conn?.hasApiKey ?? false;
  const canSave = baseUrl.trim() !== ''
    && (!needsSiteId || siteId.trim() !== '')
    && (hasStoredKey || apiKey.trim() !== '');

  const baseUrlPlaceholder = provider === 'goatcounter'
    ? 'https://yoursite.goatcounter.com'
    : provider === 'plausible_v2'
      ? 'https://plausible.io'
      : 'https://plausible.example.com (or https://plausible.io)';

  const save = () => {
    setSaving(true);
    setError(null);
    const body: Record<string, string> = {
      provider,
      baseUrl: baseUrl.trim(),
      siteId: needsSiteId ? siteId.trim() : '',
    };
    if (apiKey.trim()) body.apiKey = apiKey.trim();
    apiFetch<AnalyticsConnectionStatus>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/connection`,
      { method: 'PUT', body: JSON.stringify(body) },
    )
      .then(() => { setApiKey(''); onSaved(); })
      .catch((e: Error) => setError(e.message))
      .finally(() => setSaving(false));
  };

  const fieldStyle: CSSProperties = {
    background: colors.bgDeeper, color: colors.text, border: `1px solid ${colors.border}`,
    borderRadius: radius.md, padding: '6px 10px', fontSize: 12, fontFamily: font.body, width: '100%',
    boxSizing: 'border-box',
  };
  const labelStyle: CSSProperties = {
    fontSize: 10, fontFamily: font.mono, color: colors.textDim,
    textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 4, display: 'block',
  };

  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: 16, display: 'flex', flexDirection: 'column', gap: 12,
    }}>
      <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>
        {conn?.connected ? 'Edit analytics connection' : 'Connect analytics'}
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))', gap: 12 }}>
        <label>
          <span style={labelStyle}>Provider</span>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value as AnalyticsProviderId)}
            style={fieldStyle}
          >
            {(Object.keys(PROVIDER_LABELS) as AnalyticsProviderId[]).map((p) => (
              <option key={p} value={p}>{PROVIDER_LABELS[p]}</option>
            ))}
          </select>
        </label>
        <label>
          <span style={labelStyle}>Base URL</span>
          <input
            type="url"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={baseUrlPlaceholder}
            style={fieldStyle}
          />
        </label>
        {needsSiteId && (
          <label>
            <span style={labelStyle}>Site ID (domain)</span>
            <input
              type="text"
              value={siteId}
              onChange={(e) => setSiteId(e.target.value)}
              placeholder="example.com"
              style={fieldStyle}
            />
          </label>
        )}
        <label>
          <span style={labelStyle}>API key</span>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={hasStoredKey ? 'stored — leave blank to keep' : 'paste your stats API key'}
            autoComplete="off"
            style={fieldStyle}
          />
        </label>
      </div>
      <div style={{ fontSize: 10, color: colors.textDim, lineHeight: 1.5 }}>
        {provider === 'goatcounter'
          ? 'GoatCounter: your site lives in the URL (no separate site id). Create an API token under Settings → API in your GoatCounter dashboard.'
          : 'Plausible: the site id is the domain as it appears in Plausible. Create a Stats API key under Settings → API keys.'}
        {' '}Read-only — this never writes to your analytics account.
      </div>
      {error && <div style={{ fontSize: 11, color: colors.warning }}>{error}</div>}
      <div style={{ display: 'flex', gap: 8 }}>
        <button
          onClick={save}
          disabled={!canSave || saving}
          style={{
            fontSize: 12, fontFamily: font.body,
            color: canSave ? colors.cyan : colors.textDim,
            background: canSave ? colors.cyanSoft : 'transparent',
            border: `1px solid ${canSave ? colors.borderHi : colors.border}`,
            borderRadius: radius.md, padding: '6px 14px',
            cursor: canSave && !saving ? 'pointer' : 'default',
          }}
        >{saving ? 'Saving…' : 'Save connection'}</button>
        <button
          onClick={onCancel}
          style={{
            fontSize: 12, fontFamily: font.body, color: colors.textMuted, background: 'transparent',
            border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '6px 14px', cursor: 'pointer',
          }}
        >Cancel</button>
      </div>
    </div>
  );
}

// ── Growth inbox (Analytics lens headline) ───────────────────────────────────
// The deterministic inbox rendered atop the analytics lens: this week's ranked
// moves + a "keep doing" wins strip. All content comes from the backend ranker
// (grow.rs) — this component only presents it, with honest loading / error /
// empty states. No Henry drafting hand-offs here (those belong to GrowView's
// prompt seams); the inbox is informational.

function priorityMeta(priority: MovePriority, colors: ThemeColors): { label: string; color: string } {
  switch (priority) {
    case 'high': return { label: 'High priority', color: colors.warning };
    case 'medium': return { label: 'Medium priority', color: colors.cyan };
    default: return { label: 'Low priority', color: colors.textDim };
  }
}

function GrowthInboxSection({
  colors, state, inbox, onRetry,
}: {
  colors: ThemeColors;
  state: LoadState;
  inbox: GrowthInboxData | null;
  onRetry: () => void;
}) {
  // Defensive against a partial payload. This section previously only rendered
  // inside the Analytics lens; it is now the top of Actions, the default tab,
  // so a malformed response would crash the first thing the user sees.
  const signal = inbox?.signal;
  const moves = inbox?.moves ?? [];
  const wins = inbox?.wins ?? [];
  const hasSignal = !!signal && ((signal.posts ?? 0) > 0 || (signal.shipped ?? 0) > 0);
  const empty = !!inbox && moves.length === 0 && wins.length === 0;

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, margin: '0 0 12px', flexWrap: 'wrap' }}>
        <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>
          Your growth moves this week
        </h3>
        {hasSignal && signal && (
          <span style={{ fontSize: 10, color: colors.textDim }}>
            from {signal.posts} {signal.posts === 1 ? 'post' : 'posts'} · {signal.shipped} shipped
          </span>
        )}
      </div>

      {state === 'error' ? (
        <ErrorState colors={colors} inline message="Couldn't load your growth moves." onRetry={onRetry} />
      ) : state === 'loading' ? (
        <SkeletonCards colors={colors} count={3} height={68} />
      ) : !inbox ? null : empty ? (
        <div style={{
          border: `1px dashed ${colors.border}`, borderRadius: radius.lg, padding: 28,
          textAlign: 'center', fontSize: 12, color: colors.textDim, lineHeight: 1.6,
        }}>
          Not enough signal yet. Publish a post or ship a goal and I'll start surfacing your 2-3
          highest-leverage growth moves here each week — ranked, no guesswork.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {moves.length > 0 ? (
            moves.map((m) => <MoveCard key={m.title} move={m} colors={colors} />)
          ) : (
            <div style={{
              border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '12px 14px',
              fontSize: 12, color: colors.textMuted, background: colors.surface,
            }}>
              You're on track — no urgent moves this week. Keep doing what's working below.
            </div>
          )}
          {wins.length > 0 && <WinsStrip wins={wins} colors={colors} />}
        </div>
      )}
    </section>
  );
}

function MoveCard({ move, colors }: { move: GrowthMove; colors: ThemeColors }) {
  const meta = priorityMeta(move.priority, colors);
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 6,
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderLeft: `3px solid ${meta.color}`, borderRadius: radius.md, padding: '12px 14px',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{
          fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase', letterSpacing: '0.06em',
          color: meta.color, border: `1px solid ${meta.color}`, borderRadius: radius.pill, padding: '1px 8px',
        }}>{meta.label}</span>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 10, color: colors.textDim, fontVariantNumeric: 'tabular-nums' }}>
          {move.evidenceCount} {move.evidenceCount === 1 ? 'signal' : 'signals'}
        </span>
      </div>
      <div style={{ fontSize: 14, fontWeight: 600, color: colors.text }}>{move.title}</div>
      <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>{move.why}</div>
    </div>
  );
}

function WinsStrip({ wins, colors }: { wins: GrowthWin[]; colors: ThemeColors }) {
  return (
    <div style={{ marginTop: 4 }}>
      <div style={{ fontSize: 10, fontFamily: font.mono, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>
        Keep doing
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {wins.map((w) => (
          <div key={w.title} style={{
            display: 'flex', alignItems: 'flex-start', gap: 8,
            background: colors.surface, border: `1px solid ${colors.border}`,
            borderLeft: `3px solid ${colors.success}`, borderRadius: radius.md, padding: '10px 12px',
          }}>
            <span aria-hidden style={{ color: colors.success, fontSize: 13, lineHeight: '18px' }}>✓</span>
            <div>
              <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{w.title}</div>
              <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5, marginTop: 2 }}>{w.why}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
