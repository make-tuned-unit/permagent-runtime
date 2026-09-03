/**
 * The self-hosted analytics panel (#23): enable → copy the install brief → the
 * relay drains into this daemon → the panel flips live and shows the figures.
 *
 * Split out of GrowView.tsx (R9), unchanged — including the Refresh control
 * that #1167 made pull from the relay before reloading (`checkNow`), which is
 * load-bearing now that a site is drained once a day.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { drainFreshness } from './analyticsFormat';
import { ErrorState } from './GrowStateBlocks';
import { FIELD_CLASS, growCard, growField, growLabel } from './growChrome';
import { CARD_INNER_R, CARD_PAD, CARD_R } from './growGeometry';
import type { FirstPartySetup, FirstPartyStats, LoadState, VerifyResponse } from './growTypes';

// ── First-party analytics panel (#23) ────────────────────────────────────────
// The self-hosted path: enable → copy the agent prompt (a coding agent adds
// the snippet to the site) → come back and the panel flips live on the first
// beacon. No third-party dependency; the daemon is the collector.

export function FirstPartyAnalyticsPanel({
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
  // reports back after installing the relay. The same call runs the first pass
  // immediately, so setup does not sit on "not receiving" until tomorrow's
  // scheduled drain.
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

  // Pull from the relay right now. The daemon drains each site once a day and
  // nothing faster — anything faster stops the site's own database ever scaling
  // to zero, which is what the old two-minute poller cost ($91 of Neon compute
  // for August 2026). So the daily schedule is the floor, and this is how the
  // user gets today's numbers when they actually want them.
  //
  // POST to the drain route with NO drainUrl: absent means "leave the target
  // alone, just drain". Sending an empty string would CLEAR it.
  const [drainingNow, setDrainingNow] = useState(false);
  const checkNow = useCallback(async () => {
    setDrainingNow(true);
    try {
      const s = await apiFetch<FirstPartySetup>(
        `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/drain`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({}),
        },
      );
      // A failed pass comes back 200 with lastError set (the panel's
      // convention), so this render path covers both outcomes.
      setSetup(s);
      onRefresh();
    } catch {
      // Transport-level failure only. Re-read rather than inventing a state:
      // the pass may well have run and persisted before the response was lost.
      onRefresh();
    } finally {
      setDrainingNow(false);
    }
  }, [projectId, onRefresh]);

  // Rotate the drain secret. It ships inside the install brief, so it lands in
  // the coding agent's transcript and tool logs — a credential that has passed
  // through a third-party model's context should be replaceable without
  // rebuilding the install. Rotating 401s the deployed site until the new value
  // is set on the app service and it redeploys.
  const [rotating, setRotating] = useState(false);
  const [confirmingRotate, setConfirmingRotate] = useState(false);
  // The one Tier-3 action on this panel, so the one that earns a modal: it is
  // unrecoverable (the old key stops working the instant the new one is minted)
  // and it breaks a deployed site until someone redeploys. It used to be an OS
  // dialog — right tier, wrong widget. A failed rotation now says so on the
  // dialog and leaves the panel intact, rather than replacing the whole panel
  // with a load-error state that says nothing about what was attempted.
  const rotateSecret = useCallback(async () => {
    setRotating(true);
    try {
      const s = await apiFetch<FirstPartySetup>(
        `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/rotate`,
        { method: 'POST' },
      );
      setSetup(s);
      onRefresh();
      setConfirmingRotate(false);
    } finally {
      setRotating(false);
    }
  }, [projectId, onRefresh]);

  const copy = useCallback((kind: 'snippet' | 'prompt', text: string | null | undefined) => {
    if (!text) return;
    navigator.clipboard?.writeText(text).then(() => {
      setCopied(kind);
      setTimeout(() => setCopied((c) => (c === kind ? null : c)), 1600);
    });
  }, []);

  const shell: React.CSSProperties = {
    ...growCard(colors, { r: CARD_R, pad: CARD_PAD }),
    display: 'flex', flexDirection: 'column', gap: space.lg,
  };
  // Through `--pa-btn-*`, never inline `background`/`color`: an inline
  // declaration outranks `.pa-btn:hover` and would kill the state this is
  // being migrated for. The per-button `opacity` dimming is gone with it —
  // `.pa-btn:disabled` and `[data-pending]` say that for themselves now.
  const buttonStyle: React.CSSProperties = {
    '--pa-btn-bg': colors.bgDeeper,
    '--pa-btn-fg': colors.text,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-pad': `${space.sm}px ${space.xl}px`,
    '--pa-btn-radius': `${radius.md}px`,
    fontSize: textSize.caption,
  } as React.CSSProperties;

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
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl }}>
          <div>
            <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text }}>Self-hosted analytics</div>
            <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs / 2 }}>
              Your daemon collects pageviews directly — no third-party account, your data stays here.
            </div>
          </div>
          <Button
            colors={colors}
            style={buttonStyle}
            disabled={setupState === 'loading' || saving}
            pending={saving}
            onClick={() => enable()}
          >{saving ? 'Enabling…' : 'Enable'}</Button>
        </div>
      </div>
    );
  }

  const receiving = !!stats?.receiving;

  return (
    <div style={shell}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl }}>
        <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text }}>
          Self-hosted analytics
          {receiving && (
            <span style={{ marginLeft: space.md, fontSize: textSize.micro, color: colors.cyan, fontFamily: font.mono }}>
              ● live{stats && stats.eventsLast5m > 0 ? ` · ${stats.eventsLast5m} events / 5m` : ''}
            </span>
          )}
        </div>
        {/* One refresh control, and it does the thing the word promises. It
            used to re-read only THIS hub's copy, which was honest while the
            poller ran every two minutes; now that a site is polled once a day
            (to let its database scale to zero), a button that reloads a
            day-old local copy would show a quiet day and be believed. So
            Refresh pulls from the relay first, then reloads. */}
        <Tooltip content="Pull from the site now, then reload. The daily schedule lets the site's database sleep; this is how you get the last few hours on demand." placement="bottom">
          <Button
            colors={colors}
            style={buttonStyle}
            disabled={drainingNow}
            pending={drainingNow}
            onClick={checkNow}
          >{drainingNow ? 'Checking…' : 'Refresh'}</Button>
        </Tooltip>
      </div>

      {!receiving && (
        <>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
            <b style={{ color: colors.text }}>Step 1.</b> Copy the install brief below and give it to
            a coding agent inside this project's repo. It builds the relay: visitors beacon
            same-origin to your own app, which buffers events in your own database.
            <br />
            <b style={{ color: colors.text }}>Step 2.</b> Paste the drain URL it reports back. This
            Mac then pulls events outbound every couple of minutes — nothing here is ever exposed to
            the internet, and events survive while it sleeps.
          </div>
          <div style={{ display: 'flex', gap: space.md }}>
            {/* Each already says "Copied ✓" for itself, so the primitive's tick
                would be the same confirmation twice. */}
            <Button colors={colors} style={buttonStyle} flashSuccess={false} onClick={() => copy('prompt', setup.agentPrompt)}>
              {copied === 'prompt' ? 'Copied ✓' : 'Copy install brief'}
            </Button>
            <Button colors={colors} style={buttonStyle} flashSuccess={false} onClick={() => copy('snippet', setup.snippet)}>
              {copied === 'snippet' ? 'Copied ✓' : 'Copy snippet only'}
            </Button>
            <Tooltip content="Mint a new drain key — the old one stops working immediately">
              <Button
                colors={colors}
                style={buttonStyle}
                disabled={rotating}
                pending={rotating}
                onClick={() => setConfirmingRotate(true)}
              >{rotating ? 'Rotating…' : 'Rotate key'}</Button>
            </Tooltip>
          </div>
          <div style={{ display: 'flex', gap: space.md, alignItems: 'center', flexWrap: 'wrap' }}>
            <input
              value={ingestBase}
              onChange={(e) => setIngestBase(e.target.value)}
              placeholder="https://yoursite.com/api/permagent-analytics/drain"
              className={FIELD_CLASS}
              style={{ ...growField(colors, { mono: true }), flex: '1 1 260px', borderRadius: CARD_INNER_R }}
            />
            <Button
              colors={colors}
              style={buttonStyle}
              disabled={saving}
              pending={saving}
              onClick={() => setDrain(ingestBase.trim())}
            >{saving ? 'Saving…' : 'Start ingesting'}</Button>
            <Tooltip content="Fetch the deployed site and assert the install actually works">
              <Button
                colors={colors}
                style={buttonStyle}
                disabled={verifying}
                pending={verifying}
                onClick={() => {
                // Derive the origin from the drain URL the agent reported.
                const url = ingestBase.trim() || setup.drainUrl || '';
                try { runVerify(new URL(url).origin); } catch { /* not a URL yet */ }
              }}
              >{verifying ? 'Verifying…' : 'Verify install'}</Button>
            </Tooltip>
          </div>
          {verifyResult && (
            <div style={{
              fontSize: textSize.micro, fontFamily: font.mono, whiteSpace: 'pre-wrap',
              background: colors.bgDeeper, borderRadius: CARD_INNER_R, padding: space.lg,
              border: `1px solid ${verifyResult.verified ? colors.border : colors.danger}`,
              color: verifyResult.verified ? colors.textMuted : colors.text,
              maxHeight: 260, overflowY: 'auto',
            }}>{verifyResult.summary}</div>
          )}
          {setup.lastError && (
            <div style={{ fontSize: textSize.micro, color: colors.danger, fontFamily: font.mono }}>
              Last drain failed: {setup.lastError}
            </div>
          )}
          <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
            {setup.drainUrl
              ? `Draining from ${setup.drainUrl}${setup.lastDrainAt ? ` · last checked ${new Date(setup.lastDrainAt).toLocaleTimeString()}` : ' · waiting for the first pass…'}`
              : 'Waiting for a drain URL.'}
          </div>
        </>
      )}
      {receiving && (() => {
        if (setup.lastError) {
          return (
            <div style={{ fontSize: textSize.micro, color: colors.danger, fontFamily: font.mono }}>
              Drain failing: {setup.lastError}
            </div>
          );
        }
        // Drain health, subtly, but honest: freshness comes from stats (it
        // refreshes with the panel; setup only loads once), and a drain that
        // has missed a whole daily cycle — or a relay holding events we have
        // not pulled — gets the warning tint. A stale figure must never read
        // as a quiet day (the botsExcluded rule). The threshold sits past a
        // day because the poller only runs daily (DRAIN_STALE_MS says why), so
        // "drained 6h ago" is the schedule working, not a fault — Refresh
        // above is how you close the gap on demand.
        const fresh = drainFreshness(stats?.lastDrainAt ?? setup.lastDrainAt);
        if (!fresh) return null;
        const lag = stats?.drainLagEvents ?? 0;
        return (
          <div style={{
            fontSize: textSize.micro, fontFamily: font.mono,
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
                  <Tooltip content={d.pageviews > 0
                      ? `${d.day}: ${d.pageviews} pageviews · ${d.visitors} devices`
                      : `${d.day}: no traffic`}>
                    <span tabIndex={0} style={{ outline: 'none' }}>
                      <div
                        key={d.day}
                        style={{
                          flex: 1, minWidth: 2,
                          // A zero day is a hairline, not a bar — visibly empty
                          // rather than a misleading minimum-height stub.
                          height: d.pageviews > 0 ? `${Math.max(8, (d.pageviews / max) * 100)}%` : '1px',
                          // Solid, like every other bar on this screen. A vertical
                          // gradient down a 2px column encodes nothing and is the
                          // "rainbow tinting" the anti-slop list names.
                          background: d.pageviews > 0 ? colors.cyan : colors.border,
                          // No radius. These bars are 2px wide at thirty days in a
                          // 320px panel: a 1px rounding is invisible at that scale
                          // and is not a step on the scale either — decoration
                          // standing in for nothing (D13).
                          opacity: d.pageviews > 0 ? 0.9 : 1,
                        }}
     />
                    </span>
                  </Tooltip>
                ))}
              </div>
            );
          })()}
          {/* Headline figures, each labelled for what it actually is. */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(110px, 1fr))', gap: space.lg }}>
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
                borderRadius: CARD_INNER_R, padding: space.lg,
              }}>
                <div style={{ fontFamily: font.display, fontSize: textSize.title, fontWeight: 700, color: colors.text, fontVariantNumeric: 'tabular-nums' }}>{value}</div>
                <div style={{ ...growLabel(colors), marginTop: space.xs / 2 }}>{label}</div>
                <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: 1 }}>{sub}</div>
              </div>
            ))}
          </div>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono }}>
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
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginBottom: space.md }}>
              <span style={growLabel(colors)}>AEO</span>
              {' '}{(stats.aeoVisits ?? 0).toLocaleString()} answer-engine visit{(stats.aeoVisits === 1) ? '' : 's'}
            </div>
          )}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: space.xl }}>
            {([
              ['Top pages', stats.topPages],
              ['Sources', stats.topSources],
              ['Referrers', stats.topReferrers],
              ['Campaigns', stats.topCampaigns],
              ['Entry pages', stats.topEntryPages],
              ['Events', stats.topEvents],
            ] as const).map(([label, rows]) => (
              <div key={label}>
                <div style={{ ...growLabel(colors), marginBottom: space.xs }}>{label}</div>
                {rows.length === 0 && <div style={{ fontSize: textSize.micro, color: colors.textDim }}>—</div>}
                {rows.slice(0, 5).map((r) => (
                  <div key={r.name} style={{ display: 'flex', justifyContent: 'space-between', gap: space.md, fontSize: textSize.micro, color: colors.textMuted, padding: `${space.xs / 2}px 0` }}>
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{r.name}</span>
                    <span style={{ fontFamily: font.mono, color: colors.text }}>{r.count.toLocaleString()}</span>
                  </div>
                ))}
              </div>
            ))}
          </div>
        </>
      )}

      {confirmingRotate && (
        <ConfirmDialog
          title="Mint a new drain key?"
          consequence={
            'The current key stops working the moment the new one is minted. '
            + 'Ingestion fails with 401 until you set the new value on your app service '
            + 'and it redeploys — copy the fresh install brief afterwards.'
          }
          confirmLabel="Mint a new key"
          failureLabel="Couldn't mint a new key"
          onConfirm={rotateSecret}
          onCancel={() => setConfirmingRotate(false)}
        />
      )}
    </div>
  );
}
