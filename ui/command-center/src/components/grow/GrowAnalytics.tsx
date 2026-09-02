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
import { Button } from '../common/Button';
import type { Project } from '../projects/types';
import { FunnelPanel } from './FunnelPanel';
import { growCard, growLabel } from './growChrome';
import { CARD_PAD, CARD_R } from './growGeometry';
import type { SocialCard } from './calendarPosts';
import { FirstPartyAnalyticsPanel } from './FirstPartyAnalyticsPanel';
import { AnalyticsConnectionPanel } from './AnalyticsConnectionPanel';
import {
  PROVIDER_LABELS,
  type AnalyticsConnectionStatus,
  type AnalyticsStatsData,
  type FirstPartyStats,
  type LoadState,
} from './growTypes';

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
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))', gap: space.xl }}>
        {tiles.map((t) => (
          <div key={t.label} style={growCard(colors, { r: CARD_R, pad: CARD_PAD })}>
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
                  <div style={{
                    width: `${Math.max(6, ((f.value ?? 0) / maxV) * 100)}%`, height: '100%',
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
