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

import { useEffect, useState } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { navigateToTool } from '../../lib/store';
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

export function GrowView() {
  const { colors, gradient } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [posts, setPosts] = useState<SocialCard[]>([]);
  const [lens, setLens] = useState<GrowLens>('strategy');

  useEffect(() => {
    apiFetch<Project[]>('/api/projects')
      .then((ps) => {
        const real = ps.filter((p) => p.status !== 'archived');
        setProjects(real);
        setActiveId((cur) => cur ?? real[0]?.id ?? null);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!activeId) return;
    // Content calendar = social_post cards on this project (reserved card
    // type already exists; empty until Henry/the user create them).
    apiFetch<SocialCard[]>(`/api/projects/${encodeURIComponent(activeId)}/cards?card_type=social_post`)
      .then(setPosts)
      .catch(() => setPosts([]));
  }, [activeId]);

  const active = projects.find((p) => p.id === activeId) ?? null;
  const send = (prompt: string) => {
    navigator.clipboard.writeText(prompt).catch(() => {});
    navigateToTool('chat');
  };

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      {/* Header + project switcher */}
      <div style={{ padding: '16px 24px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 16 }}>
        <div>
          <div style={{ fontFamily: font.display, fontSize: 16, fontWeight: 600, letterSpacing: '-0.01em' }}>Grow</div>
          <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2 }}>
            Take {active ? active.name : 'your project'} to market — Henry knows the project, so he drafts with real context.
          </div>
        </div>
        <div style={{ flex: 1 }} />
        <div style={{ display: 'flex', gap: 2, background: colors.bgDeeper, borderRadius: radius.md, padding: 2 }}>
          {(['strategy', 'calendar', 'analytics'] as GrowLens[]).map((l) => (
            <button
              key={l}
              onClick={() => setLens(l)}
              style={{
                fontSize: 12, fontFamily: font.body, textTransform: 'capitalize',
                padding: '5px 12px', borderRadius: radius.sm, cursor: 'pointer', border: 'none',
                background: lens === l ? colors.cyanSoft : 'transparent',
                color: lens === l ? colors.cyan : colors.textMuted,
              }}
            >{l}</button>
          ))}
        </div>
        <select
          value={activeId ?? ''}
          onChange={(e) => setActiveId(e.target.value)}
          style={{
            background: colors.bgDeeper, color: colors.text, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: '6px 10px', fontSize: 13, fontFamily: font.body,
          }}
        >
          {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </div>

      {active ? (
        <div style={{ flex: 1, overflowY: 'auto', padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 20 }}>
          {lens === 'analytics' && <GrowAnalytics project={active} posts={posts} colors={colors} />}
          {lens === 'strategy' && (
          <section>
            <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 12px' }}>Go-to-market strategy</h3>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 12 }}>
              {PILLARS.map((pillar) => (
                <div key={pillar.key} style={{
                  background: colors.surface, backdropFilter: 'blur(24px) saturate(140%)',
                  border: `1px solid ${colors.border}`, borderRadius: radius.lg, padding: 16,
                  display: 'flex', flexDirection: 'column', gap: 8, minHeight: 120,
                }}>
                  <div style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text }}>{pillar.label}</div>
                  <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5, flex: 1 }}>{pillar.hint}</div>
                  <button
                    onClick={() => send(pillar.prompt(active.name))}
                    style={{
                      alignSelf: 'flex-start', fontSize: 11, fontFamily: font.body,
                      color: colors.cyan, background: colors.cyanSoft,
                      border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
                      padding: '5px 10px', cursor: 'pointer',
                    }}
                  >Ask Henry ↗</button>
                </div>
              ))}
            </div>
          </section>
          )}

          {lens === 'calendar' && (
          <section>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, margin: '0 0 12px' }}>
              <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>Content calendar</h3>
              <span style={{ fontSize: 10, color: colors.textDim, background: 'rgba(255,255,255,0.06)', padding: '1px 6px', borderRadius: 8 }}>{posts.length}</span>
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
            {posts.length === 0 ? (
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
  colors: ReturnType<typeof useTheme>['colors'];
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
        fontSize: 11, color: colors.textDim, background: 'rgba(255,255,255,0.03)',
        border: `1px solid ${colors.border}`, borderRadius: 8, padding: '8px 12px', marginBottom: 4,
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
            borderRadius: 14, padding: 16,
          }}>
            <div style={{ fontFamily: font.display, fontSize: 26, fontWeight: 700, color: colors.text }}>{t.value}</div>
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
              <div style={{ flex: 1, height: 26, background: 'rgba(255,255,255,0.03)', borderRadius: 6, overflow: 'hidden', position: 'relative' }}>
                {f.source ? (
                  <div style={{
                    width: `${Math.max(6, ((f.value ?? 0) / maxV) * 100)}%`, height: '100%',
                    background: `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`,
                    borderRadius: 6, display: 'flex', alignItems: 'center', paddingLeft: 10,
                  }}>
                    <span style={{ fontFamily: font.mono, fontSize: 11, color: '#0A0E1A', fontWeight: 700 }}>{f.value}</span>
                  </div>
                ) : (
                  <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', paddingLeft: 10 }}>
                    <span style={{ fontSize: 10, color: colors.textDim, fontStyle: 'italic' }}>{f.hint}</span>
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}
