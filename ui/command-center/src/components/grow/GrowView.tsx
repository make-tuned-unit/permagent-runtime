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

import { useCallback, useEffect, useState } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { Project } from '../projects/types';

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
    prompt: (p) => `Draft 3 one-line value propositions for "${p}" — the sharp promise that makes the target audience stop scrolling. Ground them in the project's actual capabilities.`,
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
    prompt: (p) => `For "${p}", outline the launch content: one substantial hub piece (a guide/thread that establishes authority) and a week of social posts that link back to it. Draft the first post so I can schedule it.`,
  },
];

interface SocialCard {
  id: string;
  title: string;
  description: string;
  // social_post cards carry scheduling in metadata; tolerant read.
  metadata_json?: Record<string, unknown> | null;
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
  const setActivePanel = useCommandCenter((st) => st.setActivePanel);
  const sendMessage = useCommandCenter((st) => st.sendMessage);
  const openGrowForProject = useCommandCenter((st) => st.openGrowForProject);

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
  const loadPosts = useCallback((id: string) => {
    setPostsState('loading');
    apiFetch<SocialCard[]>(`/api/projects/${encodeURIComponent(id)}/cards?card_type=social_post`)
      .then((p) => { setPosts(p); setPostsState('ready'); })
      .catch(() => { setPosts([]); setPostsState('error'); });
  }, []);

  useEffect(() => {
    if (!activeId) return;
    loadPosts(activeId);
  }, [activeId, loadPosts]);

  const active = projects.find((p) => p.id === activeId) ?? null;

  // Honor a cross-tab deep link (Projects → Grow this project).
  useEffect(() => {
    if (openGrowForProject) {
      setActiveId(openGrowForProject);
    }
  }, [openGrowForProject]);

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
            {ctx && (
              <span style={{ color: colors.textDim }}>{ctx.goals} shipped · {ctx.people} {ctx.people === 1 ? 'person' : 'people'}</span>
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
                onClick={() => send(`For "${active.name}", draft a social post I can schedule (pick the best channel from the strategy above), and create it as a social_post card on this project.`)}
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

// ── Analytics lens — PostHog-style growth funnel + metric tiles ──────────────
//
// v1 shows REAL, derivable signal (content published, goals shipped) and the
// funnel scaffold the growth metrics plug into. Live product analytics
// (visitors, signups, retention) require an events pipeline — the Grow epic's
// analytics slice wires a self-hosted PostHog / native event bridge here. The
// scaffold is honest: metrics with no source yet say so rather than faking a
// number.

function GrowAnalytics({
  project, posts, colors,
}: {
  project: Project;
  posts: SocialCard[];
  colors: ThemeColors;
}) {
  // The classic growth funnel (research: awareness → interest → action →
  // retention). Awareness/reach comes from published content; the deeper
  // stages await the analytics pipeline.
  const funnel = [
    { stage: 'Content live', value: posts.length, source: true, hint: 'Published social posts' },
    { stage: 'Reach', value: null, source: false, hint: 'Impressions — connect a channel' },
    { stage: 'Visitors', value: null, source: false, hint: 'Site sessions — connect analytics' },
    { stage: 'Signups', value: null, source: false, hint: 'Conversions — connect analytics' },
    { stage: 'Retained', value: null, source: false, hint: 'Return users — connect analytics' },
  ];
  const maxV = Math.max(1, ...funnel.map((f) => f.value ?? 0));

  const tiles = [
    { label: 'POSTS PUBLISHED', value: String(posts.length), sub: 'this project' },
    { label: 'ACTIVE CHANNELS', value: '0', sub: 'connect in the epic' },
    { label: 'REACH (30D)', value: '—', sub: 'awaiting analytics' },
    { label: 'CONVERSIONS', value: '—', sub: 'awaiting analytics' },
  ];

  return (
    <>
      <div style={{
        fontSize: 11, color: colors.textDim, background: colors.bgDeeper,
        border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: '8px 12px', marginBottom: 4,
      }}>
        Growth analytics for <strong style={{ color: colors.text }}>{project.name}</strong>. Real
        signal shows now; live product metrics (visitors, signups, retention) light up when the
        analytics pipeline is connected — nothing here is faked.
      </div>

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
                width: 40, textAlign: 'right', flexShrink: 0, fontFamily: font.mono, fontSize: 12,
                color: colors.text, fontVariantNumeric: 'tabular-nums',
              }}>{f.source ? f.value : ''}</div>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}
