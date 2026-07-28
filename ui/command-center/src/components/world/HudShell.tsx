import { useEffect, useState } from 'react';
import { COLORS } from './constants';

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
        <button onClick={onClose} style={closeBtnStyle} title="Close (ESC)">✕</button>
      </div>

      {/* Tab bar — only when tabs prop provided */}
      {tabs && tabs.length > 0 && (
        <div style={tabBarStyle}>
          {tabs.map((tab, idx) => {
            const isActive = tab.id === activeTab;
            const isDisabled = !!tab.disabled;
            return (
              <button
                key={tab.id}
                onClick={() => {
                  if (!isDisabled && onTabChange) onTabChange(tab.id);
                }}
                disabled={isDisabled}
                style={{
                  ...tabBtnBase,
                  ...(idx === 0 ? { paddingLeft: 0 } : {}),
                  color: isActive
                    ? tab.accentColor
                    : isDisabled
                      ? '#4B5563'
                      : '#6B7280',
                  borderBottom: isActive
                    ? `2px solid ${tab.accentColor}`
                    : '2px solid transparent',
                  cursor: isDisabled ? 'default' : 'pointer',
                  opacity: isDisabled ? 0.45 : 1,
                }}
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
              </button>
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
  borderRadius: 8,
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

const closeBtnStyle: React.CSSProperties = {
  background: 'none',
  border: 'none',
  color: '#6B7280',
  cursor: 'pointer',
  fontSize: 14,
  padding: '2px 4px',
  lineHeight: 1,
};

const tabBarStyle: React.CSSProperties = {
  display: 'flex',
  gap: 0,
  padding: '0 14px',
  borderBottom: `1px solid ${COLORS.marbleVeining}15`,
  marginBottom: 4,
};

const tabBtnBase: React.CSSProperties = {
  background: 'none',
  border: 'none',
  fontFamily: 'monospace',
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: '0.08em',
  padding: '6px 10px 5px',
  lineHeight: 1,
};
