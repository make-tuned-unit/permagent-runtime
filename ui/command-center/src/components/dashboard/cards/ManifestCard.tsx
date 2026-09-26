import { useState, useEffect, useCallback, useRef, type CSSProperties } from 'react';
import { radius, font, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Stat, SectionTitle, EmptyNote, CardScrollBody } from '../atoms';
import { apiFetch } from '../../../lib/api';
import type { CardManifest } from './registry';
import { CardIcon } from './cardIcons';
import { correctCardNote, isSupersededCardNote } from './cardNotes';
import { calendarAccessView, mergeCalendarCells } from './calendarAccess';
import { useShellCalendar } from './useShellCalendar';
import { CalendarAccessPanel } from './CalendarAccessPanel';
import { AsOf } from '../../common/AsOf';
import { Button } from '../../common/Button';

import { Tooltip } from '../../common/Tooltip';
/**
 * How long a fetch-once card's reading stays plain before its age becomes part
 * of what it says. Five minutes: long enough that opening the dashboard and
 * reading it is not nagged at, short enough that a tab left open all afternoon
 * cannot present breakfast's numbers as this moment's.
 */
const STATIC_CARD_STALE_AFTER_MS = 5 * 60_000;

/**
 * Widest a `list`/`key-value` label/value row is ever drawn, regardless of
 * how wide the card itself is (R3 "status near label"; coordinator, 2560px
 * round 2). Within the 560–720px range the coordinator gave, matching the
 * 720 already used for Growth's per-project row (GrowthResultsCard.tsx) so
 * the two label/value treatments in this app read the same width.
 */
const KV_ROW_MAX_WIDTH = 640;

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

  // Apple Calendar is read by the desktop shell through EventKit, not by the
  // daemon: the daemon's AppleScript read is refused by macOS with no prompt
  // (launchd owns permagentd), which left the user with "i cant find anywhere
  // to grant access to my iCal". The shell can ask, so the Calendar card gets
  // its Apple events — and its Connect button — from there, and keeps the
  // daemon for meetings and follow-ups logged on People.
  const isCalendar = manifest.type === 'calendar';
  const shellCalendar = useShellCalendar(isCalendar, manifest.refreshSeconds);
  const calendarView = isCalendar
    ? calendarAccessView(shellCalendar.inDesktopApp, shellCalendar.status, shellCalendar.fullAccessAsked)
    : null;

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
      padding: isCompact ? space.xxl : space.huge, borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      // auto_height cards (a one-line status strip) size to their content
      // instead of stretching to fill the grid row the packer reserved —
      // Dashboard.tsx pairs this with `alignSelf: 'start'` on the grid cell
      // itself so the reserved space is left blank below, not stretched into.
      height: manifest.autoHeight ? 'auto' : '100%', boxSizing: 'border-box',
      overflow: 'hidden',
      display: 'flex', flexDirection: 'column',
    }}>
      {children}
      {!polls && fetchedAt != null && (
        <div
          data-testid="manifest-card-as-of"
          style={{
            marginTop: 'auto', paddingTop: space.md,
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
  // The Calendar card does not wait for the daemon once the shell has
  // answered. `/api/dashboard/calendar` runs two osascript reads against
  // Calendar.app, each allowed eight seconds, before it replies — so the card
  // sat on "Loading…" while the Connect Calendar button (or today's Apple
  // events) it already had stayed hidden behind a request that could only add
  // People meetings. Only a connected calendar with nothing of its own yet
  // still waits: "No events today" before People has answered would be a guess.
  const shellCanSpeak = calendarView != null
    && calendarView.kind !== 'checking'
    && (calendarView.kind !== 'connected' || shellCalendar.events.length > 0);
  if (phase === 'loading' && !data && !shellCanSpeak) {
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
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: space.lg }}>
          {data?.note && (
            <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textDim, textAlign: 'center' }}>
              {data.note}
            </div>
          )}
          {configOpen ? (
            <div style={{ display: 'flex', gap: space.sm }}>
              <input
                autoFocus
                value={configValue}
                onChange={e => setConfigValue(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') submitConfig(); }}
                placeholder={manifest.configure!.placeholder}
                disabled={configBusy}
                style={{
                  flex: 1, padding: `${space.sm}px ${space.lg}px`, borderRadius: radius.sm,
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

  const cells = isCalendar
    ? mergeCalendarCells(shellCalendar.events, data?.cells ?? [])
    : (data?.cells ?? []);

  // A note that tells the user to grant a permission is worse than no note when
  // the card itself now offers the control: the stale instruction would sit
  // next to the button that replaced it. See `cardNotes.ts`.
  const note = isSupersededCardNote(manifest.type, data?.note) ? undefined : data?.note;
  const corrected = correctCardNote(manifest.type, note);

  const accessPanel = (compact: boolean) => calendarView && (
    <CalendarAccessPanel
      view={calendarView}
      onConnect={shellCalendar.connect}
      onOpenSettings={shellCalendar.openSettings}
      compact={compact}
    />
  );

  // ── Calendar with nothing listed ─────────────────────────────────────────
  // Every daemon note on this card is about its own Apple-events read ("No
  // events today", "Calendar is unavailable right now"). Once the shell owns
  // Apple Calendar those notes cannot speak for it: "No events today" beside a
  // Connect button would claim knowledge of a calendar nobody has read.
  if (calendarView && cells.length === 0) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        {calendarView.kind === 'connected' ? (
          <EmptyNote
            hint={phase === 'error' ? "Meetings logged on people couldn't be loaded just now." : undefined}
          >
            No events today
          </EmptyNote>
        ) : calendarView.kind === 'checking' ? (
          <EmptyNote>Checking Calendar access…</EmptyNote>
        ) : (
          accessPanel(false)
        )}
      </>,
    );
  }

  // ── Error / empty ──────────────────────────────────────────────────────────
  if (phase === 'error' && cells.length === 0) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <EmptyNote hint={corrected?.hint}>
          {corrected?.text || note || "Couldn't load this card"}
        </EmptyNote>
      </>,
    );
  }
  if (cells.length === 0) {
    return shell(
      <>
        <SectionTitle title={manifest.name} />
        <EmptyNote hint={corrected?.hint}>
          {corrected?.text || note || 'Nothing to show'}
        </EmptyNote>
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
    // A hero-only card (no supporting values, no forecast — e.g. a "status
    // line" card like henry_status) has nothing that needs the label/value
    // stacked on their own rows; one inline row is the whole content, so the
    // tile's height follows it instead of reserving two rows' worth of
    // vertical space for a single fact (coordinator, Phase-1 presentation
    // pass: "card height must follow content"). Weather/system-stats always
    // carry inline cells, so this branch never changes their look.
    const heroOnly = inline.length === 0 && forecast.length === 0;
    if (heroOnly) {
      return shell(
        <div style={{ display: 'flex', alignItems: 'center', gap: space.md, minHeight: 0 }}>
          <CardIcon name={hero.icon} size={14} />
          <span style={{
            fontFamily: font.body, fontSize: 10, fontWeight: 600, letterSpacing: '0.08em',
            textTransform: 'uppercase', color: colors.textDim, flexShrink: 0,
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          }}>
            {hero.label || manifest.name}
          </span>
          <span style={{
            fontFamily: font.display, fontSize: 15, fontWeight: 600, lineHeight: 1.2,
            color: hero.accent ? colors.cyan : colors.text,
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0,
            fontVariantNumeric: 'tabular-nums',
          }}>
            {hero.value}
          </span>
        </div>,
      );
    }
    return shell(
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm, minHeight: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: space.sm, color: colors.textDim }}>
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
            display: 'flex', flexWrap: 'wrap', gap: `${space.xs}px ${space.lg}px`,
            fontFamily: font.body, fontSize: 10.5, lineHeight: 1.45, minWidth: 0,
          }}>
            {inline.map((c, i) => (
              <Tooltip content={`${c.label}: ${c.value}`}>
                <span tabIndex={0} style={{ outline: 'none' }}>
                  <span
                    key={i}
                    style={{ display: 'inline-flex', alignItems: 'center', gap: space.xs, color: colors.textDim }}
                  >
                    <CardIcon name={c.icon} size={11} />
                    <span style={{
                      color: c.accent ? colors.cyan : colors.textMuted, fontWeight: 500,
                      fontVariantNumeric: 'tabular-nums',
                    }}>{c.value}</span>
                  </span>
                </span>
              </Tooltip>
            ))}
          </div>
        )}

        {forecast.length > 0 && (
          <div
            role="list"
            aria-label="Forecast"
            style={{
              display: 'flex', gap: space.xs, marginTop: space.xxs, minWidth: 0,
              borderTop: `1px solid ${colors.border}`, paddingTop: space.md,
            }}
          >
            {forecast.map((c, i) => (
              <Tooltip content={c.sub ? `${c.label}: ${c.value} · ${c.sub}` : `${c.label}: ${c.value}`}>
                <span tabIndex={0} style={{ outline: 'none' }}>
                  <div
                    key={i}
                    role="listitem"
                    style={{
                      flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column',
                      alignItems: 'center', gap: space.xs, fontFamily: font.body,
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
                </span>
              </Tooltip>
            ))}
          </div>
        )}
      </div>,
    );
  }

  return shell(
    <>
      <SectionTitle title={manifest.name} />
      <CardScrollBody testId="manifest-body">
        {manifest.layout === 'stat-grid' && (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: space.xxxl, alignContent: 'center', height: '100%' }}>
            {cells.map((c, i) => (
              <Stat key={i} label={c.label} value={c.value} delta={c.delta} cyan={c.accent} />
            ))}
          </div>
        )}

        {manifest.layout === 'key-value' && (
          // Capped (R3, coordinator 2560px round 2): a card stretched to the
          // full grid width used to put `justifyContent: 'space-between'`'s
          // two ends ~1,900px apart — a label at the left edge and its own
          // value at the right, read as two unrelated lines instead of one
          // row. `KV_ROW_MAX_WIDTH` keeps every row the width of its content
          // regardless of how wide the card is drawn, same fix already
          // applied to Growth's per-project row (GrowthResultsCard.tsx).
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg, maxWidth: KV_ROW_MAX_WIDTH }}>
            {cells.map((c, i) => (
              <div key={i} data-card-row="" style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: space.xl }}>
                <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textDim }}>{c.label}</span>
                <span style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: c.accent ? colors.cyan : colors.text }}>{c.value}</span>
              </div>
            ))}
          </div>
        )}

        {manifest.layout === 'list' && (
          // Same cap as `key-value` above, and for the same reason: this is
          // the layout Cost vs outcome and the Finance board actually use, so
          // it is the one the coordinator's 2560px reading caught label and
          // value stretched apart on.
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.xs, maxWidth: KV_ROW_MAX_WIDTH }}>
            {cells.map((c, i) => (
              <div key={i} data-card-row="" style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: space.xl, padding: `${space.sm}px 0`, borderBottom: i < cells.length - 1 ? `1px solid ${colors.border}` : 'none' }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 500, color: colors.text, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.label}</div>
                  {c.sub && <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, marginTop: space.xxs }}>{c.sub}</div>}
                </div>
                {c.value !== '' && c.value != null && (
                  <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: c.accent ? colors.cyan : colors.textMuted, flexShrink: 0 }}>{c.value}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </CardScrollBody>
      {/* Events from People are listed, but Apple Calendar is not connected:
          the way to connect it stays on the card rather than disappearing
          the moment anything else has something to show. */}
      {accessPanel(true)}
    </>,
  );
}

