import { FiPlus, FiX, FiGlobe } from 'react-icons/fi';
import { useTheme } from '../../styles/useTheme';

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
}

export function BrowserTabs({
  tabs,
  activeTabId,
  closingTabId,
  onSelectTab,
  onCloseTab,
  onNewTab,
}: BrowserTabsProps) {
  const { colors } = useTheme();
  return (
    <div className="flex items-center border-b border-dark-border" style={{ backgroundColor: colors.bg }}>
      <div className="flex flex-1 items-center overflow-x-auto">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => onSelectTab(tab.id)}
            className={`group flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-mono border-r border-dark-border transition-colors shrink-0 ${
              tab.id === activeTabId
                ? 'text-accent'
                : 'text-dark-muted hover:text-dark-text hover:bg-white/5'
            }`}
            style={tab.id === activeTabId ? { backgroundColor: colors.surface } : undefined}
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
                    : 'opacity-0 group-hover:opacity-100 hover:bg-white/10 text-dark-muted'
                }`}
              >
                <FiX size={10} />
              </span>
            )}
          </button>
        ))}
      </div>
      <button
        onClick={onNewTab}
        className="px-2.5 py-1.5 text-dark-muted hover:text-accent transition-colors"
        title="New tab (Cmd+T)"
      >
        <FiPlus size={13} />
      </button>
    </div>
  );
}
