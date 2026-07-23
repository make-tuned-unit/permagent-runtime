import { useState, useEffect, useCallback, useRef } from 'react';
import { radius, font } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Stat, SectionTitle } from '../atoms';
import { apiFetch } from '../../../lib/api';
import type { CardManifest } from './registry';

/**
 * The normalized payload every manifest-card data endpoint returns. Layout is
 * decided by the manifest (not the payload) so a single generic renderer can
 * draw any registered card. See `docs/architecture/DASHBOARD_CARD_EXTENSIBILITY.md`.
 */
export interface CardCell {
  label: string;
  value: string | number;
  /** Secondary line (list layout) or unit hint. */
  sub?: string;
  /** Small trailing delta badge. */
  delta?: string;
  /** Render the value in the accent colour. */
  accent?: boolean;
}

export interface CardData {
  cells?: CardCell[];
  /** A subtle empty / permission / error message. */
  note?: string;
  /**
   * `false` ⇒ the endpoint needs setup before it has data. When the manifest
   * declares a `configure` flow the card shows an inline setup input.
   */
  configured?: boolean;
}

interface Props {
  manifest: CardManifest;
}

type Phase = 'loading' | 'ready' | 'error';

/**
 * First-party renderer for declarative (manifest) cards. Self-fetches from the
 * manifest's `dataEndpoint`, polls on `refreshSeconds`, and draws one of a
 * constrained set of layouts. No skill-provided code runs here — the manifest
 * is pure data, which is the whole point of the extension boundary.
 */
export function ManifestCard({ manifest }: Props) {
  const { colors } = useTheme();
  const [data, setData] = useState<CardData | null>(null);
  const [phase, setPhase] = useState<Phase>('loading');
  const [configOpen, setConfigOpen] = useState(false);
  const [configValue, setConfigValue] = useState('');
  const [configBusy, setConfigBusy] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const requestGeneration = useRef(0);

  const fetchData = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const result = await apiFetch<CardData>(manifest.dataEndpoint);
      if (generation !== requestGeneration.current) return;
      setData(result);
      setPhase('ready');
    } catch {
      if (generation !== requestGeneration.current) return;
      // A failed poll keeps the last good data on screen rather than blanking.
      setPhase(prev => (prev === 'ready' ? 'ready' : 'error'));
    }
  }, [manifest.dataEndpoint]);

  useEffect(() => {
    fetchData();
    if (manifest.refreshSeconds && manifest.refreshSeconds > 0) {
      intervalRef.current = setInterval(fetchData, manifest.refreshSeconds * 1000);
    }
    return () => {
      clearInterval(intervalRef.current);
      ++requestGeneration.current;
    };
  }, [fetchData, manifest.refreshSeconds]);

  const submitConfig = useCallback(async () => {
    if (!manifest.configure || !configValue.trim()) return;
    setConfigBusy(true);
    try {
      await apiFetch(manifest.configure.endpoint, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ query: configValue.trim() }),
      });
      setConfigOpen(false);
      setConfigValue('');
      setPhase('loading');
      await fetchData();
    } catch {
      // Leave the input open so the user can retry.
    } finally {
      setConfigBusy(false);
    }
  }, [manifest.configure, configValue, fetchData]);

  const shell = (children: React.ReactNode) => (
    <div style={{
      padding: 24, borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
      height: '100%', boxSizing: 'border-box',
      overflow: 'hidden',
      display: 'flex', flexDirection: 'column',
    }}>
      {children}
    </div>
  );

  // ── Loading ──────────────────────────────────────────────────────────────
  if (phase === 'loading' && !data) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <CenteredNote color={colors.textDim}>Loading…</CenteredNote>
      </>,
    );
  }

  // ── Needs setup (e.g. weather with no location yet) ────────────────────────
  const needsSetup = data?.configured === false && manifest.configure;
  if (needsSetup) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 10 }}>
          {data?.note && (
            <div style={{ fontFamily: font.body, fontSize: 12, color: colors.textDim, textAlign: 'center' }}>
              {data.note}
            </div>
          )}
          {configOpen ? (
            <div style={{ display: 'flex', gap: 6 }}>
              <input
                autoFocus
                value={configValue}
                onChange={e => setConfigValue(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') submitConfig(); }}
                placeholder={manifest.configure!.placeholder}
                disabled={configBusy}
                style={{
                  flex: 1, padding: '6px 10px', borderRadius: radius.sm,
                  border: `1px solid ${colors.border}`, background: colors.bg,
                  color: colors.text, fontFamily: font.body, fontSize: 12, outline: 'none',
                }}
              />
              <button
                onClick={submitConfig}
                disabled={configBusy || !configValue.trim()}
                style={{
                  padding: '6px 12px', borderRadius: radius.sm, border: 'none',
                  background: colors.cyan, color: colors.textOnCyan,
                  fontFamily: font.body, fontSize: 12, fontWeight: 600,
                  cursor: configBusy ? 'default' : 'pointer', opacity: configBusy ? 0.6 : 1,
                }}
              >
                {configBusy ? '…' : 'Set'}
              </button>
            </div>
          ) : (
            <button
              onClick={() => setConfigOpen(true)}
              style={{
                alignSelf: 'center', padding: '6px 14px', borderRadius: radius.md,
                border: `1px solid ${colors.borderHi}`, background: colors.cyanSoft,
                color: colors.cyan, fontFamily: font.body, fontSize: 12, fontWeight: 600, cursor: 'pointer',
              }}
            >
              {manifest.configure!.label}
            </button>
          )}
        </div>
      </>,
    );
  }

  const cells = data?.cells ?? [];

  // ── Error / empty ──────────────────────────────────────────────────────────
  if (phase === 'error' && cells.length === 0) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <CenteredNote color={colors.textDim}>{data?.note || "Couldn't load this card"}</CenteredNote>
      </>,
    );
  }
  if (cells.length === 0) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <CenteredNote color={colors.textDim}>{data?.note || 'Nothing to show'}</CenteredNote>
      </>,
    );
  }

  // ── Populated layouts ──────────────────────────────────────────────────────
  return shell(
    <>
      <SectionTitle title={manifest.name} />
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        {manifest.layout === 'stat-grid' && (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20, alignContent: 'center', height: '100%' }}>
            {cells.map((c, i) => (
              <Stat key={i} label={c.label} value={c.value} delta={c.delta} cyan={c.accent} />
            ))}
          </div>
        )}

        {manifest.layout === 'key-value' && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {cells.map((c, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 12 }}>
                <span style={{ fontFamily: font.body, fontSize: 12, color: colors.textDim }}>{c.label}</span>
                <span style={{ fontFamily: font.body, fontSize: 13, fontWeight: 600, color: c.accent ? colors.cyan : colors.text }}>{c.value}</span>
              </div>
            ))}
          </div>
        )}

        {manifest.layout === 'list' && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            {cells.map((c, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 12, padding: '6px 0', borderBottom: i < cells.length - 1 ? `1px solid ${colors.border}` : 'none' }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontFamily: font.body, fontSize: 13, fontWeight: 500, color: colors.text, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.label}</div>
                  {c.sub && <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textDim, marginTop: 1 }}>{c.sub}</div>}
                </div>
                {c.value !== '' && c.value != null && (
                  <span style={{ fontFamily: font.body, fontSize: 12, color: c.accent ? colors.cyan : colors.textMuted, flexShrink: 0 }}>{c.value}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </>,
  );
}

function CenteredNote({ children, color }: { children: React.ReactNode; color: string }) {
  return (
    <div style={{
      flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
      fontFamily: font.body, fontSize: 12, color, textAlign: 'center',
    }}>
      {children}
    </div>
  );
}
