/**
 * The third-party analytics connection: the status row, and the connect/edit
 * form behind it.
 *
 * Split out of GrowView.tsx (R9), unchanged. Every control hits a real endpoint
 * (save / test / stats / disconnect) — no dead UI. The API key is write-only:
 * sent on save, never read back.
 */

import { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { Button } from '../common/Button';
import { growAccent, growChip } from './growStyles';
import { FIELD_CLASS, growCard, growField, growLabel } from './growChrome';
import { CARD_INNER_R, CARD_PAD, CARD_R } from './growGeometry';
import { ErrorState, LoadingState } from './GrowStateBlocks';
import {
  PROVIDER_LABELS,
  type AnalyticsConnectionStatus,
  type AnalyticsProviderId,
  type AnalyticsStatsData,
  type AnalyticsTestResult,
  type LoadState,
} from './growTypes';

// ── Analytics connection panel ───────────────────────────────────────────────
// The "connect analytics" settings surface on the analytics lens. Every
// control hits a real endpoint (save / test / stats / disconnect) — no dead
// UI. The API key is write-only: sent on save, never read back.

export function AnalyticsConnectionPanel({
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

  const btnStyle: CSSProperties = growChip();

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
        border: `1px dashed ${colors.border}`, borderRadius: CARD_R, padding: CARD_PAD,
        display: 'flex', alignItems: 'center', gap: space.xxl, flexWrap: 'wrap',
      }}>
        <div style={{ flex: 1, minWidth: 220 }}>
          <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text }}>Connect analytics</div>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs, lineHeight: 1.5 }}>
            Point the funnel at your existing Plausible or GoatCounter account — a read-only stats
            fetch, your data stays where it is.
          </div>
        </div>
        <Button
          colors={colors}
          onClick={() => setShowForm(true)}
          style={{ ...growAccent(colors, `${space.sm}px ${space.xxl}px`), fontSize: textSize.caption }}
        >Connect analytics</Button>
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
      ...growCard(colors, { r: CARD_R, pad: CARD_PAD }),
      display: 'flex', flexDirection: 'column', gap: space.md,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.lg, flexWrap: 'wrap' }}>
        <span aria-hidden style={{
          width: space.md, height: space.md, borderRadius: '50%', flexShrink: 0,
          background: statsFailed ? colors.warning : colors.success,
        }} />
        <span style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.text }}>{providerLabel}</span>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.mono }}>
          {conn.baseUrl}{conn.siteId ? ` · ${conn.siteId}` : ''}
        </span>
        <div style={{ flex: 1 }} />
        <Button colors={colors} onClick={onRefreshStats} disabled={statsState === 'loading'} pending={statsState === 'loading'} style={btnStyle}>Refresh</Button>
        <Button colors={colors} onClick={runTest} disabled={testing} pending={testing} style={btnStyle}>{testing ? 'Testing…' : 'Test connection'}</Button>
        <Button colors={colors} onClick={() => { setTestResult(null); setShowForm(true); }} style={btnStyle}>Edit</Button>
        <Button
          colors={colors}
          onClick={disconnect}
          disabled={disconnecting}
          pending={disconnecting}
          style={{ ...btnStyle, '--pa-btn-fg': colors.warning, '--pa-btn-border-hover': colors.warning } as CSSProperties}
        >{disconnecting ? 'Disconnecting…' : 'Disconnect'}</Button>
      </div>
      {statsLine && (
        <div style={{ fontSize: textSize.micro, color: statsFailed ? colors.warning : colors.textMuted }}>{statsLine}</div>
      )}
      {testResult && (
        <div style={{ fontSize: textSize.micro, color: testResult.ok ? colors.success : colors.warning }}>{testResult.message}</div>
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
    ...growField(colors), borderRadius: CARD_INNER_R, width: '100%',
  };
  const labelStyle: CSSProperties = {
    ...growLabel(colors), marginBottom: space.xs, display: 'block',
  };

  return (
    <div style={{
      ...growCard(colors, { r: CARD_R, pad: CARD_PAD }),
      display: 'flex', flexDirection: 'column', gap: space.xl,
    }}>
      <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text }}>
        {conn?.connected ? 'Edit analytics connection' : 'Connect analytics'}
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))', gap: space.xl }}>
        <label>
          <span style={labelStyle}>Provider</span>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value as AnalyticsProviderId)}
            className={FIELD_CLASS}
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
            className={FIELD_CLASS}
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
              className={FIELD_CLASS}
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
            className={FIELD_CLASS}
            style={fieldStyle}
          />
        </label>
      </div>
      <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
        {provider === 'goatcounter'
          ? 'GoatCounter: your site lives in the URL (no separate site id). Create an API token under Settings → API in your GoatCounter dashboard.'
          : 'Plausible: the site id is the domain as it appears in Plausible. Create a Stats API key under Settings → API keys.'}
        {' '}Read-only — this never writes to your analytics account.
      </div>
      {error && <div style={{ fontSize: textSize.micro, color: colors.warning }}>{error}</div>}
      <div style={{ display: 'flex', gap: space.md }}>
        {/* The unsaveable state keeps its own muted chrome — it is what says
            "there is nothing here to save yet" before the button is pressed. */}
        <Button
          colors={colors}
          onClick={save}
          disabled={!canSave || saving}
          pending={saving}
          style={{
            '--pa-btn-fg': canSave ? colors.cyan : colors.textDim,
            '--pa-btn-bg': canSave ? colors.cyanSoft : 'transparent',
            '--pa-btn-border': canSave ? colors.borderHi : colors.border,
            '--pa-btn-bg-hover': canSave ? colors.cyanGlow : 'transparent',
            '--pa-btn-border-hover': canSave ? colors.cyan : colors.border,
            '--pa-btn-pad': `${space.sm}px ${space.xxl}px`,
            '--pa-btn-radius': `${radius.md}px`,
            fontFamily: font.body, fontSize: textSize.caption,
          } as CSSProperties}
        >{saving ? 'Saving…' : 'Save connection'}</Button>
        <Button
          colors={colors}
          onClick={onCancel}
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border': colors.border,
            '--pa-btn-pad': `${space.sm}px ${space.xxl}px`,
            '--pa-btn-radius': `${radius.md}px`,
            fontFamily: font.body, fontSize: textSize.caption,
          } as CSSProperties}
        >Cancel</Button>
      </div>
    </div>
  );
}
