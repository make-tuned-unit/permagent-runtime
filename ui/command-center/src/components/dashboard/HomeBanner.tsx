/**
 * The Home banner shell — one chrome, one kicker, one dismiss.
 *
 * Echo and Learn next carried byte-identical style objects for the card, the
 * mono `✦ LABEL` kicker and the ✕ button, written out twice. Two unrelated
 * features that look this alike are read as one feature, and drifting them
 * apart later would have been worse than the duplication. One shell, two
 * banners; `bannerSlot` decides which of them is on screen.
 */

import { type CSSProperties, type ReactNode } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

import { Tooltip } from '../common/Tooltip';
export interface HomeBannerProps {
  /** The word after the ✦, uppercase — "ECHO", "LEARN NEXT". */
  kicker: string;
  /** A quiet reading beside the kicker ("3/9 explored"). */
  meta?: string;
  /** What the banner is, for a screen reader. */
  ariaLabel: string;
  /** Optional decoration to the left of the words. */
  art?: ReactNode;
  /** The banner's own controls, right-aligned. The ✕ is added after them. */
  actions?: ReactNode;
  onDismiss: () => void;
  /** What the ✕ dismisses, for a screen reader. */
  dismissLabel: string;
  'data-testid'?: string;
  children: ReactNode;
}

export function HomeBanner({
  kicker, meta, ariaLabel, art, actions, onDismiss, dismissLabel,
  'data-testid': testId, children,
}: HomeBannerProps) {
  const { colors } = useTheme();
  return (
    <div
      role="note"
      aria-label={ariaLabel}
      data-testid={testId}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 18,
        marginBottom: 20,
        padding: '14px 16px',
        borderRadius: 14,
        background: colors.surface,
        border: `1px solid ${colors.borderHi}`,
        boxShadow: colors.cardShadow,
        overflow: 'hidden',
      }}
    >
      {art}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontFamily: font.mono,
            fontSize: textSize.micro,
            letterSpacing: '0.14em',
            color: colors.textDim,
            marginBottom: 4,
            display: 'flex',
            gap: 10,
            alignItems: 'center',
          }}
        >
          <span>✦ {kicker}</span>
          {meta && <span style={{ color: colors.textMuted }}>{meta}</span>}
        </div>
        <div style={{ fontFamily: font.body, fontSize: textSize.body, color: colors.text, lineHeight: 1.4 }}>
          {children}
        </div>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
        {actions}
        <Tooltip content="Not now">
          <Button
            colors={colors}
            variant="bare"
            type="button"
            onClick={onDismiss}
            aria-label={dismissLabel}
            style={dismissBtn(colors)}
          >
            ✕
          </Button>
        </Tooltip>
      </div>
    </div>
  );
}

/* The banner's three button looks, as `--pa-btn-*` custom properties rather
   than inline `color`/`background`: an inline declaration outranks the
   `:hover` rule and would silently cancel the hover and press states the
   primitive exists to provide. */
export function bannerPrimaryBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': colors.cyanSoft,
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-border': colors.cyan,
    '--pa-btn-bg-hover': colors.cyanSoft,
    '--pa-btn-fg-hover': colors.cyan,
    '--pa-btn-border-hover': colors.cyan,
    '--pa-btn-bg-active': colors.cyanGlow,
    '--pa-btn-pad': '7px 14px',
    '--pa-btn-radius': '9px',
    '--pa-btn-weight': 600,
    fontFamily: font.body,
    fontSize: textSize.caption,
    whiteSpace: 'nowrap',
  } as CSSProperties;
}

export function bannerGhostBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-bg-active': colors.surface,
    '--pa-btn-pad': '7px 12px',
    '--pa-btn-radius': '9px',
    fontFamily: font.body,
    fontSize: textSize.caption,
    whiteSpace: 'nowrap',
  } as CSSProperties;
}

function dismissBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': colors.textDim,
    '--pa-btn-border': 'transparent',
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-active': colors.surface,
    '--pa-btn-pad': '0',
    '--pa-btn-radius': `${radius.md}px`,
    '--pa-btn-weight': 400,
    width: 26,
    height: 26,
    fontSize: textSize.caption,
  } as CSSProperties;
}
