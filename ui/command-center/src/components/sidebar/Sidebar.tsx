import { useState, useEffect, useCallback, useRef } from 'react';
import { useCommandCenter } from '../../lib/store';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { markAllRead, toggleTray, useNotifications, useTrayOpen } from '../../lib/notifications';
import { resolveIconPath } from './icons';
import { SidebarTooltip, useSidebarTooltip } from './SidebarTooltip';
import { MeetingRecorder } from '../voice/MeetingRecorder';

const SETTINGS_ICON = 'M12 9a3 3 0 100 6 3 3 0 000-6zM19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09A1.65 1.65 0 008 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06A1.65 1.65 0 004.6 15a1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06A1.65 1.65 0 009 4.6a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9c.16.55.62.96 1.18 1H21a2 2 0 110 4h-.09c-.6.04-1.06.45-1.51 1z';

// "History" glyph (clock + counter-clockwise arrow) — opens the Sessions
// overlay to return to a past conversation.
/** Console — terminal prompt (`>_`): the ops surface for Sessions / Inbox /
 *  Trace / Governance. The row previously reused the Sessions history-clock
 *  icon, which stopped being contextually right after the consolidation. */
const CONSOLE_ICON = 'M4 17l6-5-6-5M12 19h8';

const BELL_ICON = 'M18 8a6 6 0 10-12 0c0 7-3 9-3 9h18s-3-2-3-9M13.7 21a2 2 0 01-3.4 0';

/** Notifications row — a standard sidebar row (above Settings) that toggles
 *  the tray rendered by NotificationHost; open state is shared through the
 *  notifications store since the two live in different subtrees. The unread
 *  count rides the label when expanded and a dot marks the icon when collapsed. */
function NotificationBellRow({ open, onHover, onLeave }: {
  open: boolean;
  onHover?: (el: HTMLElement | null, label: string, shortcut?: string) => void;
  onLeave?: () => void;
}) {
  const { colors } = useTheme();
  const { unread } = useNotifications();
  const trayOpen = useTrayOpen();
  return (
    <div data-notifications-ui style={{ position: 'relative' }}>
      <SidebarRow
        icon={BELL_ICON}
        label={unread > 0 ? `Notifications (${unread > 9 ? '9+' : unread})` : 'Notifications'}
        active={trayOpen}
        open={open}
        onHover={onHover}
        onLeave={onLeave}
        onClick={() => {
          // Mark read on CLOSE so the unread highlight is visible while open.
          if (trayOpen) markAllRead();
          toggleTray();
        }}
      />
      {unread > 0 && (
        <span style={{
          position: 'absolute', top: 7,
          left: open ? 24 : 'calc(50% + 5px)',
          width: 8, height: 8, borderRadius: 4,
          background: colors.cyan, pointerEvents: 'none',
        }} />
      )}
    </div>
  );
}

interface SidebarRowProps {
  icon: string;
  label: string;
  active: boolean;
  open: boolean;
  onClick: () => void;
  /** Keyboard hint shown in the tooltip (e.g. "\u23181"). */
  shortcut?: string;
  onHover?: (el: HTMLElement | null, label: string, shortcut?: string) => void;
  onLeave?: () => void;
}

function SidebarRow({
  icon, label, active, open, onClick, shortcut, onHover, onLeave,
}: SidebarRowProps) {
  const { colors, reduceMotion } = useTheme();
  const [hovered, setHovered] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);

  // Hover reads as a lift toward the active state rather than a separate
  // colour: the row you are pointing at should look like a preview of what
  // selecting it gives you.
  const background = active
    ? colors.cyanSoft
    : hovered ? colors.borderHi : 'transparent';
  const color = active || hovered ? colors.cyan : colors.textMuted;

  return (
    <button
      ref={ref}
      onClick={onClick}
      onMouseEnter={() => { setHovered(true); onHover?.(ref.current, label, shortcut); }}
      onMouseLeave={() => { setHovered(false); onLeave?.(); }}
      onFocus={() => { setHovered(true); onHover?.(ref.current, label, shortcut); }}
      onBlur={() => { setHovered(false); onLeave?.(); }}
      // The accessible name must survive the collapsed rail, where the text
      // label is not rendered at all. The visual tooltip is decoration; this
      // is what a screen reader announces.
      aria-label={shortcut ? `${label} (${shortcut})` : label}
      aria-current={active ? 'page' : undefined}
      style={{
        position: 'relative',
        width: open ? 'calc(100% - 16px)' : 40,
        height: 40, borderRadius: 10,
        display: 'flex', alignItems: 'center', gap: 12,
        padding: open ? '0 12px' : 0,
        justifyContent: open ? 'flex-start' : 'center',
        margin: open ? '0 8px' : '0 auto',
        background,
        border: `1px solid ${active ? colors.borderHi : 'transparent'}`,
        color,
        cursor: 'pointer',
        transition: reduceMotion ? 'none' : `background 160ms ${ease.out}, color 160ms ${ease.out}, border-color 160ms ${ease.out}`,
        fontFamily: font.body, fontSize: 13, fontWeight: active ? 600 : 500,
        textAlign: 'left',
      }}
    >
      {/* Active rail marker. In the collapsed rail the background tint alone is
          easy to miss at a glance, so selection also gets an edge indicator. */}
      <span
        aria-hidden
        style={{
          position: 'absolute', left: -8, top: '50%',
          width: 3, height: active ? 18 : 0,
          marginTop: active ? -9 : 0,
          borderRadius: 2, background: colors.cyan,
          opacity: active ? 1 : 0,
          transition: reduceMotion ? 'none' : `height 180ms ${ease.out}, opacity 180ms ${ease.out}, margin-top 180ms ${ease.out}`,
        }}
      />
      <svg
        width="18" height="18" viewBox="0 0 24 24" fill="none"
        style={{
          flexShrink: 0,
          transform: hovered && !reduceMotion ? 'scale(1.09)' : 'scale(1)',
          transition: reduceMotion ? 'none' : `transform 160ms ${ease.out}`,
        }}
        stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round"
      >
        <path d={icon} />
      </svg>
      {open && <span style={{
        opacity: 1, transition: 'opacity 160ms', whiteSpace: 'nowrap',
      }}>{label}</span>}
    </button>
  );
}

export function Sidebar() {
  const { gradient, colors } = useTheme();
  const [open, setOpen] = useState(true);
  const workspaces = useCommandCenter(s => s.workspaces);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const activePanel = useCommandCenter(s => s.activePanel);
  const switchWorkspace = useCommandCenter(s => s.switchWorkspace);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  const { target: tooltipTarget, show: showTooltip, hide: hideTooltip } = useSidebarTooltip();

  const isSettingsOpen = activePanel === 'settings';
  const isConsoleOpen = ['sessions', 'inbox', 'trace', 'governance'].includes(activePanel);
  // Any non-chat panel (settings, inbox, skills, sessions) is a full-screen overlay. It
  // must be dismissed when the user picks a workspace, or the overlay stays
  // stuck over the tab they just selected.
  const overlayOpen = activePanel !== 'chat';
  const W = open ? 208 : 64;

  const goToWorkspace = useCallback((workspaceId: string) => {
    switchWorkspace(workspaceId);
    if (overlayOpen) {
      setActivePanel('chat');
    }
  }, [switchWorkspace, setActivePanel, overlayOpen]);

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
      borderRight: `1px solid ${colors.border}`,
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
        const isActive = activeWorkspaceId === ws.id && !overlayOpen;
        // By NAME, not by the stored icon key — that is what lets the corrected
        // glyphs reach profiles seeded before the rename. See icons.ts.
        const iconPath = resolveIconPath(ws.name, ws.icon);
        const shortcut = i < 9
          ? (navigator.platform.includes('Mac') ? `⌘${i + 1}` : `Ctrl+${i + 1}`)
          : undefined;
        return (
          <SidebarRow
            key={ws.id}
            icon={iconPath}
            label={ws.name}
            active={isActive}
            open={open}
            onClick={() => goToWorkspace(ws.id)}
            shortcut={shortcut}
            onHover={showTooltip}
            onLeave={hideTooltip}
          />
        );
      })}

      <div style={{ flex: 1 }} />

      {/* Meeting dictation (call-notes MVP 1A) — click-to-toggle Record button
          (NOT push-to-talk: PTT dies when the embedded webview holds focus).
          Lives here because the Sidebar never unmounts, so a recording
          survives workspace and overlay switches.

          Deleted as collateral in the 2026-07-28 mega-session (065d617ed) and
          restored 2026-08-04. It is the only surface that turns a call into a
          project note, and the self-knowledge descriptor never stopped telling
          Henry it existed — so for a week he pointed users at a button that
          was not there. */}
      <MeetingRecorder open={open} />

      {/* Console — Sessions, Inbox, Trace, and Governance as tabs of one
          overlay (2026-07-27 consolidation). Toggles closed when any console
          tab is open; opens on Sessions otherwise. */}
      <SidebarRow
        icon={CONSOLE_ICON}
        label="Console"
        active={isConsoleOpen}
        open={open}
        onClick={() => setActivePanel(isConsoleOpen ? 'chat' : 'sessions')}
        onHover={showTooltip}
        onLeave={hideTooltip}
      />

      {/* Notifications — bell row with unread badge, tray anchors beside it. */}
      <NotificationBellRow open={open} onHover={showTooltip} onLeave={hideTooltip} />

      {/* Settings */}
      <SidebarRow
        icon={SETTINGS_ICON}
        label="Settings"
        active={isSettingsOpen}
        open={open}
        onClick={() => setActivePanel(isSettingsOpen ? 'chat' : 'settings')}
        onHover={showTooltip}
        onLeave={hideTooltip}
      />

      {/* Collapse / Expand toggle */}
      {open ? (
        <button onClick={() => setOpen(false)} title="Collapse" style={{
          width: 'calc(100% - 16px)', height: 32, borderRadius: 8,
          margin: '4px 8px 0', background: 'transparent',
          border: 'none', color: colors.textDim, cursor: 'pointer',
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
          border: 'none', color: colors.textDim, cursor: 'pointer',
          display: 'grid', placeItems: 'center',
        }}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M9 6l6 6-6 6" />
            <path d="M3 4v16" opacity={0.4} />
          </svg>
        </button>
      )}

      {/* Portalled to document.body — the rail sets overflow:hidden to animate
          its width, which would clip a tooltip rendered inside it. */}
      <SidebarTooltip target={tooltipTarget} />
    </div>
  );
}
