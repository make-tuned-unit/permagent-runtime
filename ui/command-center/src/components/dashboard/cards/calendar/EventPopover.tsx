/**
 * The event detail popover: title, when, where, calendar, attendees and a way
 * into Calendar.app. Portalled to <body> and positioned from the clicked
 * element, because the Dashboard cell it would otherwise live in clips
 * (`overflow: hidden`) and a card scrolls.
 *
 * Closes on Escape, on a press outside, and on the Close button; the card puts
 * focus back on the element that opened it.
 */

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { FiX } from 'react-icons/fi';
import { font, radius, space, textSize } from '../../../../styles/tokens';
import { useTheme } from '../../../../styles/useTheme';
import { Button } from '../../../common/Button';
import { Tooltip } from '../../../common/Tooltip';
import { calendarColor, whenLabel, type CalEvent } from './calendarModel';

const POPOVER_W = 300;
const EDGE = 8;

/** http(s) links go to the system browser; anything else is not opened. */
export async function openExternal(url: string): Promise<boolean> {
  if (!/^https?:\/\//i.test(url)) return false;
  try {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(url);
  } catch {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
  return true;
}

/** Bring Calendar.app forward (desktop shell only; resolves false elsewhere). */
export async function openCalendarApp(): Promise<boolean> {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return false;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_calendar_app');
    return true;
  } catch {
    return false;
  }
}

export function EventPopover({ event, anchor, onClose }: {
  event: CalEvent; anchor: HTMLElement; onClose: () => void;
}) {
  const { colors } = useTheme();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number }>({ top: 0, left: 0 });
  const hue = calendarColor(event.calendar, colors);
  const inDesktop = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  useLayoutEffect(() => {
    const r = anchor.getBoundingClientRect();
    const h = ref.current?.offsetHeight ?? 200;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let left = r.left + r.width / 2 - POPOVER_W / 2;
    left = Math.min(Math.max(EDGE, left), Math.max(EDGE, vw - POPOVER_W - EDGE));
    let top = r.bottom + EDGE;
    if (top + h > vh - EDGE) top = Math.max(EDGE, r.top - h - EDGE);
    setPos({ top, left });
  }, [anchor, event]);

  useEffect(() => {
    ref.current?.focus({ preventScroll: true });
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node | null;
      if (t && !ref.current?.contains(t) && !anchor.contains(t)) onClose();
    };
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('pointerdown', onDown, true);
    return () => {
      document.removeEventListener('keydown', onKey, true);
      document.removeEventListener('pointerdown', onDown, true);
    };
  }, [anchor, onClose]);

  const row = (label: string, value: string) => (
    <div style={{ display: 'flex', gap: space.lg, fontFamily: font.body, fontSize: textSize.caption, lineHeight: 1.4 }}>
      <span style={{ width: 64, flexShrink: 0, color: colors.textDim }}>{label}</span>
      <span style={{ color: colors.text, minWidth: 0, overflowWrap: 'anywhere' }}>{value}</span>
    </div>
  );
  const linkBtn = {
    '--pa-btn-bg': 'transparent', '--pa-btn-fg': colors.cyan, '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.cyanSoft, '--pa-btn-fg-hover': colors.cyan, '--pa-btn-border-hover': colors.cyan,
    '--pa-btn-bg-active': colors.cyanGlow, '--pa-btn-pad': `${space.sm}px ${space.xl}px`,
    '--pa-btn-radius': `${radius.sm}px`, fontFamily: font.body, fontSize: textSize.caption,
  } as CSSProperties;

  return createPortal(
    <div
      ref={ref}
      role="dialog"
      aria-label={event.title}
      tabIndex={-1}
      data-testid="calendar-popover"
      style={{
        position: 'fixed', top: pos.top, left: pos.left, width: POPOVER_W, zIndex: 130,
        boxSizing: 'border-box', padding: space.xxl, outline: 'none',
        display: 'flex', flexDirection: 'column', gap: space.md,
        background: colors.surfaceHi, border: `1px solid ${colors.borderHi}`,
        borderRadius: radius.lg, boxShadow: colors.elevationOverlay,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: space.lg }}>
        <span aria-hidden style={{ width: 4, alignSelf: 'stretch', borderRadius: radius.pill, background: hue, flexShrink: 0 }} />
        <div style={{ flex: 1, minWidth: 0, fontFamily: font.display, fontSize: textSize.body, fontWeight: 600,
          color: colors.text, overflowWrap: 'anywhere', lineHeight: 1.3 }}>{event.title}</div>
        <Tooltip content="Close" shortcut="Esc">
          <Button colors={colors} variant="bare" type="button" aria-label="Close" onClick={onClose}
            flashSuccess={false} style={{ '--pa-btn-pad': `${space.xs}px`, '--pa-btn-fg': colors.textMuted } as CSSProperties}>
            <FiX size={14} />
          </Button>
        </Tooltip>
      </div>
      {row('When', whenLabel(event))}
      {event.location && row('Where', event.location)}
      {event.calendar && row('Calendar', event.calendar)}
      {event.attendees > 0 && row('Attendees', `${event.attendees} ${event.attendees === 1 ? 'person' : 'people'}`)}
      <div style={{ display: 'flex', gap: space.md, flexWrap: 'wrap', marginTop: space.xs }}>
        {event.url && /^https?:\/\//i.test(event.url) && (
          <Button colors={colors} type="button" style={linkBtn} flashSuccess={false}
            onClick={() => openExternal(event.url as string)}>Open link</Button>
        )}
        {inDesktop && event.source === 'eventkit' && (
          <Button colors={colors} type="button" style={linkBtn} flashSuccess={false}
            onClick={() => openCalendarApp()}>Open in Calendar</Button>
        )}
      </div>
    </div>,
    document.body,
  );
}
