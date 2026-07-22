import { FiPlus, FiX, FiGlobe } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { CycleTabsButton } from '../build/CycleTabsButton';

export interface BrowserTab {
  id: string;
  label: string;
  webviewId: string | null;
  url: string;
  loading: boolean;
}

interface BrowserTabsProps {
  tabs: BrowserTab[];
  activeTabId: string;
  closingTabId: string | null;
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string, e?: React.MouseEvent) => void;
  onNewTab: () => void;
  onCycleTab: () => void;
}

export function BrowserTabs({
  tabs,
  activeTabId,
  closingTabId,
  onSelectTab,
  onCloseTab,
  onNewTab,
  onCycleTab,
}: BrowserTabsProps) {
  const { colors } = useTheme();
  return (
    <div className="flex items-center" style={{ backgroundColor: colors.bg, borderBottom: `1px solid ${colors.border}` }}>
      <div className="flex flex-1 items-center overflow-x-auto">
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          return (
            <button
              key={tab.id}
              onClick={() => onSelectTab(tab.id)}
              className={`group flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors shrink-0 ${isActive ? '' : 'hover:bg-white/5'}`}
              style={{
                fontFamily: font.mono,
                borderRight: `1px solid ${colors.border}`,
                color: isActive ? colors.cyan : colors.textMuted,
                backgroundColor: isActive ? colors.surface : undefined,
              }}
              onMouseEnter={e => { if (!isActive) e.currentTarget.style.color = colors.text; }}
              onMouseLeave={e => { if (!isActive) e.currentTarget.style.color = colors.textMuted; }}
            >
              <FiGlobe size={11} className={tab.loading ? 'animate-spin' : ''} />
              <span className="truncate max-w-[120px]">
                {tab.label || 'New Tab'}
              </span>
              {tabs.length > 1 && (
                <span
                  onClick={(e) => onCloseTab(tab.id, e)}
                  className={`ml-1 rounded p-0.5 transition-colors ${
                    closingTabId === tab.id
                      ? 'bg-red-500/20 text-red-400'
                      : 'opacity-0 group-hover:opacity-100 hover:bg-white/10'
                  }`}
                  style={closingTabId === tab.id ? undefined : { color: colors.textMuted }}
                >
                  <FiX size={10} />
                </span>
              )}
            </button>
          );
        })}
      </div>
      <CycleTabsButton pane="browser" onCycle={onCycleTab} />
      <button
        onClick={onNewTab}
        className="px-2.5 py-1.5 transition-colors"
        style={{ color: colors.textMuted }}
        onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; }}
        onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
        title="New tab (Cmd+T)"
      >
        <FiPlus size={13} />
      </button>
    </div>
  );
}
