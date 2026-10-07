/**
 * The Analytics lens — the funnel, the metric tiles, and the two panels that
 * connect a source.
 *
 * Split out of GrowView.tsx (R9), unchanged.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { AnimatedNumber } from '../common/AnimatedNumber';
import { Button } from '../common/Button';
import { Chip } from '../common/Chip';
import type { Project } from '../projects/types';
import { FunnelPanel } from './FunnelPanel';
import { growCard, growLabel } from './growChrome';
import { CARD_PAD, CARD_R, magnitudeBarPct } from './growGeometry';
import { readPostMeta, type SocialCard } from './calendarPosts';
import { FirstPartyAnalyticsPanel } from './FirstPartyAnalyticsPanel';
import { AnalyticsConnectionPanel } from './AnalyticsConnectionPanel';
import { NETWORKS, useConnectedChannels } from './PublisherSettings';
import {
  PROVIDER_LABELS,
  type AnalyticsConnectionStatus,
  type AnalyticsStatsData,
  type FirstPartyStats,
  type LoadState,
} from './growTypes';

/**
 * Period-over-period comparison for one headline figure (dagK grow-deltas,
 * 2026-09-26). `direction: 'unknown'` — never a fabricated 'flat' — is what a
 * missing `previous` (no prior period, or nothing live yet) must produce; a
 * `previous` of exactly 0 has no percentage form (division by zero), so `pct`
 * stays null there even though the direction ('up', since anything beats
 * zero) is still known.
 */
export interface DeltaResult {
  direction: 'up' | 'down' | 'flat' | 'unknown';
  pct: number | null;
}

export function computeDelta(current: number | null, previous: number | null | undefined): DeltaResult {
  if (current == null || previous == null) return { direction: 'unknown', pct: null };
  if (current === previous) return { direction: 'flat', pct: 0 };
  const direction = current > previous ? 'up' : 'down';
  if (previous === 0) return { direction, pct: null };
  return { direction, pct: ((current - previous) / previous) * 100 };
}

/** The delta chip's label and tone — one function, so a tile never disagrees
 *  with its own arithmetic (R5: one state vocabulary per concept). */
function deltaChipText(delta: DeltaResult): string {
  if (delta.direction === 'unknown') return 'no prior period';
  if (delta.direction === 'flat') return 'flat vs prior period';
  const arrow = delta.direction === 'up' ? '▲' : '▼';
  if (delta.pct == null) return `${arrow} new`;
  const rounded = Math.round(Math.abs(delta.pct));
  return `${arrow} ${rounded}%`;
}

function deltaChipTone(delta: DeltaResult): 'success' | 'danger' | 'neutral' {
  if (delta.direction === 'up') return 'success';
  if (delta.direction === 'down') return 'danger';
  return 'neutral';
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

export function GrowAnalytics({
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
  // Only posts that actually went out. Drafts and scheduled posts are on the
  // calendar, not in front of anyone, so "published" must not count them (the
  // same rule the daemon's growth inbox applies).
  const published = posts.filter((p) => readPostMeta(p).status === 'posted').length;
  const funnel = [
    { stage: 'Content live', value: published, source: true, hint: 'Published social posts' },
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
      hint: connected ? `Set up a signup goal in ${providerLabel} to count these` : 'Conversions — connect analytics below',
    },
    {
      stage: 'Retained',
      value: null as number | null,
      source: false,
      hint: connected ? `Set up a return-visit goal in ${providerLabel} to count these` : 'Return users — connect analytics below',
    },
  ];
  const maxV = Math.max(1, ...funnel.map((f) => f.value ?? 0));

  // The channel count is READ, not assumed. The tile was a hard-coded "0" with
  // "connect in the epic" under it — a number that was wrong the moment a
  // channel was connected, and a sub-line in the team's planning vocabulary
  // rather than the user's. It now counts the same bindings the Calendar's
  // "Connect Instagram / LinkedIn / X" row writes, and says where that row is.
  const connectedChannels = useConnectedChannels(project.id);
  const channelLabels = NETWORKS.filter((n) => connectedChannels?.has(n.id)).map((n) => n.label);

  // UX_BENCHMARK_RESEARCH §8 (Typefully: "narrow but actionable"): lead with
  // the metric that answers "is this working", push setup/config metrics
  // below it. Reach is the one number here backed by a live source; the other
  // three either restate what the user already knows (posts published) or are
  // permanently "—" until a provider goal or channel is configured.
  //
  // Period-over-period delta (dagK grow-deltas, 2026-09-26): both stats
  // routes now return `previousPeriod` — the equivalent preceding window,
  // computed server-side from the same tables/provider, `null` when there is
  // genuinely no prior period rather than a faked flat 0%. Read from whichever
  // source is live, same as `pageviews` itself just above.
  const previousPageviews = fpLive
    ? (fpStats?.previousPeriod?.pageviews ?? null)
    : connected
      ? (stats?.previousPeriod?.pageviews ?? null)
      : null;
  const reachDelta = computeDelta(pageviews, previousPageviews);
  const periodDays = fpLive ? (fpStats?.period.days ?? null) : connected ? (stats?.periodDays ?? null) : null;
  const leadTile = {
    label: 'REACH (30D)',
    value: pageviews,
    sub: pageviews != null
      ? `pageviews · ${fpLive ? 'self-hosted' : providerLabel}`
      : liveHint(`not exposed by ${providerLabel}`, 'awaiting analytics'),
  };
  const tiles = [
    { label: 'POSTS PUBLISHED', value: String(published), sub: 'this project' },
    {
      label: 'ACTIVE CHANNELS',
      value: connectedChannels ? String(channelLabels.length) : '—',
      sub: !connectedChannels
        ? 'checking connected accounts…'
        : channelLabels.length > 0
          ? channelLabels.join(', ')
          : 'Connect Instagram, LinkedIn or X in Calendar',
    },
    {
      label: 'CONVERSIONS',
      value: '—',
      sub: connected ? `set up a goal in ${providerLabel} to count these` : 'awaiting analytics',
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
        fontSize: textSize.micro, color: colors.textDim, background: colors.bgDeeper,
        border: `1px solid ${colors.border}`, borderRadius: CARD_R, padding: `${space.md}px ${space.xl}px`,
        marginBottom: space.xs,
      }}>
        Analytics for <strong style={{ color: colors.text }}>{project.name}</strong>, collected by
        Permagent onto your own infrastructure — nothing here is faked or shared. What to DO about
        it lives in <strong style={{ color: colors.text }}>Actions</strong>.
      </div>

      {/* Self-hosted analytics (#23) — the daemon is the collector.
          KEYED ON THE PROJECT. `loadSetup` refetches on a projectId change and
          guards stale responses with a generation counter, but the panel's
          other state does not reset — so after verifying Acme and switching
          to Beacon, the previous project's PASS was still on screen, telling
          the user analytics was installed here when it was not (reported
          2026-08-04). In a surface where every failure is silent, a false
          "verified" is the worst thing this panel can say. Keying remounts it,
          which clears the whole class rather than the one field that leaked.

          `fp:` prefix: this panel and `AnalyticsConnectionPanel` below are
          both keyed on the same `project.id` and sit as siblings in this same
          fragment (one hidden behind `providersOpen`, but both mounted in the
          same React children list), so two bare `key={project.id}` collided —
          "two children with the same key" in dev, and undefined behaviour
          per React's own docs. The prefix keeps each panel's remount-on-
          project-switch behaviour (still id-scoped, still changes when the
          project changes) while making the two keys distinct from each
          other. See `analyticsPanelScope.test.ts` for the pinned contract. */}
      <FirstPartyAnalyticsPanel
        key={`fp:${project.id}`}
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
        <Button
          colors={colors}
          variant="bare"
          onClick={() => setShowProviders(true)}
          style={{
            '--pa-btn-fg': colors.textDim,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '2px 0',
            alignSelf: 'flex-start',
            fontFamily: font.body,
            textDecoration: 'underline',
          } as CSSProperties}
        >
          Already use Plausible, Fathom or GA? Connect it read-only instead
        </Button>
      ) : (
        <AnalyticsConnectionPanel
          key={`conn:${project.id}`}
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

      {/* Metric tiles — Reach leads, on its own row and at a larger size, so
          the one number backed by a live source is what the eye lands on
          first (UX_BENCHMARK_RESEARCH §8). The setup/config tiles follow at
          the previous size. */}
      <div data-testid="metric-tile-lead" style={{ ...growCard(colors, { r: CARD_R, pad: CARD_PAD }), marginBottom: space.xl }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: space.md, flexWrap: 'wrap' }}>
          <div style={{ fontFamily: font.display, fontSize: textSize.heading, fontWeight: 700, color: colors.text, fontVariantNumeric: 'tabular-nums' }}>
            {leadTile.value != null ? <AnimatedNumber value={leadTile.value} /> : '—'}
          </div>
          {/* R8/R6: the one live number gets a delta, honestly — "no prior
              period" muted rather than a fabricated flat 0% (dagK grow-deltas,
              2026-09-26). */}
          {leadTile.value != null && (
            <Chip
              kind="count"
              tone={deltaChipTone(reachDelta)}
              quiet={reachDelta.direction === 'unknown'}
              data-testid="reach-delta"
              title={periodDays ? `vs the prior ${periodDays}-day period` : undefined}
            >
              {deltaChipText(reachDelta)}
            </Chip>
          )}
        </div>
        <div style={{ ...growLabel(colors), marginTop: space.xs }}>{leadTile.label}</div>
        <div style={{ fontSize: textSize.caption, color: colors.textDim, marginTop: space.xs / 2 }}>{leadTile.sub}</div>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))', gap: space.xl }}>
        {tiles.map((t) => (
          <div key={t.label} data-testid="metric-tile" style={growCard(colors, { r: CARD_R, pad: CARD_PAD })}>
            <div style={{ fontFamily: font.display, fontSize: textSize.title, fontWeight: 700, color: colors.text, fontVariantNumeric: 'tabular-nums' }}>{t.value}</div>
            <div style={{ ...growLabel(colors), marginTop: space.xs }}>{t.label}</div>
            <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs / 2 }}>{t.sub}</div>
          </div>
        ))}
      </div>

      {/* Funnel */}
      <section style={{ marginTop: space.md }}>
        <h3 style={{ ...growLabel(colors), margin: `0 0 ${space.xl}px` }}>Growth funnel</h3>
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          {funnel.map((f) => (
            <div key={f.stage} style={{ display: 'flex', alignItems: 'center', gap: space.xl }}>
              <div style={{ width: 96, fontSize: textSize.caption, color: colors.textMuted, textAlign: 'right', flexShrink: 0 }}>{f.stage}</div>
              <div style={{ flex: 1, height: 26, background: colors.bgDeeper, borderRadius: radius.sm, overflow: 'hidden', position: 'relative' }}>
                {f.source ? (
                  // One tint, not two. The bar was a cyan-to-purple gradient,
                  // which reads as a second dimension the number does not have —
                  // a magnitude bar encodes length and nothing else. D8: one
                  // accent per view, and it belongs to the action.
                  // A zero is an EMPTY track. The fill had a 6% floor, so
                  // "Content live 0" drew a cyan sliver that read as "a little".
                  <div data-testid={`funnel-fill-${f.stage}`} style={{
                    width: `${magnitudeBarPct(f.value ?? 0, maxV)}%`, height: '100%',
                    background: colors.cyan,
                    borderRadius: radius.sm,
                  }} />
                ) : (
                  <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', paddingLeft: space.lg }}>
                    <span style={{ fontSize: textSize.micro, color: colors.textDim, fontStyle: 'italic' }}>{f.hint}</span>
                  </div>
                )}
              </div>
              <div style={{
                minWidth: 40, textAlign: 'right', flexShrink: 0, fontFamily: font.mono, fontSize: textSize.caption,
                color: colors.text, fontVariantNumeric: 'tabular-nums',
              }}>{f.source ? f.value?.toLocaleString() : ''}</div>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}
