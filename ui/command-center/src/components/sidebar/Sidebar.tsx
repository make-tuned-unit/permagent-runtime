import { useState, useEffect, useCallback } from 'react';
import { useCommandCenter } from '../../lib/store';
import { color, font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';

/** SVG icon paths from design handoff (view-dashboard.jsx lines 17-20, 107) */
const ICON_PATHS: Record<string, string> = {
  home: 'M3 11l9-8 9 8v9a2 2 0 01-2 2h-4v-7h-6v7H5a2 2 0 01-2-2v-9z',
  'layout-dashboard': 'M4 7h16M4 12h16M4 17h10',
  globe: 'M12 2a10 10 0 100 20 10 10 0 000-20zM2 12h20M12 2a15 15 0 014 10 15 15 0 01-4 10M12 2a15 15 0 00-4 10 15 15 0 004 10',
  code: 'M4 4h7v7H4zM13 4h7v4h-7zM13 10h7v10h-7zM4 13h7v7H4z',
  brain: 'M9 4a4 4 0 00-4 4 3 3 0 00-1 5.5A3 3 0 005 18a4 4 0 004 3M15 4a4 4 0 014 4 3 3 0 011 5.5A3 3 0 0119 18a4 4 0 01-4 3M9 4a3 3 0 013 3v14M15 4a3 3 0 00-3 3',
};

const SETTINGS_ICON = 'M12 9a3 3 0 100 6 3 3 0 000-6zM19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09A1.65 1.65 0 008 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06A1.65 1.65 0 004.6 15a1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06A1.65 1.65 0 009 4.6a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9c.16.55.62.96 1.18 1H21a2 2 0 110 4h-.09c-.6.04-1.06.45-1.51 1z';

function SidebarRow({
  icon, label, active, open, onClick, title,
}: {
  icon: string; label: string; active: boolean; open: boolean;
  onClick: () => void; title?: string;
}) {
  return (
    <button onClick={onClick} title={open ? title : label} style={{
      width: open ? 'calc(100% - 16px)' : 40,
      height: 40, borderRadius: 10,
      display: 'flex', alignItems: 'center', gap: 12,
      padding: open ? '0 12px' : 0,
      justifyContent: open ? 'flex-start' : 'center',
      margin: open ? '0 8px' : '0 auto',
      background: active ? 'rgba(0,213,255,0.10)' : 'transparent',
      border: active ? `1px solid ${color.borderHi}` : '1px solid transparent',
      color: active ? color.cyan : color.textMuted,
      cursor: 'pointer', transition: `all 200ms ${ease.out}`,
      fontFamily: font.body, fontSize: 13, fontWeight: active ? 600 : 500,
      textAlign: 'left',
    }}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0 }}
        stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round">
        <path d={icon} />
      </svg>
      {open && <span style={{
        opacity: 1, transition: 'opacity 160ms', whiteSpace: 'nowrap',
      }}>{label}</span>}
    </button>
  );
}

export function Sidebar() {
  const { gradient } = useTheme();
  const [open, setOpen] = useState(true);
  const workspaces = useCommandCenter(s => s.workspaces);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const activePanel = useCommandCenter(s => s.activePanel);
  const switchWorkspace = useCommandCenter(s => s.switchWorkspace);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  const isSettingsOpen = activePanel === 'settings';
  const W = open ? 208 : 64;

  const goToWorkspace = useCallback((workspaceId: string) => {
    switchWorkspace(workspaceId);
    if (isSettingsOpen) {
      setActivePanel('chat');
    }
  }, [switchWorkspace, setActivePanel, isSettingsOpen]);

  // Keyboard shortcuts: Cmd+1..5
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (!e.metaKey && !e.ctrlKey) return;
      const num = parseInt(e.key, 10);
      if (num >= 1 && num <= workspaces.length) {
        e.preventDefault();
        const ws = workspaces[num - 1];
        if (ws) goToWorkspace(ws.id);
      }
    }
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [workspaces, goToWorkspace]);

  return (
    <div style={{
      width: W, height: '100%', flexShrink: 0,
      borderRight: `1px solid ${color.border}`,
      background: gradient.sidebar,
      display: 'flex', flexDirection: 'column',
      padding: '14px 0', gap: 4,
      transition: `width 220ms ${ease.out}`,
      overflow: 'hidden',
    }}>
      {/* Brand row */}
      <div style={{
        display: 'flex', alignItems: 'center',
        padding: open ? '0 14px 14px' : '0 0 14px',
        justifyContent: open ? 'flex-start' : 'center',
      }}>
        <Mobius size={14} state="idle" glow={0.7} />
      </div>

      {/* Workspace items */}
      {workspaces.map((ws, i) => {
        const isActive = activeWorkspaceId === ws.id && !isSettingsOpen;
        const iconPath = ICON_PATHS[ws.icon] || ICON_PATHS.home;
        const shortcut = navigator.platform.includes('Mac') ? `⌘${i + 1}` : `Ctrl+${i + 1}`;
        return (
          <SidebarRow
            key={ws.id}
            icon={iconPath}
            label={ws.name}
            active={isActive}
            open={open}
            onClick={() => goToWorkspace(ws.id)}
            title={`${ws.name} (${shortcut})`}
          />
        );
      })}

      <div style={{ flex: 1 }} />

      {/* Settings */}
      <SidebarRow
        icon={SETTINGS_ICON}
        label="Settings"
        active={isSettingsOpen}
        open={open}
        onClick={() => setActivePanel(isSettingsOpen ? 'chat' : 'settings')}
      />

      {/* Collapse / Expand toggle */}
      {open ? (
        <button onClick={() => setOpen(false)} title="Collapse" style={{
          width: 'calc(100% - 16px)', height: 32, borderRadius: 8,
          margin: '4px 8px 0', background: 'transparent',
          border: 'none', color: color.textDim, cursor: 'pointer',
          display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8,
          fontFamily: font.body, fontSize: 11, fontWeight: 500,
          transition: `all 200ms ${ease.out}`,
        }}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M15 6l-6 6 6 6" />
            <path d="M21 4v16" opacity={0.4} />
          </svg>
          Collapse
        </button>
      ) : (
        <button onClick={() => setOpen(true)} title="Expand" style={{
          width: 40, height: 32, margin: '4px auto 0',
          borderRadius: 8, background: 'transparent',
          border: 'none', color: color.textDim, cursor: 'pointer',
          display: 'grid', placeItems: 'center',
        }}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M9 6l6 6-6 6" />
            <path d="M3 4v16" opacity={0.4} />
          </svg>
        </button>
      )}
    </div>
  );
}
