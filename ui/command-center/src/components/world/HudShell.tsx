import { useEffect, useState, type CSSProperties } from 'react';
import { COLORS } from './constants';
import { radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

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
  statusPill?: React.ReactNode;
  tabs?: HudTab[];
  activeTab?: string;
  onTabChange?: (id: string) => void;
  children: React.ReactNode;
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
  // The HUD chrome paints from the world palette, not the app theme, so every
  // colour below is still a `COLORS`/hex value — `colors` is here only because
  // the button primitive takes a theme for its variant defaults.
  const { colors } = useTheme();

  // ESC to close — capture phase
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

  return (
    <div style={panelStyle} onClick={(e) => e.stopPropagation()}>
      {/* Header */}
      <div style={headerStyle}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontWeight: 600, letterSpacing: '0.05em' }}>
            {title}
          </span>
          {statusPill}
        </div>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          onClick={onClose}
          title="Close (ESC)"
          aria-label="Close"
          style={closeBtnVars}
        >✕</Button>
      </div>

      {/* Tab bar — only when tabs prop provided */}
      {tabs && tabs.length > 0 && (
        <div style={tabBarStyle}>
          {tabs.map((tab, idx) => {
            const isActive = tab.id === activeTab;
            const isDisabled = !!tab.disabled;
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
                  ...tabBtnVars,
                  '--pa-btn-pad': idx === 0 ? '6px 10px 5px 0' : '6px 10px 5px',
                  '--pa-btn-fg': isActive
                    ? tab.accentColor
                    : isDisabled
                      ? '#4B5563'
                      : '#6B7280',
                  '--pa-btn-fg-hover': isActive ? tab.accentColor : COLORS.primaryMarble,
                  // Only the underline is drawn — `.pa-btn`'s `border` shorthand
                  // paints all four edges, so this longhand has to stay inline.
                  borderBottom: isActive
                    ? `2px solid ${tab.accentColor}`
                    : '2px solid transparent',
                } as CSSProperties}
              >
                {tab.label}
                {isDisabled && tab.disabledLabel && (
                  <span style={{
                    fontSize: 10,
                    marginLeft: 4,
                    color: '#4B5563',
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

      {/* Body */}
      {children}
    </div>
  );
}

// ── Shared sub-components (exported for reuse by HUD bodies) ─────

export function Section({ title, trimColor, children }: {
  title: string;
  trimColor: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ padding: '0 14px 8px' }}>
      <div style={{
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: '0.1em',
        color: trimColor,
        borderBottom: `1px solid ${trimColor}30`,
        paddingBottom: 3,
        marginBottom: 6,
      }}>
        {title}
      </div>
      {children}
    </div>
  );
}

export function StatRow({ label, value }: { label: string; value: string | number }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, lineHeight: 1.6 }}>
      <span style={{ color: '#9CA3AF' }}>{label}</span>
      <span style={{ color: COLORS.primaryMarble, fontWeight: 500 }}>{String(value)}</span>
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

// ── Styles ───────────────────────────────────────────────────────

const panelStyle: React.CSSProperties = {
  position: 'absolute',
  top: 16,
  left: 16,
  width: 300,
  background: 'rgba(10, 14, 26, 0.88)',
  backdropFilter: 'blur(12px)',
  border: `1px solid ${COLORS.marbleVeining}25`,
  borderRadius: radius.md,
  fontFamily: 'monospace',
  color: COLORS.primaryMarble,
  zIndex: 20,
  pointerEvents: 'auto',
  overflow: 'hidden',
};

const headerStyle: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
  padding: '10px 14px 6px',
  fontSize: 13,
  color: COLORS.primaryMarble,
  borderBottom: `1px solid ${COLORS.marbleVeining}20`,
  marginBottom: 0,
};

const closeBtnVars = {
  '--pa-btn-fg': '#6B7280',
  '--pa-btn-fg-hover': COLORS.primaryMarble,
  '--pa-btn-bg-hover': 'rgba(255,255,255,0.06)',
  '--pa-btn-pad': '2px 4px',
  '--pa-btn-radius': `${radius.xs}px`,
  fontSize: 14,
  lineHeight: 1,
} as CSSProperties;

const tabBarStyle: React.CSSProperties = {
  display: 'flex',
  gap: 0,
  padding: '0 14px',
  borderBottom: `1px solid ${COLORS.marbleVeining}15`,
  marginBottom: 4,
};

const tabBtnVars = {
  '--pa-btn-bg': 'transparent',
  '--pa-btn-bg-hover': 'transparent',
  '--pa-btn-bg-active': 'transparent',
  '--pa-btn-border': 'transparent',
  '--pa-btn-border-hover': 'transparent',
  '--pa-btn-radius': '0',
  '--pa-btn-weight': 700,
  fontFamily: 'monospace',
  fontSize: 10,
  letterSpacing: '0.08em',
  lineHeight: 1,
} as CSSProperties;
