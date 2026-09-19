/**
 * The Calendar card's Apple Calendar access line: what state access is in, and
 * the one control that moves it forward.
 *
 * Exists because of the report "i cant find anywhere to grant access to my
 * iCal so my calendar can sync" — the card used to describe a permission with
 * no control anywhere to grant it. Every state here either carries a button
 * that works (Connect Calendar, Open System Settings) or says plainly why
 * there is none (a browser tab, a restricted Mac).
 */

import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import type { CalendarAccessView } from './calendarAccess';

interface Props {
  view: CalendarAccessView;
  onConnect: () => Promise<boolean>;
  onOpenSettings: () => Promise<boolean>;
  /** Tighter spacing when it sits under a list of events. */
  compact?: boolean;
}

export function CalendarAccessPanel({ view, onConnect, onOpenSettings, compact }: Props) {
  const { colors } = useTheme();
  if (!view.text) return null;

  const onClick = view.action === 'connect' ? onConnect
    : view.action === 'open-settings' ? onOpenSettings
    : undefined;

  return (
    <div
      data-testid="calendar-access"
      data-state={view.kind}
      style={{
        display: 'flex', flexDirection: 'column', alignItems: 'flex-start',
        gap: space.sm,
        paddingTop: compact ? space.md : space.xxs,
        marginTop: compact ? space.sm : 0,
        borderTop: compact ? `1px solid ${colors.border}` : 'none',
      }}
    >
      <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.4 }}>
        {view.text}
      </div>
      {view.hint && (
        <div style={{ fontFamily: font.body, fontSize: 10.5, color: colors.textDim, lineHeight: 1.4 }}>
          {view.hint}
        </div>
      )}
      {onClick && view.actionLabel && (
        // Keyed by action: when "Allow Full Access" comes back write-only the
        // control becomes "Open System Settings". Reusing the same Button
        // would carry the finished request's pending spinner (and its click
        // guard) onto a control nobody has pressed yet.
        <Button
          key={view.action}
          colors={colors}
          type="button"
          onClick={onClick}
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
            fontFamily: font.body, fontSize: textSize.caption,
          } as CSSProperties}
        >
          {view.actionLabel}
        </Button>
      )}
    </div>
  );
}
