/**
 * The three drawings of the Calendar card — Month, Week, Day — plus the small
 * pieces they share (event chip, time grid). Pure drawing: every number they
 * show comes from `calendarModel.ts`, and every click goes back out through
 * `onPickEvent` / `onPickDay`.
 *
 * Geometry is deliberately fixed (a month row is `MONTH_ROW_PX`, an hour is
 * `HOUR_PX`) instead of stretched to the card: the Dashboard lets a card grow
 * to its content, so a view whose height is its own is the one that does not
 * jitter when the user pages from a 5-week month to a 6-week one.
 */

import { useLayoutEffect, useRef, type CSSProperties } from 'react';
import { font, radius, space, tabularNums, textSize } from '../../../../styles/tokens';
import { useTheme } from '../../../../styles/useTheme';
import type { ThemeColors } from '../../../../styles/tokens';
import {
  calendarColor, clockLabel, eventKey, eventsOnDay, layoutDay, monthDayChips,
  monthGrid, sameDay, startOfDay, weekDays, wash,
  type CalEvent, type MonthCell,
} from './calendarModel';

export const MONTH_ROW_PX = 92;
export const HOUR_PX = 44;
export const TIME_GUTTER_PX = 44;
const TIME_GRID_PX = 420;
const CHIP_PX = 18;

export type PickEvent = (event: CalEvent, anchor: HTMLElement) => void;

function btnVars(colors: ThemeColors, bgHover: string = colors.fillHover): CSSProperties {
  return {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': colors.text,
    '--pa-btn-border': 'transparent',
    '--pa-btn-bg-hover': bgHover,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border-hover': 'transparent',
    '--pa-btn-bg-active': colors.fillActive,
    '--pa-btn-pad': '0px',
    '--pa-btn-radius': `${radius.xs}px`,
    '--pa-btn-weight': 500,
  } as CSSProperties;
}

// ── event chip (Month, all-day rows, lists) ──────────────────────────────

export function EventChip({
  event, onPick, showTime = true, testId,
}: { event: CalEvent; onPick: PickEvent; showTime?: boolean; testId?: string }) {
  const { colors } = useTheme();
  const hue = calendarColor(event.calendar, colors);
  return (
    <button
      type="button"
      className="pa-btn pa-btn--composite"
      data-testid={testId ?? 'calendar-chip'}
      data-event-key={eventKey(event)}
      aria-label={`${event.title}, ${event.allDay ? 'all day' : clockLabel(event.startMs)}`}
      onClick={e => onPick(event, e.currentTarget)}
      style={{
        ...btnVars(colors, wash(hue, 30)),
        '--pa-btn-bg': wash(hue, 16),
        justifyContent: 'flex-start', gap: space.xs,
        height: CHIP_PX, minWidth: 0, width: '100%',
        paddingInline: space.sm,
        borderLeft: `2px solid ${hue}`,
        fontFamily: font.body, fontSize: textSize.micro, lineHeight: `${CHIP_PX - 2}px`,
        textAlign: 'left', overflow: 'hidden', whiteSpace: 'nowrap',
      } as CSSProperties}
    >
      {showTime && !event.allDay && (
        <span style={{ color: colors.textMuted, flexShrink: 0, ...tabularNums }}>
          {shortTime(event.startMs)}
        </span>
      )}
      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 }}>{event.title}</span>
    </button>
  );
}

/** "9a", "2:30p"; a 24-hour locale keeps its own clock ("14:30"). */
export function shortTime(ms: number): string {
  const s = new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (!/[AP]M/i.test(s)) return s;
  return s.replace(':00', '').replace(/\s?([AP])M$/i, (_m, c: string) => c.toLowerCase());
}

const dayNum = (ms: number) => new Date(ms).getDate();
const weekdayShort = (ms: number) => new Date(ms).toLocaleDateString([], { weekday: 'short' });

function DayNumber({ ms, isToday, dim, onClick, label }: {
  ms: number; isToday: boolean; dim?: boolean; onClick: () => void; label?: string;
}) {
  const { colors } = useTheme();
  return (
    <button
      type="button"
      className="pa-btn pa-btn--composite"
      onClick={onClick}
      aria-label={label ?? new Date(ms).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })}
      aria-current={isToday ? 'date' : undefined}
      style={{
        ...btnVars(colors),
        '--pa-btn-radius': `${radius.pill}px`,
        '--pa-btn-bg': isToday ? colors.cyan : 'transparent',
        '--pa-btn-fg': isToday ? colors.textOnCyan : dim ? colors.textDim : colors.text,
        '--pa-btn-bg-hover': isToday ? colors.cyan : colors.fillHover,
        '--pa-btn-fg-hover': isToday ? colors.textOnCyan : colors.text,
        minWidth: 20, height: 20, justifyContent: 'center',
        fontFamily: font.body, fontSize: textSize.micro, fontWeight: isToday ? 700 : 500,
        ...tabularNums,
      } as CSSProperties}
    >
      {dayNum(ms)}
    </button>
  );
}

// ── Month ─────────────────────────────────────────────────────────────────

export function MonthView({ anchorMs, nowMs, firstDay, events, onPickEvent, onPickDay }: {
  anchorMs: number; nowMs: number; firstDay: number; events: readonly CalEvent[];
  onPickEvent: PickEvent; onPickDay: (dayMs: number) => void;
}) {
  const { colors } = useTheme();
  const cells = monthGrid(anchorMs, nowMs, firstDay);
  const head = weekDays(cells[0].dayMs, firstDay);
  return (
    <div data-testid="calendar-month" style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, minmax(0, 1fr))' }}>
        {head.map(d => (
          <div key={d} style={{
            fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
            textTransform: 'uppercase', color: colors.textDim, padding: `${space.xs}px ${space.sm}px`,
          }}>{weekdayShort(d)}</div>
        ))}
      </div>
      <div style={{
        display: 'grid', gridTemplateColumns: 'repeat(7, minmax(0, 1fr))',
        gridAutoRows: MONTH_ROW_PX,
        borderTop: `1px solid ${colors.border}`, borderLeft: `1px solid ${colors.border}`,
        borderRadius: radius.sm, overflow: 'hidden',
      }}>
        {cells.map(cell => (
          <MonthCellView key={cell.dayMs} cell={cell} events={events}
            onPickEvent={onPickEvent} onPickDay={onPickDay} />
        ))}
      </div>
    </div>
  );
}

function MonthCellView({ cell, events, onPickEvent, onPickDay }: {
  cell: MonthCell; events: readonly CalEvent[]; onPickEvent: PickEvent; onPickDay: (d: number) => void;
}) {
  const { colors } = useTheme();
  const { shown, hidden } = monthDayChips(events, cell.dayMs, 3);
  return (
    <div
      data-testid="calendar-month-cell"
      data-day={cell.dayMs}
      data-today={cell.isToday ? 'true' : undefined}
      style={{
        borderRight: `1px solid ${colors.border}`, borderBottom: `1px solid ${colors.border}`,
        background: cell.inMonth ? 'transparent' : colors.fillSubtle,
        padding: space.xs, minWidth: 0, overflow: 'hidden',
        display: 'flex', flexDirection: 'column', gap: space.xxs,
      }}
    >
      <DayNumber ms={cell.dayMs} isToday={cell.isToday} dim={!cell.inMonth} onClick={() => onPickDay(cell.dayMs)} />
      {shown.map(e => <EventChip key={eventKey(e)} event={e} onPick={onPickEvent} />)}
      {hidden > 0 && (
        <button
          type="button"
          className="pa-btn pa-btn--composite"
          data-testid="calendar-more"
          onClick={() => onPickDay(cell.dayMs)}
          aria-label={`${hidden} more events, open this day`}
          style={{
            ...btnVars(colors), justifyContent: 'flex-start', paddingInline: space.sm,
            height: CHIP_PX - space.xs, fontFamily: font.body, fontSize: textSize.micro,
            color: colors.textMuted, '--pa-btn-fg': colors.textMuted,
          } as CSSProperties}
        >
          +{hidden} more
        </button>
      )}
    </div>
  );
}

// ── Week / Day: the time grid ─────────────────────────────────────────────

function hourLabel(h: number): string {
  const d = new Date(2000, 0, 1, h);
  return d.toLocaleTimeString([], { hour: 'numeric' }).replace(/\s/g, '').toLowerCase();
}

function TimeGrid({ days, nowMs, events, onPickEvent, scrollKey }: {
  days: number[]; nowMs: number; events: readonly CalEvent[]; onPickEvent: PickEvent; scrollKey: string;
}) {
  const { colors } = useTheme();
  const scroller = useRef<HTMLDivElement>(null);
  const shownToday = days.some(d => sameDay(d, nowMs));
  // Open on the working day: an hour before now when today is on screen,
  // otherwise an hour before the first event, otherwise 8am.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const sod = startOfDay(nowMs);
    let focusMin = 8 * 60;
    if (shownToday) focusMin = (nowMs - sod) / 60_000;
    else {
      const firstTimed = events.filter(e => !e.allDay && days.some(d => sameDay(d, e.startMs)))
        .map(e => (e.startMs - startOfDay(e.startMs)) / 60_000).sort((a, b) => a - b)[0];
      if (firstTimed != null) focusMin = firstTimed;
    }
    el.scrollTop = Math.max(0, (focusMin / 60 - 1) * HOUR_PX);
    // Only when the period changes, never on a refresh (it would yank the scroll).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scrollKey]);

  const nowMin = (nowMs - startOfDay(nowMs)) / 60_000;
  return (
    <div
      ref={scroller}
      data-testid="calendar-timegrid"
      style={{ height: TIME_GRID_PX, overflowY: 'auto', overflowX: 'hidden', position: 'relative',
        borderTop: `1px solid ${colors.border}` }}
    >
      <div style={{ display: 'grid', gridTemplateColumns: `${TIME_GUTTER_PX}px repeat(${days.length}, minmax(0, 1fr))`,
        height: 24 * HOUR_PX, position: 'relative' }}>
        <div style={{ position: 'relative' }}>
          {Array.from({ length: 24 }, (_, h) => (
            <div key={h} style={{
              position: 'absolute', top: h * HOUR_PX, right: space.sm, transform: 'translateY(-50%)',
              fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, ...tabularNums,
              display: h === 0 ? 'none' : undefined,
            }}>{hourLabel(h)}</div>
          ))}
        </div>
        {days.map(day => {
          const placed = layoutDay(events, day);
          const isToday = sameDay(day, nowMs);
          return (
            <div key={day} data-testid="calendar-day-col" data-day={day} style={{
              position: 'relative', borderLeft: `1px solid ${colors.border}`,
              background: isToday ? colors.cyanWash : 'transparent',
              backgroundImage: `repeating-linear-gradient(to bottom, transparent 0, transparent ${HOUR_PX - 1}px, ${colors.border} ${HOUR_PX - 1}px, ${colors.border} ${HOUR_PX}px)`,
            }}>
              {placed.map(p => {
                const hue = calendarColor(p.event.calendar, colors);
                const heightPx = Math.max(p.height / 60 * HOUR_PX - 1, 14);
                return (
                  <button
                    key={eventKey(p.event)}
                    type="button"
                    className="pa-btn pa-btn--composite"
                    data-testid="calendar-block"
                    data-event-key={eventKey(p.event)}
                    data-col={p.col}
                    data-cols={p.cols}
                    aria-label={`${p.event.title}, ${clockLabel(p.event.startMs)} to ${clockLabel(p.event.endMs)}`}
                    onClick={e => onPickEvent(p.event, e.currentTarget)}
                    style={{
                      ...btnVars(colors, wash(hue, 34)),
                      '--pa-btn-bg': wash(hue, 20),
                      position: 'absolute',
                      top: p.top / 60 * HOUR_PX,
                      height: heightPx,
                      left: `calc(${(p.col / p.cols) * 100}% + 1px)`,
                      width: `calc(${100 / p.cols}% - 2px)`,
                      flexDirection: 'column', alignItems: 'flex-start', justifyContent: 'flex-start',
                      gap: 0, padding: `${space.xxs}px ${space.sm}px`,
                      borderLeft: `3px solid ${hue}`, overflow: 'hidden',
                      fontFamily: font.body, fontSize: textSize.micro, lineHeight: '14px', textAlign: 'left',
                    } as CSSProperties}
                  >
                    <span style={{ fontWeight: 600, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis',
                      whiteSpace: heightPx < 34 ? 'nowrap' : 'normal' }}>{p.event.title}</span>
                    {heightPx >= 34 && (
                      <span style={{ color: colors.textMuted, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', ...tabularNums }}>
                        {clockLabel(p.event.startMs)}{p.event.location ? ` · ${p.event.location}` : ''}
                      </span>
                    )}
                  </button>
                );
              })}
              {isToday && (
                <div data-testid="calendar-now-line" aria-hidden style={{
                  position: 'absolute', left: 0, right: 0, top: nowMin / 60 * HOUR_PX, height: 2,
                  background: colors.dangerStrong, pointerEvents: 'none', zIndex: 2,
                }}>
                  <span style={{ position: 'absolute', left: -4, top: -3, width: 8, height: 8,
                    borderRadius: radius.pill, background: colors.dangerStrong }} />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function AllDayRow({ days, events, onPickEvent, onPickDay }: {
  days: number[]; events: readonly CalEvent[]; onPickEvent: PickEvent; onPickDay: (d: number) => void;
}) {
  const { colors } = useTheme();
  const per = days.map(d => eventsOnDay(events, d).filter(e => e.allDay));
  if (per.every(list => list.length === 0)) return null;
  return (
    <div data-testid="calendar-allday" style={{
      display: 'grid', gridTemplateColumns: `${TIME_GUTTER_PX}px repeat(${days.length}, minmax(0, 1fr))`,
      borderTop: `1px solid ${colors.border}`, padding: `${space.xs}px 0`,
    }}>
      <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim,
        padding: `0 ${space.sm}px 0 0`, textAlign: 'right' }}>all-day</div>
      {per.map((list, i) => (
        <div key={days[i]} style={{ display: 'flex', flexDirection: 'column', gap: space.xxs,
          paddingInline: space.xxs, minWidth: 0 }}>
          {list.slice(0, 2).map(e => <EventChip key={eventKey(e)} event={e} onPick={onPickEvent} />)}
          {list.length > 2 && (
            <button type="button" className="pa-btn pa-btn--composite" onClick={() => onPickDay(days[i])}
              aria-label={`${list.length - 2} more all-day events, open this day`}
              style={{ ...btnVars(colors), justifyContent: 'flex-start', paddingInline: space.sm,
                fontFamily: font.body, fontSize: textSize.micro, '--pa-btn-fg': colors.textMuted } as CSSProperties}>
              +{list.length - 2} more
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

export function WeekView({ anchorMs, nowMs, firstDay, events, onPickEvent, onPickDay }: {
  anchorMs: number; nowMs: number; firstDay: number; events: readonly CalEvent[];
  onPickEvent: PickEvent; onPickDay: (dayMs: number) => void;
}) {
  const { colors } = useTheme();
  const days = weekDays(anchorMs, firstDay);
  return (
    <div data-testid="calendar-week" style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
      <div style={{ display: 'grid', gridTemplateColumns: `${TIME_GUTTER_PX}px repeat(7, minmax(0, 1fr))`,
        paddingBottom: space.xs }}>
        <div />
        {days.map(d => {
          const today = sameDay(d, nowMs);
          return (
            <div key={d} style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: space.sm }}>
              <span style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
                textTransform: 'uppercase', color: today ? colors.cyan : colors.textDim }}>{weekdayShort(d)}</span>
              <DayNumber ms={d} isToday={today} onClick={() => onPickDay(d)} />
            </div>
          );
        })}
      </div>
      <AllDayRow days={days} events={events} onPickEvent={onPickEvent} onPickDay={onPickDay} />
      <TimeGrid days={days} nowMs={nowMs} events={events} onPickEvent={onPickEvent} scrollKey={String(days[0])} />
    </div>
  );
}

// ── Day: timeline + list beside it ───────────────────────────────────────

export function EventListRow({ event, onPick }: { event: CalEvent; onPick: PickEvent }) {
  const { colors } = useTheme();
  const hue = calendarColor(event.calendar, colors);
  return (
    <button
      type="button"
      className="pa-btn pa-btn--composite"
      data-testid="calendar-list-row"
      data-event-key={eventKey(event)}
      onClick={e => onPick(event, e.currentTarget)}
      style={{
        ...btnVars(colors), width: '100%', justifyContent: 'flex-start', alignItems: 'flex-start',
        gap: space.lg, padding: `${space.md}px`, textAlign: 'left',
        '--pa-btn-pad': `${space.md}px`, '--pa-btn-radius': `${radius.sm}px`,
      } as CSSProperties}
    >
      <span aria-hidden style={{ width: 3, alignSelf: 'stretch', borderRadius: radius.pill, background: hue, flexShrink: 0 }} />
      <span style={{ display: 'flex', flexDirection: 'column', gap: space.xxs, minWidth: 0, flex: 1 }}>
        <span style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {event.title}
        </span>
        <span style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, ...tabularNums }}>
          {event.allDay ? 'All day' : `${clockLabel(event.startMs)} – ${clockLabel(event.endMs)}`}
          {event.location ? ` · ${event.location}` : ''}
        </span>
        {event.calendar && (
          <span style={{ fontFamily: font.body, fontSize: textSize.micro, color: hue }}>{event.calendar}</span>
        )}
      </span>
    </button>
  );
}

export function DayView({ anchorMs, nowMs, events, onPickEvent, onPickDay, wide }: {
  anchorMs: number; nowMs: number; events: readonly CalEvent[];
  onPickEvent: PickEvent; onPickDay: (dayMs: number) => void; wide: boolean;
}) {
  const { colors } = useTheme();
  const day = startOfDay(anchorMs);
  const list = eventsOnDay(events, day);
  return (
    <div data-testid="calendar-dayview" style={{
      display: 'grid', gridTemplateColumns: wide ? 'minmax(0, 1.4fr) minmax(0, 1fr)' : 'minmax(0, 1fr)',
      gap: space.xxl, minWidth: 0,
    }}>
      <div style={{ minWidth: 0 }}>
        <AllDayRow days={[day]} events={events} onPickEvent={onPickEvent} onPickDay={onPickDay} />
        <TimeGrid days={[day]} nowMs={nowMs} events={events} onPickEvent={onPickEvent} scrollKey={String(day)} />
      </div>
      <div data-testid="calendar-daylist" style={{ minWidth: 0, display: 'flex', flexDirection: 'column', gap: space.xxs }}>
        <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
          textTransform: 'uppercase', color: colors.textDim, paddingBottom: space.xs }}>
          {list.length === 0 ? 'Nothing scheduled' : `${list.length} ${list.length === 1 ? 'event' : 'events'}`}
        </div>
        {list.map(e => <EventListRow key={eventKey(e)} event={e} onPick={onPickEvent} />)}
      </div>
    </div>
  );
}
