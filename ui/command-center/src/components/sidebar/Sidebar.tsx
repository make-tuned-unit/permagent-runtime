import { useState, useEffect, useCallback, type CSSProperties } from 'react';
import { useCommandCenter } from '../../lib/store';
import type { LayoutNode } from '../../lib/store';
import { ease, font, radius, space, shell, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { FiBell, FiChevronLeft, FiChevronRight, FiClock, FiSettings } from 'react-icons/fi';
import type { IconType } from 'react-icons';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import { Mobius } from '../mobius/Mobius';
import { markAllRead, toggleTray, useNotifications, useTrayOpen } from '../../lib/notifications';
import { resolveIconPath } from '../common/icons';
import { SidebarTooltip, useSidebarTooltip } from './SidebarTooltip';
import { MeetingRecorder } from '../voice/MeetingRecorder';
import { NavStatusBadge, NavStatusLine, useNavAgentStatus } from './NavAgentStatus';

/** The workspace whose layout is a single `dashboard` panel — "Home". Walks
 *  split layouts too, though Home in practice is never split. Identifying it
 *  by tool rather than by name keeps this honest against a renamed workspace
 *  (workspace names, like the agent's, are user-configurable). */
function hasDashboardTool(node: LayoutNode): boolean {
  if (node.type === 'panel') return node.tool === 'dashboard';
  return node.children.some(hasDashboardTool);
}


/** History row — Sessions, Downloads, Activity and Spend.
 *
 *  A record of what your agent did is not a setting, and until this row
 *  existed reading one cost a trip through the configuration screen: the
 *  2026-08 ruling folded the retired Console into Settings because there was
 *  nowhere else to put it, and #1177 made the four one component precisely so
 *  that hoisting them out would be an import and a branch rather than a move.
 *
 *  It carries the same download badge the Downloads segment shows, off the
 *  same notification stream the tray and the toasts read — a file that landed
 *  while you were elsewhere is the one thing here worth a mark on the rail,
 *  and it is not a second poll.
 */
function HistoryRow({ open, active, onOpen, onHover, onLeave }: {
  open: boolean;
  active: boolean;
  onOpen: () => void;
  onHover?: (el: HTMLElement | null, label: string, shortcut?: string) => void;
  onLeave?: () => void;
}) {
  const { colors } = useTheme();
  const { items } = useNotifications();
  const unread = items.filter(n => n.kind === 'download' && !n.read).length;
  return (
    <div style={{ position: 'relative' }}>
      <SidebarRow
        icon={FiClock}
        label={unread > 0 ? `History (${unread > 9 ? '9+' : unread})` : 'History'}
        active={active}
        open={open}
        onHover={onHover}
        onLeave={onLeave}
        onClick={onOpen}
      />
      {unread > 0 && (
        <span style={{
          position: 'absolute', top: 7,
          left: open ? 24 : 'calc(50% + 5px)',
          width: 8, height: 8, borderRadius: radius.xs,
          background: colors.cyan, pointerEvents: 'none',
        }} />
      )}
    </div>
  );
}

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
        icon={FiBell}
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
          width: 8, height: 8, borderRadius: radius.xs,
          background: colors.cyan, pointerEvents: 'none',
        }} />
      )}
    </div>
  );
}

/** A rail glyph: a Feather component for every row except the workspaces,
 *  whose glyphs come from the ratified local set as path data. Both wear the
 *  same 18px box, the same 1.6 stroke and the same hover nudge. */
function RowGlyph({ icon, style }: { icon: string | IconType; style: CSSProperties }) {
  if (typeof icon !== 'string') {
    const Glyph = icon;
    return <Glyph size={18} style={style} />;
  }
  return (
    <svg
      width="18" height="18" viewBox="0 0 24 24" fill="none"
      style={style}
      stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round"
    >
      <path d={icon} />
    </svg>
  );
}

interface SidebarRowProps {
  /** A `Fi*` component, or a path string from the ratified set in
   *  `components/common/icons` (workspace rows resolve to one of those). */
  icon: string | IconType;
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
  // Still local state, but only for what CSS cannot do from here: the icon's
  // hover nudge and the portalled tooltip. The row's own hover colours moved
  // to `--pa-btn-bg-hover` / `--pa-btn-fg-hover`, where a stylesheet can
  // express them. The tooltip anchors on the event's own element, which is
  // the same node the ref used to hold.
  const [hovered, setHovered] = useState(false);
  const iconStyle: CSSProperties = {
    flexShrink: 0,
    transform: hovered && !reduceMotion ? 'scale(1.09)' : 'scale(1)',
    transition: reduceMotion ? 'none' : `transform 160ms ${ease.out}`,
  };

  return (
    <Button
      colors={colors}
      variant="bare"
      onClick={onClick}
      onMouseEnter={e => { setHovered(true); onHover?.(e.currentTarget, label, shortcut); }}
      onMouseLeave={() => { setHovered(false); onLeave?.(); }}
      onFocus={e => { setHovered(true); onHover?.(e.currentTarget, label, shortcut); }}
      onBlur={() => { setHovered(false); onLeave?.(); }}
      // The accessible name must survive the collapsed rail, where the text
      // label is not rendered at all. The visual tooltip is decoration; this
      // is what a screen reader announces.
      aria-label={shortcut ? `${label} (${shortcut})` : label}
      aria-current={active ? 'page' : undefined}
      style={{
        // Hover reads as a lift toward the active state rather than a separate
        // colour: the row you are pointing at should look like a preview of
        // what selecting it gives you.
        '--pa-btn-bg': active ? colors.cyanSoft : 'transparent',
        '--pa-btn-fg': active ? colors.cyan : colors.textMuted,
        '--pa-btn-border': active ? colors.borderHi : 'transparent',
        '--pa-btn-bg-hover': active ? colors.cyanSoft : colors.borderHi,
        '--pa-btn-fg-hover': colors.cyan,
        '--pa-btn-border-hover': active ? colors.borderHi : 'transparent',
        '--pa-btn-bg-active': active ? colors.cyanSoft : colors.borderHi,
        '--pa-btn-pad': open ? '0 12px' : '0',
        '--pa-btn-radius': '10px',
        '--pa-btn-weight': active ? 600 : 500,
        position: 'relative',
        width: open ? 'calc(100% - 16px)' : 40,
        height: 40,
        display: 'flex', gap: space.xl,
        justifyContent: open ? 'flex-start' : 'center',
        margin: open ? '0 8px' : '0 auto',
        fontFamily: font.body, fontSize: textSize.small,
        textAlign: 'left',
      } as CSSProperties}
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
      <RowGlyph icon={icon} style={iconStyle} />
      {open && <span style={{
        opacity: 1, transition: 'opacity 160ms', whiteSpace: 'nowrap',
      }}>{label}</span>}
    </Button>
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
  const navStatus = useNavAgentStatus();

  const isSettingsOpen = activePanel === 'settings';
  const isHistoryOpen = activePanel === 'history';
  // Any non-chat panel (settings, skills) is a full-screen overlay. It
  // must be dismissed when the user picks a workspace, or the overlay stays
  // stuck over the tab they just selected.
  const overlayOpen = activePanel !== 'chat';
  // The rail's silhouette now runs the full height of the window, so its
  // collapsed width is no longer a free choice: the native traffic lights sit
  // INSIDE it and span 72px (`trafficLightSpan()`), which is why
  // `shell.rail.collapsed` is 76 and not the 64 it was while a title strip sat
  // above the rail. `styles/shell.test.ts` fails if the two ever cross.
  const W = open ? shell.rail.open : shell.rail.collapsed;

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
      padding: '0 0 14px', gap: space.xs,
      transition: `width 220ms ${ease.out}`,
      overflow: 'hidden',
    }}>
      {/* The titlebar band, which belongs to the rail. The window runs
          `titleBarStyle: Overlay` + `hiddenTitle`, so this area is ours to draw
          and the native traffic lights composite over it — they are a native
          surface and nothing the webview paints can ever cover them, which is
          why this is empty space rather than something to lay out around.
          `shell.titlebar` (40) is what centres the 14px buttons at their 13px
          inset; the arithmetic lives in tokens.ts and in chrome.rs, and a test
          holds the three copies together. Draggable, so the whole top edge of
          the window moves it. */}
      <div data-tauri-drag-region style={{ height: shell.titlebar, flexShrink: 0 }} />

      {/* Brand row */}
      <div style={{
        display: 'flex', alignItems: 'center',
        padding: open ? '0 14px 14px' : '0 0 14px',
        justifyContent: open ? 'flex-start' : 'center',
      }}>
        <Mobius size={14} state="idle" glow={0.7} />
        {/* No connection indicator here by design.
            A green dot next to the logo asks the user to supervise something
            they cannot act on, and the honest states are not worth their
            attention: the shell now restarts the daemon if it dies (see
            ui/desktop/src-tauri/src/daemon.rs), and a closed per-session event
            stream is normal before a chat is open. Genuine, actionable
            failures surface as messages, not as a light to interpret. */}
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
        // Home carries the agent status indicator (2026-09-01 ruling) — the
        // dashboard's old hero card, retired, in a quiet line beside its row
        // instead of a whole card's worth of chrome. See NavAgentStatus.tsx.
        // Every other row is unaffected — same bare SidebarRow as before.
        if (!hasDashboardTool(ws.layoutJson)) {
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
        }
        return (
          <div key={ws.id} style={{ position: 'relative' }}>
            <SidebarRow
              icon={iconPath}
              label={ws.name}
              active={isActive}
              open={open}
              onClick={() => goToWorkspace(ws.id)}
              shortcut={shortcut}
              onHover={showTooltip}
              onLeave={hideTooltip}
            />
            {open ? (
              <NavStatusLine name={navStatus.name} state={navStatus.state} word={navStatus.word} />
            ) : (
              <NavStatusBadge
                name={navStatus.name}
                state={navStatus.state}
                word={navStatus.word}
                onHover={showTooltip}
                onLeave={hideTooltip}
              />
            )}
          </div>
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

      {/* History — Sessions, Downloads, Activity, Spend. This row is what the
          2026-08 ruling could not give them: the retired Console's four pages
          have their own destination again, rather than living behind Settings
          because Settings was the only overlay left standing. */}
      <HistoryRow
        open={open}
        active={isHistoryOpen}
        onOpen={() => setActivePanel(isHistoryOpen ? 'chat' : 'history')}
        onHover={showTooltip}
        onLeave={hideTooltip}
      />

      {/* Notifications — bell row with unread badge, tray anchors beside it. */}
      <NotificationBellRow open={open} onHover={showTooltip} onLeave={hideTooltip} />

      {/* Settings */}
      <SidebarRow
        icon={FiSettings}
        label="Settings"
        active={isSettingsOpen}
        open={open}
        onClick={() => setActivePanel(isSettingsOpen ? 'chat' : 'settings')}
        onHover={showTooltip}
        onLeave={hideTooltip}
      />

      {/* Collapse / Expand toggle */}
      {open ? (
        <Tooltip content="Collapse" placement="right">
          <Button
            colors={colors}
            variant="bare"
            onClick={() => setOpen(false)}
            style={{
              '--pa-btn-fg': colors.textDim,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-pad': '0',
              '--pa-btn-radius': `${radius.md}px`,
              width: 'calc(100% - 16px)', height: 32,
              margin: '4px 8px 0',
              gap: space.md,
              fontFamily: font.body, fontSize: textSize.micro,
            } as CSSProperties}
          >
            <FiChevronLeft size={12} />
            Collapse
          </Button>
        </Tooltip>
      ) : (
        <Tooltip content="Expand" placement="right">
          <Button
            colors={colors}
            variant="bare"
            onClick={() => setOpen(true)}
            aria-label="Expand"
            style={{
              '--pa-btn-fg': colors.textDim,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-pad': '0',
              '--pa-btn-radius': `${radius.md}px`,
              width: 40, height: 32, margin: '4px auto 0',
            } as CSSProperties}
          >
            <FiChevronRight size={12} />
          </Button>
        </Tooltip>
      )}

      {/* Portalled to document.body — the rail sets overflow:hidden to animate
          its width, which would clip a tooltip rendered inside it. */}
      <SidebarTooltip target={tooltipTarget} onDismiss={hideTooltip} />
    </div>
  );
}
