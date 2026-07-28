// Console — one surface for the app's operational overlays (2026-07-27
// consolidation ruling): Sessions, Inbox, Trace, and Governance were four
// separate sidebar rows; they are now tabs of a single page. Each tab renders
// the SAME panel component as before — only the chrome consolidated, so deep
// links (navigateToTool, notification targets, skill proposals) that set
// activePanel still land on the right tab.

import { useCommandCenter, type ActivePanel } from '../../lib/store';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export const CONSOLE_PANELS = ['sessions', 'inbox', 'trace', 'governance'] as const;
export type ConsolePanel = (typeof CONSOLE_PANELS)[number];

export function isConsolePanel(panel: ActivePanel): panel is ConsolePanel {
  return (CONSOLE_PANELS as readonly string[]).includes(panel);
}

const TAB_LABELS: Record<ConsolePanel, string> = {
  sessions: 'Sessions',
  inbox: 'Inbox',
  trace: 'Trace',
  governance: 'Governance',
};

export function ConsoleTabStrip({ active }: { active: ConsolePanel }) {
  const { colors } = useTheme();
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0,
      padding: '8px 16px 0',
      borderBottom: `1px solid ${colors.border}`,
    }}>
      {CONSOLE_PANELS.map(panel => {
        const isActive = panel === active;
        return (
          <button
            key={panel}
            onClick={() => setActivePanel(panel)}
            style={{
              fontFamily: font.body, fontSize: 12, fontWeight: isActive ? 600 : 500,
              color: isActive ? colors.text : colors.textMuted,
              background: 'transparent', border: 'none', cursor: 'pointer',
              padding: '8px 12px 10px',
              borderBottom: `2px solid ${isActive ? colors.cyan : 'transparent'}`,
              marginBottom: -1,
              transition: `color 150ms ${ease.out}`,
            }}
          >
            {TAB_LABELS[panel]}
          </button>
        );
      })}
      <div style={{ flex: 1 }} />
      <button
        onClick={() => setActivePanel('chat')}
        title="Close"
        aria-label="Close console"
        style={{
          background: 'transparent', border: 'none', cursor: 'pointer',
          color: colors.textMuted, fontSize: 14, padding: '4px 6px 8px',
        }}
      >✕</button>
    </div>
  );
}
