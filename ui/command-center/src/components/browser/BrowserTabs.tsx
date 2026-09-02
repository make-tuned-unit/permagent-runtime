import { type CSSProperties } from 'react';
import { FiPlus, FiX, FiGlobe, FiExternalLink } from 'react-icons/fi';
import { font, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { CycleTabsButton } from '../build/CycleTabsButton';
import { CHROME_GEOM, chromeBareVars, dangerWash } from './browserChrome';

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
  /** Trailing glyphs: ink only on the shared glass plane (D2/D3). */
  const railIcon = (hover: string): CSSProperties => ({
    ...chromeBareVars(colors, { fg: colors.textMuted, fgHover: hover, pad: `${CHROME_GEOM.tabPadY}px ${CHROME_GEOM.chipPadX}px`, radiusPx: 0 }),
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
  } as CSSProperties);

  return (
    // No fill of its own — sits on the parent glass stack (D1/D3). Hard edge
    // on the scroll strip (D11), not a soft mask.
    <div className="flex items-center" style={{ borderBottom: `1px solid ${colors.border}` }}>
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
                ...chromeBareVars(colors, {
                  bg: isActive ? colors.fillSubtle : 'transparent',
                  fg: isActive ? colors.cyan : colors.textMuted,
                  fgHover: isActive ? colors.cyan : colors.text,
                  pad: `${CHROME_GEOM.tabPadY}px ${CHROME_GEOM.tabPadX}px`,
                  radiusPx: 0,
                }),
                // Active tab keeps its resting fill on hover/press — the fill
                // ladder only lifts inactive tabs (D10 on glass).
                ...(isActive
                  ? {
                      '--pa-btn-bg-hover': colors.fillSubtle,
                      '--pa-btn-bg-active': colors.fillSubtle,
                    }
                  : null),
                fontFamily: font.mono,
                fontSize: textSize.micro,
                gap: CHROME_GEOM.bookmarksGap,
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
                    closingTabId === tab.id ? '' : 'opacity-0 group-hover:opacity-100'
                  }`}
                  style={
                    closingTabId === tab.id
                      ? { background: dangerWash(colors), color: colors.danger }
                      : { color: colors.textMuted }
                  }
                  onMouseEnter={(e) => {
                    if (closingTabId === tab.id) return;
                    e.currentTarget.style.background = colors.fillHover;
                  }}
                  onMouseLeave={(e) => {
                    if (closingTabId === tab.id) return;
                    e.currentTarget.style.background = 'transparent';
                  }}
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
          style={{ ...railIcon(colors.text), '--pa-btn-pad': `${CHROME_GEOM.tabPadY}px ${CHROME_GEOM.chipPadX}px` } as CSSProperties}
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
        style={{ ...railIcon(colors.cyan), '--pa-btn-pad': `${CHROME_GEOM.tabPadY}px ${CHROME_GEOM.chipPadX + 2}px` } as CSSProperties}
      >
        <FiPlus size={13} />
      </Button>
    </div>
  );
}
