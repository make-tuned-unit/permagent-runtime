/**
 * The Home Calendar card: a real calendar (Day · Week · Month) with a live
 * "Next up" strip above it.
 *
 * Data comes from `useCalendarEvents` — the desktop shell's EventKit bridge
 * when access is granted, the daemon route otherwise, an honest "connects in
 * the desktop app" when neither — and the access controls are the existing
 * `CalendarAccessPanel`, so the grant path the earlier fix built is unchanged.
 *
 * Size: on Home the card is minimised — today's date, "Next up" and today's
 * list — and expands into Home focus mode (homeFocus.tsx) for the whole
 * calendar. Outside Home it measures itself: narrower than
 * `COMPACT_BELOW_PX` it is minimised, wider it is the whole calendar. See
 * `CALENDAR_SIZE_PRESETS` in registry.ts for the sizes the Layouts gallery
 * offers.
 *
 * Keyboard (card focused): ← / → previous / next period, T today, D / W / M
 * switch view, Esc closes the event popover.
 */

import {
  useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent,
} from 'react';
import { FiChevronLeft, FiChevronRight } from 'react-icons/fi';
import { font, radius, space, tabularNums, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { Tooltip } from '../../common/Tooltip';
import { segmentedTab, SEGMENT_STRIP_PAD, SEGMENT_STRIP_RADIUS } from '../../common/segmented';
import { runViewTransition } from '../../../lib/viewTransition';
import { EmptyNote, SectionTitle } from '../atoms';
import { calendarAccessView } from './calendarAccess';
import { CalendarAccessPanel } from './CalendarAccessPanel';
import { useShellCalendar } from './useShellCalendar';
import {
  addDays, calendarColor, CALENDAR_VIEWS, clockLabel, countdownLabel, eventKey, eventsOnDay,
  localeFirstDay, nextUp, nextUpText, periodTitle, rangeFor, shiftAnchor, sortEvents, startOfDay,
  upcomingEvents, wash, type CalEvent, type CalendarView,
} from './calendar/calendarModel';
import { useCalendarEvents, type CalendarSource } from './calendar/useCalendarEvents';
import {
  DayView, EventListRow, MonthView, WeekView, type PickEvent,
} from './calendar/CalendarViews';
import { EventPopover } from './calendar/EventPopover';
import { useCardPresentation } from '../homeFocus';

/** Below this card width the calendar is "Next up" + today's list. */
export const COMPACT_BELOW_PX = 420;
/** Day view puts its list beside the timeline from this width up. */
const DAY_WIDE_FROM_PX = 560;
/** Month chips carry a start time from this width up; narrower, the title wins. */
const MONTH_TIMES_FROM_PX = 700;
export const VIEW_STORAGE_KEY = 'permagent.dashboard.calendar.view.v1';
const NOW_TICK_MS = 30_000;

const VIEW_LABEL: Record<CalendarView, string> = { day: 'Day', week: 'Week', month: 'Month' };
const VIEW_KEY: Record<string, CalendarView> = { d: 'day', w: 'week', m: 'month' };

function readStoredView(): CalendarView {
  try {
    const v = localStorage.getItem(VIEW_STORAGE_KEY);
    if (v === 'day' || v === 'week' || v === 'month') return v;
  } catch { /* private mode: fall through */ }
  return 'week';
}

interface Props {
  /** Pin "now" (tests, evidence harness). Absent: the real clock, ticking. */
  nowMs?: number;
  /** `auto` measures the card; the others force a layout (tests). */
  sizeMode?: 'auto' | 'compact' | 'full';
  initialView?: CalendarView;
}

export function CalendarCard({ nowMs: pinnedNow, sizeMode = 'auto', initialView }: Props) {
  const { colors } = useTheme();
  const calendarId = useId();
  const tabId = (v: CalendarView) => `${calendarId}-tab-${v}`;
  const panelId = `${calendarId}-panel`;
  const rootRef = useRef<HTMLDivElement>(null);
  const [clock, setClock] = useState(() => Date.now());
  const nowMs = pinnedNow ?? clock;
  const [view, setView] = useState<CalendarView>(() => initialView ?? readStoredView());
  const [anchor, setAnchor] = useState(() => pinnedNow ?? Date.now());
  const [width, setWidth] = useState(0);
  const [popover, setPopover] = useState<{ event: CalEvent; anchor: HTMLElement } | null>(null);
  const focusViewAfterExpand = useRef<CalendarView | null>(null);
  const firstDay = useMemo(localeFirstDay, []);

  useEffect(() => {
    if (pinnedNow != null) return;
    const t = setInterval(() => setClock(Date.now()), NOW_TICK_MS);
    return () => clearInterval(t);
  }, [pinnedNow]);

  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    setWidth(el.getBoundingClientRect().width);
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(entries => {
      const w = entries[0]?.contentRect.width;
      if (w) setWidth(w);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // On Home the card is minimised (Next up + today) until it is expanded
  // into focus, where it is the full Day / Week / Month calendar. Elsewhere
  // (no Home focus context) it still sizes itself by width; a forced
  // `sizeMode` always wins.
  const home = useCardPresentation();
  const compact = sizeMode === 'compact' ? true
    : sizeMode === 'full' ? false
    : home ? home.presentation !== 'focused'
    : width > 0 && width < COMPACT_BELOW_PX;

  useEffect(() => {
    if (compact || !focusViewAfterExpand.current) return;
    const target = focusViewAfterExpand.current;
    focusViewAfterExpand.current = null;
    rootRef.current?.querySelector<HTMLButtonElement>(`[data-testid="calendar-view-${target}"]`)?.focus();
  }, [compact, view]);

  const shell = useShellCalendar(true, 300, false);
  const access = calendarAccessView(shell.inDesktopApp, shell.status, shell.fullAccessAsked);

  const mainRange = useMemo(
    () => (compact ? null : rangeFor(view, anchor, firstDay)),
    [compact, view, anchor, firstDay],
  );
  // "Next up" always looks a week ahead from today, whatever page is open.
  const nextRange = useMemo(() => {
    const today = startOfDay(nowMs);
    return { startMs: today, endMs: addDays(today, 8) };
  }, [nowMs]);

  const main = useCalendarEvents(mainRange, shell, nowMs);
  const ahead = useCalendarEvents(nextRange, shell, nowMs);

  const next = useMemo(() => nextUp(ahead.events, nowMs), [ahead.events, nowMs]);

  const pick: PickEvent = useCallback((event, el) => setPopover({ event, anchor: el }), []);
  const closePopover = useCallback(() => {
    setPopover(p => {
      if (p && document.contains(p.anchor)) p.anchor.focus({ preventScroll: true });
      return null;
    });
  }, []);

  const goToday = () => setAnchor(nowMs);
  const step = (dir: -1 | 1) => setAnchor(a => shiftAnchor(view, a, dir));
  const switchView = (next: CalendarView) => {
    if (next === view) return;
    const dir = CALENDAR_VIEWS.indexOf(next) < CALENDAR_VIEWS.indexOf(view) ? 'backward' : 'forward';
    runViewTransition(() => {
      setView(next);
      try { localStorage.setItem(VIEW_STORAGE_KEY, next); } catch { /* not persisted: still works */ }
    }, { direction: dir });
  };
  const openCompactView = (next: CalendarView, fromKeyboard = false) => {
    if (fromKeyboard) focusViewAfterExpand.current = next;
    switchView(next);
    home?.expand();
  };
  const pickDay = (dayMs: number) => { setAnchor(dayMs); switchView('day'); };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement;
    // React bubbles a portal's keys through its React parent: a key pressed in
    // the event popover (portalled to <body>) must not page the calendar.
    if (!rootRef.current?.contains(t)) return;
    if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable) return;
    if (compact) {
      if (!t.closest('[data-calendar-view-launcher]')) return;
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
      e.preventDefault();
      const focusedView = CALENDAR_VIEWS.find(candidate => t.closest(`[data-testid="calendar-open-${candidate}"]`)) ?? view;
      const index = CALENDAR_VIEWS.indexOf(focusedView);
      const next = e.key === 'Home' ? CALENDAR_VIEWS[0]
        : e.key === 'End' ? CALENDAR_VIEWS[CALENDAR_VIEWS.length - 1]
          : CALENDAR_VIEWS[(index + (e.key === 'ArrowRight' ? 1 : -1) + CALENDAR_VIEWS.length) % CALENDAR_VIEWS.length];
      openCompactView(next, true);
      return;
    }
    if (t.closest('[role="tablist"]') && ['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) {
      e.preventDefault();
      const index = CALENDAR_VIEWS.indexOf(view);
      const next = e.key === 'Home' ? CALENDAR_VIEWS[0]
        : e.key === 'End' ? CALENDAR_VIEWS[CALENDAR_VIEWS.length - 1]
          : CALENDAR_VIEWS[(index + (e.key === 'ArrowRight' ? 1 : -1) + CALENDAR_VIEWS.length) % CALENDAR_VIEWS.length];
      switchView(next);
      rootRef.current.querySelector<HTMLButtonElement>(`[data-testid="calendar-view-${next}"]`)?.focus();
      return;
    }
    if (e.key === 'ArrowLeft') { e.preventDefault(); step(-1); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); step(1); }
    else if (e.key.toLowerCase() === 't') { e.preventDefault(); goToday(); }
    else if (VIEW_KEY[e.key.toLowerCase()]) { e.preventDefault(); switchView(VIEW_KEY[e.key.toLowerCase()]); }
  };

  const ghost = {
    '--pa-btn-bg': 'transparent', '--pa-btn-fg': colors.textMuted, '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi, '--pa-btn-fg-hover': colors.text, '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-bg-active': colors.surface, '--pa-btn-pad': `${space.sm}px ${space.lg}px`,
    '--pa-btn-radius': `${radius.sm}px`, '--pa-btn-weight': 500,
    fontFamily: font.body, fontSize: textSize.caption,
  } as CSSProperties;

  const showAccess = access.kind !== 'connected' && access.kind !== 'checking' && access.text !== '';
  const emptyWord = view === 'day' ? (startOfDay(anchor) === startOfDay(nowMs) ? 'today' : 'on this day') : view === 'week' ? 'this week' : 'this month';
  // Granted, yet neither EventKit nor the daemon answered and nothing is
  // cached: say so rather than claim an empty week. (Without a grant, or
  // outside the desktop app, the access panel already explains the gap.)
  const shown = compact ? ahead : main;
  const readFailed = shown.failed && shell.inDesktopApp && shell.status === 'fullAccess';
  const aheadFailed = ahead.failed && shell.inDesktopApp && shell.status === 'fullAccess';
  const loadingFirst =(compact ? ahead : main).loading && (compact ? ahead : main).events.length === 0;
  const showNextUp = !compact || Boolean(next) || ahead.loading || aheadFailed;

  return (
    <div
      ref={rootRef}
      tabIndex={0}
      role="group"
      aria-label="Calendar"
      data-testid="calendar-card"
      data-mode={compact ? 'compact' : 'full'}
      data-view={view}
      data-source={(compact ? ahead : main).source}
      onKeyDown={onKeyDown}
      style={{
        height: '100%', boxSizing: 'border-box', outline: 'none',
        padding: compact ? space.xxl : space.xxxl, borderRadius: radius.lg,
        background: colors.surface, border: `1px solid ${colors.border}`,
        boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
        display: 'flex', flexDirection: 'column', gap: space.lg, overflow: 'hidden', minWidth: 0,
      }}
    >
      {compact && <SectionTitle title="Calendar" />}
      {compact && home && (
        <CompactDateHeader nowMs={nowMs} events={ahead.events} loading={ahead.loading && ahead.events.length === 0}
          source={ahead.source} failed={readFailed} onOpenView={nextView => openCompactView(nextView)} />
      )}
      <div data-testid={compact ? 'calendar-content-scroll' : undefined} style={compact ? {
        minHeight: 0, flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: space.lg,
      } : { display: 'contents' }}>
      {!compact && (
        <div data-testid="calendar-toolbar" style={{ display: 'flex', flexDirection: 'column', gap: space.sm, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.lg, flexWrap: 'wrap', minWidth: 0 }}>
            <h3 data-testid="calendar-title" aria-live="polite" style={{
              margin: 0, flex: '1 1 150px', minWidth: 0, fontFamily: font.display, fontSize: textSize.heading,
              fontWeight: 600, color: colors.text, letterSpacing: '-0.005em', textAlign: 'left',
            }}>
              {periodTitle(view, anchor, firstDay)}
            </h3>
            <div role="tablist" aria-label="Calendar view" style={{
              display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))',
              flex: '1 1 192px', minWidth: 'min(100%, 192px)', maxWidth: 252,
              gap: SEGMENT_STRIP_PAD, padding: SEGMENT_STRIP_PAD,
              borderRadius: SEGMENT_STRIP_RADIUS, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            }}>
              {CALENDAR_VIEWS.map(v => (
                <Button key={v} colors={colors} role="tab" id={tabId(v)} aria-selected={v === view} tabIndex={v === view ? 0 : -1}
                  aria-controls={panelId} data-testid={`calendar-view-${v}`} onClick={() => switchView(v)} flashSuccess={false}
                  style={{ ...segmentedTab(colors, v === view), width: '100%', minWidth: 0, whiteSpace: 'nowrap' }}>
                  {VIEW_LABEL[v]}
                </Button>
              ))}
            </div>
          </div>
          <div data-testid="calendar-period-controls" style={{ display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: space.sm }}>
            <Tooltip content="Previous" shortcut="←">
              <Button colors={colors} type="button" aria-label="Previous period" onClick={() => step(-1)}
                flashSuccess={false} data-testid="calendar-prev" style={{ ...ghost, '--pa-btn-pad': `${space.sm}px` } as CSSProperties}>
                <FiChevronLeft size={14} />
              </Button>
            </Tooltip>
            <Tooltip content="Jump to today" shortcut="T">
              <Button colors={colors} type="button" onClick={goToday} flashSuccess={false}
                data-testid="calendar-today" style={ghost}>Today</Button>
            </Tooltip>
            <Tooltip content="Next" shortcut="→">
              <Button colors={colors} type="button" aria-label="Next period" onClick={() => step(1)}
                flashSuccess={false} data-testid="calendar-next" style={{ ...ghost, '--pa-btn-pad': `${space.sm}px` } as CSSProperties}>
                <FiChevronRight size={14} />
              </Button>
            </Tooltip>
          </div>
        </div>
      )}

      {showNextUp && <NextUpStrip stacked={compact} next={next} loading={ahead.loading && ahead.events.length === 0}
        empty={ahead.source === 'eventkit'} todayOnly={ahead.source === 'daemon'} failed={aheadFailed} onPick={pick} />}

      {(compact ? ahead : main).source === 'daemon' && (
        <div data-testid="calendar-partial" role="status">
          <EmptyNote>Only today's events are available. Other dates have not been checked.</EmptyNote>
        </div>
      )}

      {compact && readFailed && <div data-testid="calendar-failed" role="status" style={{ display: 'flex', alignItems: 'center', gap: space.lg }}>
        <EmptyNote>The calendar could not be read just now</EmptyNote>
        <Button colors={colors} type="button" onClick={ahead.refresh} flashSuccess={false} data-testid="calendar-retry" style={ghost}>Retry</Button>
      </div>}
      {compact ? (
        <CompactList events={ahead.events} nowMs={nowMs} onPick={pick} loading={loadingFirst} />
      ) : (
        <div key={view} id={panelId} role="tabpanel" aria-labelledby={tabId(view)} tabIndex={0}
          className="pa-enter" data-testid="calendar-body" style={{ minWidth: 0 }}>
          {readFailed ? (
            <div data-testid="calendar-failed" style={{ display: 'flex', alignItems: 'center', gap: space.lg, marginBottom: space.md }}>
              <EmptyNote>The calendar could not be read just now</EmptyNote>
              <Button colors={colors} type="button" onClick={main.refresh} flashSuccess={false}
                data-testid="calendar-retry" style={ghost}>Retry</Button>
            </div>
          ) : main.source === 'eventkit' && main.events.length === 0 && !main.loading && view !== 'month' ? (
            <EmptyNote>
              No events {emptyWord}
            </EmptyNote>
          ) : null}
          {view === 'month' && (
            <MonthView anchorMs={anchor} nowMs={nowMs} firstDay={firstDay} events={main.events}
              onPickEvent={pick} onPickDay={pickDay} roomy={width >= MONTH_TIMES_FROM_PX} />
          )}
          {view === 'week' && (
            <>
              {width > 0 && width < 620 && <EmptyNote>Scroll horizontally to see the full week</EmptyNote>}
              <div data-testid="calendar-week-scroll" style={{ overflowX: 'auto' }}>
                <div style={{ minWidth: 560 }}>
                  <WeekView anchorMs={anchor} nowMs={nowMs} firstDay={firstDay} events={main.events}
                    onPickEvent={pick} onPickDay={pickDay} />
                </div>
              </div>
            </>
          )}
          {view === 'day' && (
            <DayView anchorMs={anchor} nowMs={nowMs} events={main.events} onPickEvent={pick}
              onPickDay={pickDay} wide={width >= DAY_WIDE_FROM_PX}
              emptyLabel={main.loading ? 'Checking events…' : readFailed ? 'Events unavailable' : main.source === 'none' ? 'Connect a calendar to see events' : main.source === 'daemon' ? (startOfDay(anchor) !== startOfDay(nowMs) ? 'This date has not been checked' : 'No events reported today') : undefined} />
          )}
          {view === 'month' && main.source === 'eventkit' && main.events.length === 0 && !main.loading && !(readFailed) && (
            <div style={{ marginTop: space.md }}><EmptyNote>No events {emptyWord}</EmptyNote></div>
          )}
        </div>
      )}

      {showAccess && (
        <CalendarAccessPanel view={access} onConnect={shell.connect} onOpenSettings={shell.openSettings} compact />
      )}
      </div>

      {popover && <EventPopover event={popover.event} anchor={popover.anchor} onClose={closePopover} />}
    </div>
  );
}

/** Home's compact agenda: a left-aligned date summary and view launch actions. */
function CompactDateHeader({ nowMs, events, loading, source, failed, onOpenView }: {
  nowMs: number; events: readonly CalEvent[]; loading: boolean; source: CalendarSource; failed: boolean;
  onOpenView: (view: CalendarView) => void;
}) {
  const { colors } = useTheme();
  const now = new Date(nowMs);
  const fullDate = now.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
  const left = eventsOnDay(events, startOfDay(nowMs)).filter(e => e.allDay || e.endMs > nowMs).length;
  const summary = loading ? 'Checking today’s events…'
    : failed ? 'Calendar unavailable'
    : source === 'none' ? 'Calendar not connected'
    : left > 0 ? `${left} event${left === 1 ? '' : 's'} left today`
    : source === 'daemon' ? 'No more events reported today'
    : 'Nothing left today';
  return (
    <div
      data-testid="calendar-compact-header"
      data-home-card-heading
      role="group"
      aria-label={`Calendar agenda for ${fullDate}: ${summary}`}
      style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.lg,
        flexWrap: 'wrap', width: '100%', minWidth: 0, textAlign: 'left',
      }}
    >
      <span data-testid="calendar-date-summary" style={{
        display: 'flex', flex: '1 1 150px', minWidth: 0, flexDirection: 'column', alignItems: 'flex-start', gap: space.xxs,
      }}>
        <span data-testid="calendar-compact-date" style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 600, color: colors.text,
          overflowWrap: 'anywhere', textAlign: 'left' }}>{fullDate}</span>
        <span data-testid="calendar-today-summary" style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted }}>
          {summary}
        </span>
      </span>
      <div role="group" aria-label="Open calendar view" data-calendar-view-launcher style={{
        display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', flex: '1 1 192px',
        minWidth: 'min(100%, 192px)', maxWidth: 252, gap: SEGMENT_STRIP_PAD, padding: SEGMENT_STRIP_PAD,
        borderRadius: SEGMENT_STRIP_RADIUS, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      }}>
        {CALENDAR_VIEWS.map(v => (
          <Button key={v} colors={colors} type="button" aria-label={`Open ${VIEW_LABEL[v]} calendar`}
            data-testid={`calendar-open-${v}`} onClick={() => onOpenView(v)} flashSuccess={false}
            style={{ ...segmentedTab(colors, false), width: '100%', minWidth: 0, whiteSpace: 'nowrap' }}>
            {VIEW_LABEL[v]}
          </Button>
        ))}
      </div>
    </div>
  );
}

/** "Next: Standup · in 42 min · Work", re-evaluated by the card's own clock. */
function NextUpStrip({ next, loading, empty, todayOnly, failed, onPick, stacked }: {
  stacked: boolean; next: ReturnType<typeof nextUp>; loading: boolean; empty: boolean; todayOnly: boolean; failed: boolean; onPick: PickEvent;
}) {
  const { colors } = useTheme();
  const base: CSSProperties = {
    display: 'flex', alignItems: 'center', gap: space.lg, minWidth: 0,
    padding: `${space.md}px ${space.xl}px`, borderRadius: radius.md,
    fontFamily: font.body, fontSize: textSize.caption,
  };
  if (!next) {
    return (
      <div data-testid="calendar-next-up" data-state={loading ? 'loading' : 'none'}
        style={{ ...base, background: colors.fillSubtle, color: colors.textDim }}>
        {loading ? 'Checking your calendar…' : failed ? 'Upcoming events could not be read' : todayOnly ? 'Nothing else reported today; later dates are unavailable' : empty ? 'Nothing coming up in the next 7 days' : 'Next up appears once a calendar is connected'}
      </div>
    );
  }
  const hue = calendarColor(next.event.calendar, colors);
  const titleStyle: CSSProperties = { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0, flex: '0 1 auto' };
  return (
    <button
      type="button"
      className="pa-btn pa-btn--composite"
      data-testid="calendar-next-up"
      data-state={next.state}
      aria-label={nextUpText(next)}
      onClick={e => onPick(next.event, e.currentTarget)}
      style={{
        ...base, width: '100%', justifyContent: 'flex-start', textAlign: 'left', flexWrap: stacked ? 'wrap' : 'nowrap',
        '--pa-btn-bg': wash(hue, 14), '--pa-btn-fg': colors.text, '--pa-btn-border': 'transparent',
        '--pa-btn-bg-hover': wash(hue, 24), '--pa-btn-fg-hover': colors.text, '--pa-btn-border-hover': 'transparent',
        '--pa-btn-bg-active': wash(hue, 30), '--pa-btn-pad': `${space.md}px ${space.xl}px`,
        '--pa-btn-radius': `${radius.md}px`, '--pa-btn-weight': 500,
      } as CSSProperties}
    >
      <span aria-hidden style={{ width: 8, height: 8, borderRadius: radius.pill, background: hue, flexShrink: 0 }} />
      <span style={{ fontWeight: 600, color: hue, flexShrink: 0 }}>{next.state === 'ongoing' ? 'Now' : 'Next'}</span>
      {!stacked && <span style={titleStyle}>{next.event.title}</span>}
      <span style={{ color: colors.textMuted, flexShrink: 0, ...tabularNums }} data-testid="calendar-countdown">
        {next.state === 'ongoing' ? (next.minutes < 1 ? 'ending now' : `ends ${countdownLabel(next.minutes)}`) : countdownLabel(next.minutes)}
        {next.state === 'upcoming' && next.minutes >= 60 ? ` · ${clockLabel(next.event.startMs)}` : ''}
      </span>
      {next.event.calendar && (
        <span style={{ color: colors.textDim, marginLeft: 'auto', flexShrink: 0, maxWidth: '35%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {next.event.calendar}
        </span>
      )}
      {stacked && <span style={{ ...titleStyle, flexBasis: '100%', order: 5 }}>{next.event.title}</span>}
    </button>
  );
}

/** Compact size: today's list, or the next three events when today is done. */
function CompactList({ events, nowMs, onPick, loading }: {
  events: readonly CalEvent[]; nowMs: number; onPick: PickEvent; loading: boolean;
}) {
  const { colors } = useTheme();
  const today = eventsOnDay(events, startOfDay(nowMs));
  // Today's list is what is left of today (all-day items stay all day); once
  // the day's timed events are over it gives way to the next three to come.
  const stillToday = today.filter(e => !e.allDay && e.endMs > nowMs);
  const list = stillToday.length > 0
    ? sortEvents(today.filter(e => e.allDay || e.endMs > nowMs)).slice(0, 5)
    : upcomingEvents(events, nowMs, 3);
  const heading = stillToday.length > 0 ? 'Today' : 'Coming up';
  if (loading) return null;
  if (list.length === 0) return null;
  return (
    <div data-testid="calendar-compact-list" style={{ display: 'flex', flexDirection: 'column', gap: space.xxs, minWidth: 0 }}>
      <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
        textTransform: 'uppercase', color: colors.textDim, paddingBottom: space.xs }}>{heading}</div>
      {list.map(e => <EventListRow key={eventKey(e)} event={e} onPick={onPick} />)}
    </div>
  );
}
