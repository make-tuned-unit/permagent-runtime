// Grow — the tab that follows Build. Build makes the thing; Grow takes it to
// market. Per-project GTM home: the five-pillar strategy canvas, a content
// calendar of social posts, and Henry-driven growth actions. Henry knows the
// project (Brain, people, docs, goals), so Grow drafts and schedules with
// real context — the edge over a generic scheduler.
//
// v1 is the surface + the Henry hand-offs (prepared prompts → chat, the
// pattern used by the Devices/Enricher panels). The multi-platform posting
// backend (a Postiz-style bridge) and outbound sequences are epic follow-ups
// (see the Grow epic); the GTM canvas persists per project via project tags/
// metadata as those land.

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import type { Project } from '../projects/types';

// Appended to every Grow prompt that DRAFTS user-facing copy (value props,
// posts, outreach) so the output reads like a sharp human wrote it, not a
// chatbot. The full voice spec lives in the "humanize" builtin skill; this
// names it and inlines the top AI tells so the draft is humanized even before
// the skill loads. Strategy prompts (audience/positioning/channels) deliberately
// omit it — they produce internal analysis, not copy the user will publish.
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
  return `Build the complete go-to-market strategy for "${projectName}" using everything you know about the project (Brain, people, docs, goals). Work through all five pillars — audience, value, positioning, channels, content — and for EACH one, save your result with the set_project_strategy tool (project: "${projectName}", pillar: "<key>"): content = a 2-3 sentence summary, points = [{label, detail}] labeled specifics (personas with watering holes, channels with fit reasons, alternatives with your counter-positioning), metrics = [{label, value}] stat chips (price hypothesis, audience size, post cadence). The Strategy cards render this as rich content, so fill all three fields. Also save the "workback" pillar: the launch workback schedule — points = [{label: "<date or week>", detail: "<milestone>"}] counting back from launch day. THEN turn the workback into real to-dos: create a Kanban card on this project's board for each concrete milestone with the card_create tool (title = the milestone, description = why it matters and its target week). Finish with a one-paragraph summary.${HUMANIZE_VOICE}`;
}

interface SocialCard {
  id: string;
  title: string;
  description: string;
  // social_post cards carry scheduling in metadata; tolerant read.
  metadata_json?: Record<string, unknown> | null;
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
  byDay: { day: string; pageviews: number; visitors: number }[];
  topPages: { name: string; count: number }[];
  topReferrers: { name: string; count: number }[];
  topEvents: { name: string; count: number }[];
  topSources: { name: string; count: number }[];
  topCampaigns: { name: string; count: number }[];
  sessions: number;
  bounceRate: number | null;
  pagesPerSession: number | null;
  topEntryPages: { name: string; count: number }[];
}

type GrowLens = 'actions' | 'strategy' | 'calendar' | 'analytics';
// Async lifecycle for data-backed sections — loading / ready / error are
// distinct so a fetch failure never masquerades as an empty result.
type LoadState = 'loading' | 'ready' | 'error';

// Actions leads: the point of collecting analytics is deciding what to do.
const LENSES: GrowLens[] = ['actions', 'strategy', 'calendar', 'analytics'];

export function GrowView() {
  const { colors, gradient, reduceMotion } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectsState, setProjectsState] = useState<LoadState>('loading');
  const [activeId, setActiveId] = useState<string | null>(null);
  const [posts, setPosts] = useState<SocialCard[]>([]);
  const [postsState, setPostsState] = useState<LoadState>('loading');
  const [lens, setLens] = useState<GrowLens>('actions');
  const [ctx, setCtx] = useState<{ people: number; goals: number } | null>(null);
  const [focusLens, setFocusLens] = useState<GrowLens | null>(null);
  const postsRequestGeneration = useRef(0);
  const setActivePanel = useCommandCenter((st) => st.setActivePanel);
  const sendMessage = useCommandCenter((st) => st.sendMessage);
  const openChatDock = useCommandCenter((st) => st.openChatDock);
  const openGrowForProject = useCommandCenter((st) => st.openGrowForProject);
  const setOpenGrowForProject = useCommandCenter((st) => st.setOpenGrowForProject);
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
        if (!opts?.silent) { setPosts([]); setPostsState('error'); }
      });
  }, []);

  useEffect(() => {
    if (!activeId) return;
    loadPosts(activeId);
    return () => { ++postsRequestGeneration.current; };
  }, [activeId, loadPosts]);

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

  // Honor a cross-tab deep link (Projects → Grow this project), then CLEAR it
  // (the pendingProjectNavigation consume-then-clear pattern). Without the
  // clear, one agent-driven grow open stuck in the store forever: every later
  // manual Grow visit re-selected that project on remount, and a repeat
  // open for the same project was a silent no-op (same value → no re-render).
  useEffect(() => {
    if (openGrowForProject) {
      setActiveId(openGrowForProject);
      setOpenGrowForProject(null);
    }
  }, [openGrowForProject, setOpenGrowForProject]);

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
      {/* Header + project switcher — brand ribbon accent */}
      <div style={{ position: 'relative', padding: '16px 24px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 16 }}>
        <div style={{ position: 'absolute', left: 0, right: 0, bottom: -1, height: 2, background: `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`, opacity: 0.5 }} />
        <div>
          <div style={{ fontFamily: font.display, fontSize: 16, fontWeight: 600, letterSpacing: '-0.01em' }}>Grow</div>
          <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 3, display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <span>Take {active ? active.name : 'your project'} to market — Henry drafts with the project's real context.</span>
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
                  cursor: 'pointer', fontSize: 11, fontFamily: font.body,
                }}
              >open project ↗</button>
            )}
            {ctx && (
              <span style={{ color: colors.textDim }}>{ctx.goals} {ctx.goals === 1 ? 'goal' : 'goals'} · {ctx.people} {ctx.people === 1 ? 'person' : 'people'}</span>
            )}
          </div>
        </div>
        <div style={{ flex: 1 }} />
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
                  fontSize: 12, fontFamily: font.body, textTransform: 'capitalize',
                  padding: '5px 12px', borderRadius: radius.sm, cursor: 'pointer', border: 'none',
                  background: selected ? colors.cyanSoft : 'transparent',
                  color: selected ? colors.cyan : colors.textMuted,
                  fontWeight: selected ? 600 : 500,
                  outline: 'none',
                  boxShadow: focusLens === l ? `0 0 0 2px ${colors.borderHi}` : 'none',
                  transition: reduceMotion ? 'none' : 'background 150ms ease, color 150ms ease',
                }}
              >{l}</button>
            );
          })}
        </div>
        <select
          value={activeId ?? ''}
          onChange={(e) => setActiveId(e.target.value)}
          aria-label="Select project"
          style={{
            background: colors.bgDeeper, color: colors.text, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: '6px 10px', fontSize: 13, fontFamily: font.body,
          }}
        >
          {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
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
          role="tabpanel"
          aria-label={`${lens} view`}
          style={{ flex: 1, overflowY: 'auto', padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 20 }}
        >
          {lens === 'actions' && <GrowActions project={active} colors={colors} />}
          {lens === 'analytics' && <GrowAnalytics project={active} posts={posts} colors={colors} />}
          {lens === 'strategy' && (
          <section>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', margin: '0 0 12px' }}>
              <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>Go-to-market strategy</h3>
              <button
                onClick={() => send(runAllPrompt(active.name))}
                title="Henry researches every pillar and fills these cards with the results"
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
                onClick={() => send(`For "${active.name}", draft a social post I can schedule (pick the best channel from the strategy above), and create it as a social_post card on this project.${HUMANIZE_VOICE}`)}
                style={{
                  fontSize: 11, fontFamily: font.body, color: colors.text,
                  background: 'transparent', border: `1px solid ${colors.border}`,
                  borderRadius: radius.md, padding: '5px 12px', cursor: 'pointer',
                }}
              >+ Draft a post with Henry</button>
            </div>
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
                No posts yet. Draft one with Henry above — he'll write it in the project's voice and file it here as a scheduled card.
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {posts.map((post) => (
                  <div key={post.id} style={{
                    background: colors.surface, border: `1px solid ${colors.border}`,
                    borderRadius: radius.md, padding: '12px 14px',
                  }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{post.title}</div>
                    {post.description && (
                      <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 4, lineHeight: 1.5 }}>{post.description}</div>
                    )}
                  </div>
                ))}
              </div>
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

// ── Shared async-state blocks ────────────────────────────────────────────────
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

interface GrowthAction {
  title: string;
  evidence: string;
  recommendation: string;
  category: string;
  impact: string;
  confidence: string;
}

interface GrowthActionsData {
  actions: GrowthAction[];
  generatedAt: string | null;
  reason: string | null;
  periodDays: number | null;
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
function GrowActions({ project, colors }: { project: Project; colors: ThemeColors }) {
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
  const [generating, setGenerating] = useState(false);
  const actionsGen = useRef(0);

  const loadActions = useCallback((id: string) => {
    const generation = ++actionsGen.current;
    setActionsState('loading');
    apiFetch<GrowthActionsData>(`/api/projects/${encodeURIComponent(id)}/growth-actions`)
      .then((d) => {
        if (generation !== actionsGen.current) return;
        setActions(d);
        setActionsState('ready');
      })
      .catch(() => {
        if (generation !== actionsGen.current) return;
        setActionsState('error');
      });
  }, []);

  // Regeneration is explicit. It spends a model call, and actions that
  // reshuffle on every render cannot be acted on.
  const generate = useCallback((id: string) => {
    setGenerating(true);
    apiFetch<GrowthActionsData>(
      `/api/projects/${encodeURIComponent(id)}/growth-actions/generate`,
      { method: 'POST' },
    )
      .then((d) => { setActions(d); setActionsState('ready'); })
      .catch(() => setActionsState('error'))
      .finally(() => setGenerating(false));
  }, []);

  useEffect(() => {
    loadInbox(project.id);
    loadActions(project.id);
    return () => { ++inboxGen.current; ++actionsGen.current; };
  }, [project.id, loadInbox, loadActions]);

  const CATEGORY_COLOR: Record<string, string> = {
    conversion: colors.cyan,
    retention: colors.success,
    churn: colors.danger,
    ux: colors.purple,
    acquisition: colors.warning ?? colors.cyan,
    measurement: colors.textMuted,
  };

  const hasActions = (actions?.actions?.length ?? 0) > 0;

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
          <button
            onClick={() => generate(project.id)}
            disabled={generating}
            style={{
              background: colors.surface, border: `1px solid ${colors.border}`,
              borderRadius: radius.md, padding: '5px 12px', cursor: generating ? 'default' : 'pointer',
              color: colors.text, fontFamily: font.body, fontSize: 12, opacity: generating ? 0.6 : 1,
            }}
          >{generating ? 'Reviewing…' : hasActions ? 'Review again' : 'Review my analytics'}</button>
        </div>

        {actionsState === 'loading' && (
          <div style={{ fontSize: 12, color: colors.textDim }}>Loading…</div>
        )}
        {actionsState === 'error' && (
          <div style={{ fontSize: 12, color: colors.danger }}>Couldn&rsquo;t load actions.</div>
        )}

        {/* An empty list ALWAYS explains itself — silence is indistinguishable
            from breakage, and this panel is allowed to have nothing to say. */}
        {actionsState === 'ready' && !hasActions && (
          <div style={{
            fontSize: 12, color: colors.textMuted, background: colors.bgDeeper,
            border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: 12,
          }}>
            {actions?.reason ?? 'No review yet — run one to see what your data suggests.'}
          </div>
        )}

        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {actions?.actions?.map((a, i) => (
            <div key={`${a.title}-${i}`} style={{
              background: colors.surface, border: `1px solid ${colors.border}`,
              borderRadius: radius.lg, padding: 14,
            }}>
              <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 6 }}>
                <span style={{
                  fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                  textTransform: 'uppercase', color: CATEGORY_COLOR[a.category] ?? colors.textDim,
                  border: `1px solid ${CATEGORY_COLOR[a.category] ?? colors.border}`,
                  borderRadius: 999, padding: '1px 7px', flexShrink: 0,
                }}>{a.category}</span>
                <span style={{ fontFamily: font.display, fontSize: 14, fontWeight: 600, color: colors.text }}>
                  {a.title}
                </span>
                <div style={{ flex: 1 }} />
                <span style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, flexShrink: 0 }}>
                  {a.impact} impact · {a.confidence} confidence
                </span>
              </div>
              {/* Evidence first: the number is what makes this checkable. */}
              <div style={{
                fontSize: 11, color: colors.textDim, fontFamily: font.mono,
                borderLeft: `2px solid ${colors.border}`, paddingLeft: 8, marginBottom: 6,
              }}>{a.evidence}</div>
              <div style={{ fontSize: 13, color: colors.textMuted, lineHeight: 1.5 }}>
                {a.recommendation}
              </div>
            </div>
          ))}
        </div>

        {hasActions && (
          <div style={{ fontSize: 10, color: colors.textDim, marginTop: 10 }}>
            Read from your own analytics by your agent, over the last{' '}
            {actions?.periodDays ?? 30} days. Each item cites the figure it came from — check it
            before acting.
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

      {/* Self-hosted analytics (#23) — the daemon is the collector. */}
      <FirstPartyAnalyticsPanel
        colors={colors}
        projectId={project.id}
        stats={fpStats}
        onRefresh={() => loadFpStats(project.id)}
      />

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
      {receiving && (setup.lastError || setup.lastDrainAt) && (
        <div style={{ fontSize: 10, color: setup.lastError ? colors.danger : colors.textDim, fontFamily: font.mono }}>
          {setup.lastError
            ? `Drain failing: ${setup.lastError}`
            : `Last drained ${new Date(setup.lastDrainAt!).toLocaleTimeString()}`}
        </div>
      )}

      {receiving && stats && (
        <>
          {/* Daily pageview bars — pure divs, no chart dependency. */}
          <div style={{ display: 'flex', alignItems: 'flex-end', gap: 2, height: 64 }}>
            {stats.byDay.map((d) => {
              const max = Math.max(1, ...stats.byDay.map((x) => x.pageviews));
              return (
                <div
                  key={d.day}
                  title={`${d.day}: ${d.pageviews} pageviews · ${d.visitors} visitors`}
                  style={{
                    flex: 1, minWidth: 3, height: `${Math.max(6, (d.pageviews / max) * 100)}%`,
                    background: `linear-gradient(180deg, ${colors.cyan}, ${colors.purple})`,
                    borderRadius: 2, opacity: 0.85,
                  }}
                />
              );
            })}
          </div>
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
        <LoadingState colors={colors} inline label="Ranking your growth moves…" />
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
