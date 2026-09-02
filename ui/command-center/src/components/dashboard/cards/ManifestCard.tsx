import { useState, useEffect, useCallback, useRef, type CSSProperties } from 'react';
import { radius, font, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Stat, SectionTitle, EmptyNote } from '../atoms';
import { apiFetch } from '../../../lib/api';
import type { CardManifest } from './registry';
import { CardIcon } from './cardIcons';
import { AsOf } from '../../common/AsOf';
import { Button } from '../../common/Button';

/**
 * How long a fetch-once card's reading stays plain before its age becomes part
 * of what it says. Five minutes: long enough that opening the dashboard and
 * reading it is not nagged at, short enough that a tab left open all afternoon
 * cannot present breakfast's numbers as this moment's.
 */
const STATIC_CARD_STALE_AFTER_MS = 5 * 60_000;

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
  /**
   * Grouping hint from the data source, e.g. `'forecast'`. The source names
   * the meaning; this component decides the drawing. Cells with no group get
   * the dense inline treatment, which is right for a humidity reading and
   * useless for four days of weather — those need their day labels.
   */
  group?: string;
  /** Render the value in the accent colour. */
  accent?: boolean;
  /** Glyph name from the daemon (see cardIcons.tsx). */
  icon?: string;
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
  const [fetchedAt, setFetchedAt] = useState<number | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const requestGeneration = useRef(0);

  const fetchData = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const result = await apiFetch<CardData>(manifest.dataEndpoint);
      if (generation !== requestGeneration.current) return;
      setData(result);
      setFetchedAt(Date.now());
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

  // Resolves `false` on failure so the Button contract never ticks on one: this
  // handler swallows its own error (the input stays open for a retry), and a
  // green tick over a location that was never saved would be a lie.
  const submitConfig = useCallback(async () => {
    if (!manifest.configure || !configValue.trim()) return false;
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
      return true;
    } catch {
      // Leave the input open so the user can retry.
      return false;
    } finally {
      setConfigBusy(false);
    }
  }, [manifest.configure, configValue, fetchData]);

  const isCompact = manifest.layout === 'compact';

  /**
   * A card whose manifest declares `refreshSeconds: 0` fetches once when the
   * dashboard mounts and never again. Its figures then sit next to cards that
   * refresh every thirty seconds, in the same type, with nothing saying which
   * is which — so a number that has been frozen since you opened the tab reads
   * exactly like one confirmed a moment ago.
   *
   * Polling cards are left alone: they are current by construction, and a
   * timestamp on every card would be noise that teaches nobody anything.
   */
  const polls = (manifest.refreshSeconds ?? 0) > 0;

  const shell = (children: React.ReactNode) => (
    <div style={{
      padding: isCompact ? 14 : 24, borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
      height: '100%', boxSizing: 'border-box',
      overflow: 'hidden',
      display: 'flex', flexDirection: 'column',
    }}>
      {children}
      {!polls && fetchedAt != null && (
        <div
          data-testid="manifest-card-as-of"
          style={{
            marginTop: 'auto', paddingTop: 8,
            fontFamily: font.body, fontSize: 10,
          }}
        >
          <AsOf
            asOf={fetchedAt}
            prefix="As of"
            suffix="not refreshing"
            // Fetched once on mount: it starts being a reading about the past
            // the moment the fetch lands, and how far past is the whole point.
            staleAfterMs={STATIC_CARD_STALE_AFTER_MS}
          />
        </div>
      )}
    </div>
  );

  // ── Loading ──────────────────────────────────────────────────────────────
  if (phase === 'loading' && !data) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <EmptyNote>Loading…</EmptyNote>
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
            <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textDim, textAlign: 'center' }}>
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
                  color: colors.text, fontFamily: font.body, fontSize: textSize.caption, outline: 'none',
                }}
              />
              <Button
                colors={colors}
                variant="primary"
                type="button"
                onClick={submitConfig}
                disabled={configBusy || !configValue.trim()}
                style={{
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': `${radius.sm}px`,
                  fontFamily: font.body, fontSize: textSize.caption,
                } as CSSProperties}
              >
                {configBusy ? '…' : 'Set'}
              </Button>
            </div>
          ) : (
            <Button
              colors={colors}
              type="button"
              onClick={() => setConfigOpen(true)}
              style={{
                '--pa-btn-bg': colors.cyanSoft,
                '--pa-btn-fg': colors.cyan,
                '--pa-btn-border': colors.borderHi,
                '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-fg-hover': colors.cyan,
                '--pa-btn-border-hover': colors.cyan,
                '--pa-btn-bg-active': colors.cyanGlow,
                '--pa-btn-pad': '6px 14px',
                '--pa-btn-radius': `${radius.md}px`,
                '--pa-btn-weight': 600,
                alignSelf: 'center', fontFamily: font.body, fontSize: textSize.caption,
              } as CSSProperties}
            >
              {manifest.configure!.label}
            </Button>
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
        <EmptyNote>{data?.note || "Couldn't load this card"}</EmptyNote>
      </>,
    );
  }
  if (cells.length === 0) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <EmptyNote>{data?.note || 'Nothing to show'}</EmptyNote>
      </>,
    );
  }

  // ── Populated layouts ──────────────────────────────────────────────────────
  if (isCompact) {
    // Ambient tile. Three rules, all learned from the first attempt:
    //
    //  1. NO vertical void. The first version pinned the hero to the top and
    //     the detail rows to the bottom with `marginTop: auto`, which in a
    //     tile taller than its content opened a dead gap down the middle.
    //     Content now stacks from the top and the tile is sized to fit it.
    //  2. An icon carries the meaning faster than the words do — you read
    //     "rain" from the glyph before parsing "Drizzle".
    //  3. Supporting values sit on ONE dense row, not one row each.
    //  4. A group the source named gets its own treatment. The forecast is
    //     labelled days, not more supporting values — dropped into the inline
    //     row it renders as three anonymous temperature pairs.
    const [hero, ...rest] = cells;
    const inline = rest.filter(c => c.group !== 'forecast');
    const forecast = rest.filter(c => c.group === 'forecast');
    return shell(
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6, minHeight: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, color: colors.textDim }}>
          <CardIcon name={hero.icon} size={13} />
          <span style={{
            fontFamily: font.body, fontSize: 10, fontWeight: 600, letterSpacing: '0.08em',
            textTransform: 'uppercase',
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          }}>
            {hero.label || manifest.name}
          </span>
        </div>

        <div style={{
          fontFamily: font.display, fontSize: 22, fontWeight: 600, lineHeight: 1.1,
          color: hero.accent ? colors.cyan : colors.text,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          fontVariantNumeric: 'tabular-nums',
        }}>
          {hero.value}
        </div>

        {inline.length > 0 && (
          <div style={{
            display: 'flex', flexWrap: 'wrap', gap: '3px 10px',
            fontFamily: font.body, fontSize: 10.5, lineHeight: 1.45, minWidth: 0,
          }}>
            {inline.map((c, i) => (
              <span
                key={i}
                title={`${c.label}: ${c.value}`}
                style={{ display: 'inline-flex', alignItems: 'center', gap: 3, color: colors.textDim }}
              >
                <CardIcon name={c.icon} size={11} />
                <span style={{
                  color: c.accent ? colors.cyan : colors.textMuted, fontWeight: 500,
                  fontVariantNumeric: 'tabular-nums',
                }}>{c.value}</span>
              </span>
            ))}
          </div>
        )}

        {forecast.length > 0 && (
          <div
            role="list"
            aria-label="Forecast"
            style={{
              display: 'flex', gap: 4, marginTop: 2, minWidth: 0,
              borderTop: `1px solid ${colors.border}`, paddingTop: 7,
            }}
          >
            {forecast.map((c, i) => (
              <div
                key={i}
                role="listitem"
                title={c.sub ? `${c.label}: ${c.value} · ${c.sub}` : `${c.label}: ${c.value}`}
                style={{
                  flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column',
                  alignItems: 'center', gap: 2, fontFamily: font.body,
                }}
              >
                <span style={{
                  fontSize: 9.5, fontWeight: 600, letterSpacing: '0.06em',
                  textTransform: 'uppercase', color: colors.textDim,
                }}>{c.label}</span>
                <CardIcon name={c.icon} size={13} />
                <span style={{
                  fontSize: 10.5, color: colors.textMuted, fontVariantNumeric: 'tabular-nums',
                  whiteSpace: 'nowrap',
                }}>{c.value}</span>
                {c.sub && (
                  <span style={{ fontSize: 9, color: colors.textDim, whiteSpace: 'nowrap' }}>{c.sub}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>,
    );
  }

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
                <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textDim }}>{c.label}</span>
                <span style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: c.accent ? colors.cyan : colors.text }}>{c.value}</span>
              </div>
            ))}
          </div>
        )}

        {manifest.layout === 'list' && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            {cells.map((c, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 12, padding: '6px 0', borderBottom: i < cells.length - 1 ? `1px solid ${colors.border}` : 'none' }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 500, color: colors.text, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.label}</div>
                  {c.sub && <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, marginTop: 1 }}>{c.sub}</div>}
                </div>
                {c.value !== '' && c.value != null && (
                  <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: c.accent ? colors.cyan : colors.textMuted, flexShrink: 0 }}>{c.value}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </>,
  );
}

