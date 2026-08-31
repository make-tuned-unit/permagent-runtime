import { type CSSProperties } from 'react';
import { FiPlus, FiX, FiGlobe, FiExternalLink } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
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
  onPopOut?: () => void;
}

export function BrowserTabs({
  tabs,
  activeTabId,
  closingTabId,
  onSelectTab,
  onCloseTab,
  onNewTab,
  onCycleTab,
  onPopOut,
}: BrowserTabsProps) {
  const { colors } = useTheme();
  /** The two trailing glyphs: no chrome, ink only, exactly as before — the
   *  hover that was a pair of mouse handlers on "new tab" is now the class's. */
  const railIcon = (hover: string): CSSProperties => ({
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-fg-hover': hover,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
    '--pa-btn-radius': '0',
  } as CSSProperties);
  return (
    <div className="flex items-center" style={{ backgroundColor: colors.bg, borderBottom: `1px solid ${colors.border}` }}>
      <div className="flex flex-1 items-center overflow-x-auto">
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          return (
            // The tab keeps its own flex distribution — globe, truncating
            // title, close affordance — so the primitive's label wrapper is
            // dissolved with `display: contents`. `borderRight` stays inline:
            // it is the strip's divider, not the button's own outline, and
            // `--pa-btn-border` is transparent so the other three edges paint
            // nothing.
            <Button
              key={tab.id}
              colors={colors}
              variant="bare"
              onClick={() => onSelectTab(tab.id)}
              className="group shrink-0"
              style={{
                '--pa-btn-bg': isActive ? colors.surface : 'transparent',
                '--pa-btn-fg': isActive ? colors.cyan : colors.textMuted,
                '--pa-btn-border': 'transparent',
                '--pa-btn-bg-hover': isActive ? colors.surface : 'rgba(255,255,255,0.05)',
                '--pa-btn-fg-hover': isActive ? colors.cyan : colors.text,
                '--pa-btn-bg-active': isActive ? colors.surface : 'rgba(255,255,255,0.09)',
                '--pa-btn-pad': '6px 12px',
                '--pa-btn-radius': '0',
                fontFamily: font.mono,
                fontSize: 11,
                gap: 6,
                borderRight: `1px solid ${colors.border}`,
              } as CSSProperties}
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
            </Button>
          );
        })}
      </div>
      <CycleTabsButton pane="browser" onCycle={onCycleTab} />
      {onPopOut && (
        <Button
          colors={colors}
          variant="bare"
          onClick={onPopOut}
          aria-label="Pop out active browser"
          title="Pop out active browser"
          style={{ ...railIcon(colors.text), '--pa-btn-pad': '6px 8px' } as CSSProperties}
        >
          <FiExternalLink size={13} />
        </Button>
      )}
      <Button
        colors={colors}
        variant="bare"
        onClick={onNewTab}
        aria-label="New tab"
        title="New tab (Cmd+T)"
        style={{ ...railIcon(colors.cyan), '--pa-btn-pad': '6px 10px' } as CSSProperties}
      >
        <FiPlus size={13} />
      </Button>
    </div>
  );
}
