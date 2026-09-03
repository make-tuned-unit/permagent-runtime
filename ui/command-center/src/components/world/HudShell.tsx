import { useEffect, useState, type CSSProperties, type ReactNode } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { Button } from '../common/Button';
import {
  HUD_GEOM,
  HUD_INNER_RADIUS,
  HUD_PANEL_RADIUS,
  hudBareVars,
  hudCaption,
  hudTransition,
} from './hudChrome';

import { Tooltip } from '../common/Tooltip';
// ── Tab definition ────────────────────────────────────────────────

export interface HudTab {
  id: string;
  label: string;
  accentColor: string;
  disabled?: boolean;
  disabledLabel?: string;
}

// ── Props ─────────────────────────────────────────────────────────

interface HudShellProps {
  visible: boolean;
  onClose: () => void;
  title: string;
  statusPill?: ReactNode;
  tabs?: HudTab[];
  activeTab?: string;
  onTabChange?: (id: string) => void;
  children: ReactNode;
}

// ── Component ─────────────────────────────────────────────────────

export function HudShell({
  visible,
  onClose,
  title,
  statusPill,
  tabs,
  activeTab,
  onTabChange,
  children,
}: HudShellProps) {
  // Floating control over the 3D canvas — glass is correct here (D1). One plane
  // for the whole panel; tabs/close use fillHover, never a second filter (D2).
  const { colors, reduceMotion } = useTheme();
  const glass = useGlass('glass');

  useEffect(() => {
    if (!visible) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopImmediatePropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [visible, onClose]);

  if (!visible) return null;

  const panelStyle: CSSProperties = {
    position: 'absolute',
    top: HUD_GEOM.panelInset,
    left: HUD_GEOM.panelInset,
    width: HUD_GEOM.panelWidth,
    ...glass,
    border: `1px solid ${colors.border}`,
    borderRadius: HUD_PANEL_RADIUS,
    fontFamily: font.mono,
    color: colors.text,
    zIndex: 20,
    pointerEvents: 'auto',
    overflow: 'hidden',
  };

  const headerStyle: CSSProperties = {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    padding: `${HUD_GEOM.headerPadTop}px ${HUD_GEOM.panelPadX}px ${HUD_GEOM.headerPadBottom}px`,
    fontSize: textSize.small,
    color: colors.text,
    borderBottom: `1px solid ${colors.border}`,
    marginBottom: 0,
  };

  const closeVars = {
    ...hudBareVars(colors, {
      radiusPx: HUD_INNER_RADIUS > 0 ? HUD_INNER_RADIUS : radius.xs,
    }),
    fontSize: textSize.body,
    lineHeight: 1,
    transition: hudTransition(reduceMotion),
  } as CSSProperties;

  const tabBarStyle: CSSProperties = {
    display: 'flex',
    gap: 0,
    padding: `0 ${HUD_GEOM.panelPadX}px`,
    borderBottom: `1px solid ${colors.border}`,
    marginBottom: space.xs,
  };

  return (
    <div style={panelStyle} onClick={(e) => e.stopPropagation()}>
      <div style={headerStyle}>
        <div style={{ display: 'flex', alignItems: 'center', gap: space.lg }}>
          <span style={{ fontWeight: 600, letterSpacing: '0.05em' }}>
            {title}
          </span>
          {statusPill}
        </div>
        <Tooltip content="Close (ESC)">
          <Button
            colors={colors}
            variant="bare"
            type="button"
            onClick={onClose}
            aria-label="Close"
            style={closeVars}
          >✕</Button>
        </Tooltip>
      </div>

      {tabs && tabs.length > 0 && (
        <div style={tabBarStyle}>
          {tabs.map((tab, idx) => {
            const isActive = tab.id === activeTab;
            const isDisabled = !!tab.disabled;
            const padLeft = idx === 0 ? 0 : HUD_GEOM.tabPadX;
            return (
              <Button
                key={tab.id}
                colors={colors}
                variant="bare"
                type="button"
                onClick={() => {
                  if (!isDisabled && onTabChange) onTabChange(tab.id);
                }}
                disabled={isDisabled}
                flashSuccess={false}
                style={{
                  ...hudBareVars(colors, {
                    fg: isActive
                      ? tab.accentColor
                      : isDisabled
                        ? colors.textDim
                        : colors.textMuted,
                    fgHover: isActive ? tab.accentColor : colors.text,
                    // Underline-only tabs: fill stays transparent; hover still
                    // lifts via fg (D10) without a second glass plane.
                    bg: 'transparent',
                    pad: `${HUD_GEOM.tabPadY}px ${HUD_GEOM.tabPadX}px ${HUD_GEOM.tabPadY - 1}px ${padLeft}px`,
                    radiusPx: 0,
                    weight: 700,
                  }),
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-bg-active': 'transparent',
                  fontSize: textSize.micro,
                  letterSpacing: '0.08em',
                  lineHeight: 1,
                  transition: hudTransition(reduceMotion),
                  borderBottom: isActive
                    ? `2px solid ${tab.accentColor}`
                    : '2px solid transparent',
                } as CSSProperties}
              >
                {tab.label}
                {isDisabled && tab.disabledLabel && (
                  <span style={{
                    ...hudCaption,
                    marginLeft: space.xs,
                    color: colors.textDim,
                    fontWeight: 400,
                    letterSpacing: '0.04em',
                  }}>
                    {tab.disabledLabel}
                  </span>
                )}
              </Button>
            );
          })}
        </div>
      )}

      {children}
    </div>
  );
}

// ── Shared sub-components (exported for reuse by HUD bodies) ─────

export function Section({ title, trimColor, children }: {
  title: string;
  trimColor: string;
  children: ReactNode;
}) {
  return (
    <div style={{ padding: `0 ${HUD_GEOM.panelPadX}px ${HUD_GEOM.bodyPadY}px` }}>
      <div style={{
        fontSize: textSize.micro,
        fontWeight: 700,
        letterSpacing: '0.1em',
        color: trimColor,
        borderBottom: `1px solid ${trimColor}30`,
        paddingBottom: 3,
        marginBottom: HUD_GEOM.sectionGap,
      }}>
        {title}
      </div>
      {children}
    </div>
  );
}

export function StatRow({ label, value }: { label: string; value: string | number }) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: textSize.micro, lineHeight: 1.6 }}>
      <span style={{ color: colors.textMuted }}>{label}</span>
      <span style={{ color: colors.text, fontWeight: 500 }}>{String(value)}</span>
    </div>
  );
}

// ── Hook: reset tab on open ──────────────────────────────────────

export function useTabReset(visible: boolean, defaultTab: string): [string, (id: string) => void] {
  const [tab, setTab] = useState(defaultTab);
  useEffect(() => {
    if (visible) setTab(defaultTab);
  }, [visible, defaultTab]);
  return [tab, setTab];
}
