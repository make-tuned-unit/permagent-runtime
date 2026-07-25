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
    key: 'content',
    label: 'Content & launch',
    hint: 'The hub piece and the posts that orbit it.',
    prompt: (p) => `For "${p}", outline the launch content: one substantial hub piece (a guide/thread that establishes authority) and a week of social posts that link back to it. Draft the first post so I can schedule it.${HUMANIZE_VOICE}`,
  },
];

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

type GrowLens = 'strategy' | 'calendar' | 'analytics';
// Async lifecycle for data-backed sections — loading / ready / error are
// distinct so a fetch failure never masquerades as an empty result.
type LoadState = 'loading' | 'ready' | 'error';

const LENSES: GrowLens[] = ['strategy', 'calendar', 'analytics'];

export function GrowView() {
  const { colors, gradient, reduceMotion } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectsState, setProjectsState] = useState<LoadState>('loading');
  const [activeId, setActiveId] = useState<string | null>(null);
  const [posts, setPosts] = useState<SocialCard[]>([]);
  const [postsState, setPostsState] = useState<LoadState>('loading');
  const [lens, setLens] = useState<GrowLens>('strategy');
  const [ctx, setCtx] = useState<{ people: number; goals: number } | null>(null);
  const [focusLens, setFocusLens] = useState<GrowLens | null>(null);
  const postsRequestGeneration = useRef(0);
  const setActivePanel = useCommandCenter((st) => st.setActivePanel);
  const sendMessage = useCommandCenter((st) => st.sendMessage);
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

  useEffect(() => { loadProjects(); }, [loadProjects]);

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
    setActivePanel('chat');
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
          {lens === 'analytics' && <GrowAnalytics project={active} posts={posts} colors={colors} />}
          {lens === 'strategy' && (
          <section>
            <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 12px' }}>Go-to-market strategy</h3>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 12 }}>
              {PILLARS.map((pillar) => (
                <PillarCard
                  key={pillar.key}
                  label={pillar.label}
                  hint={pillar.hint}
                  projectName={active.name}
                  colors={colors}
                  reduceMotion={reduceMotion}
                  onSend={() => send(pillar.prompt(active.name))}
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
function PillarCard({
  label, hint, projectName, colors, reduceMotion, onSend,
}: {
  label: string;
  hint: string;
  projectName: string;
  colors: ThemeColors;
  reduceMotion: boolean;
  onSend: () => void;
}) {
  const [hover, setHover] = useState(false);
  const [focus, setFocus] = useState(false);
  const lit = hover || focus;
  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`Ask Henry about ${label} for ${projectName}`}
      onClick={onSend}
      onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSend(); } }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onFocus={() => setFocus(true)}
      onBlur={() => setFocus(false)}
      style={{
        background: colors.surface, backdropFilter: 'blur(24px) saturate(140%)',
        border: `1px solid ${lit ? colors.borderHi : colors.border}`, borderRadius: radius.lg, padding: 16,
        display: 'flex', flexDirection: 'column', gap: 8, minHeight: 120,
        cursor: 'pointer', outline: 'none',
        boxShadow: focus ? `0 0 0 2px ${colors.borderHi}` : 'none',
        transition: reduceMotion ? 'none' : 'border-color 150ms ease',
      }}
    >
      <div style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text }}>{label}</div>
      <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5, flex: 1 }}>{hint}</div>
      <span
        aria-hidden
        style={{
          alignSelf: 'flex-start', fontSize: 11, fontFamily: font.body,
          color: colors.cyan, background: colors.cyanSoft,
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
          padding: '5px 10px',
        }}
      >Ask Henry ↗</span>
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

function GrowAnalytics({
  project, posts, colors,
}: {
  project: Project;
  posts: SocialCard[];
  colors: ThemeColors;
}) {
  // The deterministic growth inbox — this week's ranked moves + wins, computed
  // server-side (NO LLM) from the project's real signals. Fetched on-read so
  // it's always fresh; its own load lifecycle keeps a fetch failure honest.
  const [inbox, setInbox] = useState<GrowthInboxData | null>(null);
  const [inboxState, setInboxState] = useState<LoadState>('loading');
  const inboxRequestGeneration = useRef(0);
  const connectionRequestGeneration = useRef(0);
  const statsRequestGeneration = useRef(0);

  const loadInbox = useCallback((id: string) => {
    const generation = ++inboxRequestGeneration.current;
    setInboxState('loading');
    apiFetch<GrowthInboxData>(`/api/projects/${encodeURIComponent(id)}/growth-inbox`)
      .then((d) => {
        if (generation !== inboxRequestGeneration.current) return;
        setInbox(d);
        setInboxState('ready');
      })
      .catch(() => {
        if (generation !== inboxRequestGeneration.current) return;
        setInbox(null);
        setInboxState('error');
      });
  }, []);

  useEffect(() => {
    loadInbox(project.id);
    return () => { ++inboxRequestGeneration.current; };
  }, [project.id, loadInbox]);

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

  const connected = conn?.connected ?? false;
  const providerLabel = conn?.provider ? PROVIDER_LABELS[conn.provider] : null;
  const visitors = connected ? stats?.visitors ?? null : null;
  const pageviews = connected ? stats?.pageviews ?? null : null;
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
      stage: 'Visitors',
      value: visitors,
      source: visitors != null,
      hint: liveHint(`Not exposed by ${providerLabel}`, 'Site sessions — connect analytics below'),
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
        ? `pageviews · ${providerLabel}`
        : liveHint(`not exposed by ${providerLabel}`, 'awaiting analytics'),
    },
    {
      label: 'CONVERSIONS',
      value: '—',
      sub: connected ? 'needs provider goals — follow-up' : 'awaiting analytics',
    },
  ];

  return (
    <>
      {/* Growth inbox — the headline: your 2-3 ranked moves this week + wins. */}
      <GrowthInboxSection
        colors={colors}
        state={inboxState}
        inbox={inbox}
        onRetry={() => loadInbox(project.id)}
      />

      <div style={{
        fontSize: 11, color: colors.textDim, background: colors.bgDeeper,
        border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '8px 12px', marginBottom: 4,
      }}>
        Growth analytics for <strong style={{ color: colors.text }}>{project.name}</strong>. Real
        signal shows now; visitors and pageviews go live when you connect your analytics account
        below (read-only) — nothing here is faked.
      </div>

      {/* Analytics connection — the settings surface for the live metrics */}
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

      {/* Metric tiles */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))', gap: 12 }}>
        {tiles.map((t) => (
          <div key={t.label} style={{
            background: colors.surface, border: `1px solid ${colors.border}`,
            borderRadius: radius.lg, padding: 16,
          }}>
            <div style={{ fontFamily: font.display, fontSize: 26, fontWeight: 700, color: colors.text, fontVariantNumeric: 'tabular-nums' }}>{t.value}</div>
            <div style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, letterSpacing: '0.08em', marginTop: 4 }}>{t.label}</div>
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
  const hasSignal = !!inbox && (inbox.signal.posts > 0 || inbox.signal.shipped > 0);
  const empty = !!inbox && inbox.moves.length === 0 && inbox.wins.length === 0;

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, margin: '0 0 12px', flexWrap: 'wrap' }}>
        <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>
          Your growth moves this week
        </h3>
        {hasSignal && inbox && (
          <span style={{ fontSize: 10, color: colors.textDim }}>
            from {inbox.signal.posts} {inbox.signal.posts === 1 ? 'post' : 'posts'} · {inbox.signal.shipped} shipped
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
          {inbox.moves.length > 0 ? (
            inbox.moves.map((m) => <MoveCard key={m.title} move={m} colors={colors} />)
          ) : (
            <div style={{
              border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '12px 14px',
              fontSize: 12, color: colors.textMuted, background: colors.surface,
            }}>
              You're on track — no urgent moves this week. Keep doing what's working below.
            </div>
          )}
          {inbox.wins.length > 0 && <WinsStrip wins={inbox.wins} colors={colors} />}
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
