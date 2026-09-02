import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiStar, FiX, FiLayers, FiTrash2, FiCornerUpLeft } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { api, type BrowserBookmark, type BrowserTabSet } from '../../lib/api';
import {
  isBookmarked,
  isPersistableUrl,
  removeBookmark,
  removeTabSet,
  savableTabs,
  saveTabSet,
  toggleBookmark,
} from './bookmarksLogic';
import {
  ADDRESS_RADIUS,
  CHIP_RADIUS,
  CHROME_GEOM,
  chromeBareVars,
  dangerWash,
} from './browserChrome';

/** Dense chrome caption — was literal `10` / `text-[10px]` across this bar.
 *  Not on the type ramp (micro=11); changing it would thicken the row and
 *  shrink the webview. Request a ramp step from A1c if one is wanted. */
const CHROME_CAPTION = 10;

interface BookmarksBarProps {
  /** URL/title of the active tab — what the star bookmarks. */
  currentUrl: string;
  currentTitle: string;
  /** All open tabs — what "Save tabs" snapshots into a named set. */
  openTabs: Array<{ url: string; label: string }>;
  /** Navigate the ACTIVE tab (bookmark chip click). */
  onNavigate: (url: string) => void;
  /** Open a URL in a NEW tab (tab-set restore). */
  onOpenInNewTab: (url: string) => Promise<void>;
}

/**
 * Bookmarks + saved-tabs row below the address bar (#790). All state is
 * daemon-persisted via /api/browser/bookmarks and /api/browser/tab-sets
 * (routes/browser_state.rs) so it survives app restarts — no localStorage.
 *
 * Sits on the parent glass chrome stack (D1/D3) — no fill of its own. The
 * saved-tabs menu is an opaque elevated surface (D2: never glass-on-glass).
 */
export function BookmarksBar({
  currentUrl,
  currentTitle,
  openTabs,
  onNavigate,
  onOpenInNewTab,
}: BookmarksBarProps) {
  const { colors } = useTheme();
  const [bookmarks, setBookmarks] = useState<BrowserBookmark[]>([]);
  /** The bookmark read failed. It used to be a `console.error` and an empty
   *  bar, which says "No bookmarks yet" — a user who has saved a dozen may
   *  reasonably conclude they lost them, and re-save. */
  const [loadFailed, setLoadFailed] = useState(false);
  const [tabSets, setTabSets] = useState<BrowserTabSet[]>([]);
  const [setsOpen, setSetsOpen] = useState(false);
  const [saveName, setSaveName] = useState('');
  /** Arm-then-confirm delete (2s), mirroring the tab-close pattern. */
  const [armedDelete, setArmedDelete] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // ── Load persisted state once on mount ──
  useEffect(() => {
    let cancelled = false;
    api
      .getBrowserBookmarks()
      .then((r) => {
        if (!cancelled) { setBookmarks(r.bookmarks); setLoadFailed(false); }
      })
      .catch((err) => {
        console.error('[bookmarks] load failed:', err);
        if (!cancelled) setLoadFailed(true);
      });
    api
      .getBrowserTabSets()
      .then((r) => {
        if (!cancelled) setTabSets(r.tabSets);
      })
      .catch((err) => console.error('[bookmarks] tab-set load failed:', err));
    return () => {
      cancelled = true;
    };
  }, []);

  // ── Close the sets menu on outside click ──
  useEffect(() => {
    if (!setsOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setSetsOpen(false);
        setArmedDelete(null);
      }
    };
    document.addEventListener('mousedown', onMouseDown);
    return () => document.removeEventListener('mousedown', onMouseDown);
  }, [setsOpen]);

  /** Optimistic update + daemon persist; revert on failure so the row never
   *  shows state the daemon doesn't hold. */
  const persistBookmarks = useCallback(
    (next: BrowserBookmark[], prev: BrowserBookmark[]) => {
      setBookmarks(next);
      api.putBrowserBookmarks(next).catch((err) => {
        console.error('[bookmarks] save failed:', err);
        setBookmarks(prev);
      });
    },
    [],
  );

  const persistTabSets = useCallback((next: BrowserTabSet[], prev: BrowserTabSet[]) => {
    setTabSets(next);
    api.putBrowserTabSets(next).catch((err) => {
      console.error('[bookmarks] tab-set save failed:', err);
      setTabSets(prev);
    });
  }, []);

  const handleStar = useCallback(() => {
    const next = toggleBookmark(bookmarks, currentUrl, currentTitle);
    if (next) persistBookmarks(next, bookmarks);
  }, [bookmarks, currentUrl, currentTitle, persistBookmarks]);

  const handleRemoveBookmark = useCallback(
    (url: string, e: React.MouseEvent) => {
      e.stopPropagation();
      persistBookmarks(removeBookmark(bookmarks, url), bookmarks);
    },
    [bookmarks, persistBookmarks],
  );

  const currentTabs = savableTabs(openTabs);

  const handleSaveTabs = useCallback(() => {
    const next = saveTabSet(tabSets, saveName, currentTabs);
    if (!next) return;
    persistTabSets(next, tabSets);
    setSaveName('');
  }, [tabSets, saveName, currentTabs, persistTabSets]);

  const handleRestore = useCallback(
    async (set: BrowserTabSet) => {
      setSetsOpen(false);
      // Sequential so tab order matches the saved order.
      for (const tab of set.tabs) {
        await onOpenInNewTab(tab.url);
      }
    },
    [onOpenInNewTab],
  );

  const handleDeleteSet = useCallback(
    (name: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (armedDelete === name) {
        persistTabSets(removeTabSet(tabSets, name), tabSets);
        setArmedDelete(null);
        return;
      }
      setArmedDelete(name);
      setTimeout(() => setArmedDelete((p) => (p === name ? null : p)), 2000);
    },
    [armedDelete, tabSets, persistTabSets],
  );

  const starred = isBookmarked(bookmarks, currentUrl);
  const canStar = isPersistableUrl(currentUrl);
  const chipPad = `${CHROME_GEOM.chipPadY}px ${CHROME_GEOM.chipPadX}px`;

  return (
    <div
      className="flex items-center min-w-0"
      style={{
        gap: CHROME_GEOM.bookmarksGap,
        padding: `${CHROME_GEOM.bookmarksPadY}px ${CHROME_GEOM.bookmarksPadX}px`,
        borderBottom: `1px solid ${colors.border}`,
      }}
    >
      {/* Star: bookmark the current page */}
      <Button
        colors={colors}
        variant="bare"
        onClick={handleStar}
        disabled={!canStar}
        aria-label="Bookmark this page"
        style={chromeBareVars(colors, {
          fg: starred ? colors.cyan : colors.textMuted,
          fgHover: starred ? colors.cyan : colors.text,
          pad: `${CHROME_GEOM.chipPadY * 2}px`,
          radiusPx: CHIP_RADIUS,
        })}
        title={
          !canStar
            ? 'Open a page to bookmark it'
            : starred
              ? 'Remove bookmark'
              : 'Bookmark this page'
        }
      >
        <FiStar size={13} style={starred ? { fill: colors.cyan } : undefined} />
      </Button>

      {/* Bookmark chips */}
      <div className="flex flex-1 items-center min-w-0 overflow-x-auto" style={{ gap: CHROME_GEOM.bookmarksGap }}>
        {bookmarks.length === 0 ? (
          <span
            className="select-none"
            role={loadFailed ? 'alert' : undefined}
            style={{
              fontFamily: font.body,
              fontSize: CHROME_CAPTION,
              padding: `0 ${CHROME_GEOM.bookmarksGap}px`,
              color: loadFailed ? colors.danger : colors.textMuted,
              opacity: loadFailed ? 1 : 0.6,
            }}
          >
            {loadFailed ? "Couldn't load your bookmarks — they're still saved." : 'No bookmarks yet'}
          </span>
        ) : (
          bookmarks.map((b) => (
            // `` dissolves the primitive's label
            // wrapper: the chip's title and its remove affordance must stay the
            // button's own flex children or the truncation and the
            // group-hover reveal both come apart.
            <Button
              key={b.url}
              colors={colors}
              variant="bare"
              onClick={() => onNavigate(b.url)}
              // The chip spins for the navigation, but does not tick: the tab
              // title and the address bar are what confirm a page arrived, and
              // a second green confirmation on the chip you left behind is one
              // claim too many.
              flashSuccess={false}
              className="group shrink-0"
              style={{
                ...chromeBareVars(colors, {
                  pad: chipPad,
                  radiusPx: CHIP_RADIUS,
                }),
                fontFamily: font.body,
                fontSize: CHROME_CAPTION,
                gap: CHROME_GEOM.chipPadY * 2,
              } as CSSProperties}
              title={b.url}
            >
              <span className="truncate max-w-[140px]">{b.title || b.url}</span>
              <span
                onClick={(e) => handleRemoveBookmark(b.url, e)}
                className="opacity-0 group-hover:opacity-100"
                style={{
                  color: colors.textMuted,
                  borderRadius: radius.xs,
                  padding: 2,
                }}
                title="Remove bookmark"
                onMouseEnter={(e) => { e.currentTarget.style.background = colors.fillHover; }}
                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}
              >
                <FiX size={9} />
              </span>
            </Button>
          ))
        )}
      </div>

      {/* Saved tab sets */}
      <div className="relative shrink-0" ref={menuRef}>
        <Button
          colors={colors}
          variant="bare"
          onClick={() => {
            setSetsOpen((o) => !o);
            setArmedDelete(null);
          }}
          style={{
            ...chromeBareVars(colors, {
              fg: setsOpen ? colors.cyan : colors.textMuted,
              fgHover: setsOpen ? colors.cyan : colors.text,
              pad: chipPad,
              radiusPx: CHIP_RADIUS,
            }),
            fontFamily: font.body,
            fontSize: CHROME_CAPTION,
            gap: CHROME_GEOM.chipPadY * 2,
          } as CSSProperties}
          title="Saved tab sets"
        >
          <FiLayers size={11} />
          <span>Tabs{tabSets.length > 0 ? ` (${tabSets.length})` : ''}</span>
        </Button>

        {setsOpen && (
          // Opaque elevated menu — not glass (D2: parent chrome is already glass).
          <div
            className="absolute right-0 top-full z-50 py-1"
            style={{
              marginTop: CHROME_GEOM.bookmarksGap,
              width: 256,
              backgroundColor: colors.surface,
              border: `1px solid ${colors.border}`,
              borderRadius: ADDRESS_RADIUS,
              boxShadow: colors.elevationOverlay,
            }}
          >
            {/* Save the currently open tabs as a named set */}
            <div
              className="flex items-center"
              style={{
                gap: CHROME_GEOM.bookmarksGap,
                padding: `0 ${CHROME_GEOM.chipPadX}px ${CHROME_GEOM.bookmarksGap}px`,
                borderBottom: `1px solid ${colors.border}`,
              }}
            >
              <input
                type="text"
                value={saveName}
                onChange={(e) => setSaveName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleSaveTabs();
                }}
                placeholder={
                  currentTabs.length > 0
                    ? `Name for ${currentTabs.length} open tab${currentTabs.length !== 1 ? 's' : ''}...`
                    : 'No open tabs to save'
                }
                disabled={currentTabs.length === 0}
                className="bookmarks-set-name flex-1 bg-transparent outline-none"
                style={{
                  fontFamily: font.mono,
                  fontSize: CHROME_CAPTION,
                  color: colors.text,
                  padding: `${CHROME_GEOM.bookmarksGap}px`,
                  borderRadius: CHIP_RADIUS,
                }}
              />
              <style>{`.bookmarks-set-name::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
              <Button
                colors={colors}
                variant="bare"
                onClick={handleSaveTabs}
                disabled={currentTabs.length === 0 || !saveName.trim()}
                style={{
                  ...chromeBareVars(colors, {
                    fg: colors.cyan,
                    fgHover: colors.cyan,
                    pad: `${CHROME_GEOM.chipPadY}px ${CHROME_GEOM.bookmarksGap + 2}px`,
                    radiusPx: CHIP_RADIUS,
                  }),
                  fontFamily: font.body,
                  fontSize: CHROME_CAPTION,
                } as CSSProperties}
              >
                Save
              </Button>
            </div>

            {tabSets.length === 0 ? (
              <div
                style={{
                  padding: `${CHROME_GEOM.chipPadX}px ${CHROME_GEOM.tabPadX}px`,
                  fontFamily: font.body,
                  fontSize: CHROME_CAPTION,
                  color: colors.textMuted,
                  opacity: 0.6,
                }}
              >
                No saved tab sets yet
              </div>
            ) : (
              tabSets.map((set) => (
                // Same as the chips above: the row distributes a `flex-1` name
                // against right-aligned meta, so the label wrapper has to be
                // `display: contents` or that distribution collapses.
                <Button
                  key={set.name}
                  colors={colors}
                  variant="bare"
                  onClick={() => handleRestore(set)}
                  className="group w-full"
                  style={{
                    ...chromeBareVars(colors, {
                      pad: `${CHROME_GEOM.tabPadY}px ${CHROME_GEOM.tabPadX}px`,
                      radiusPx: 0,
                    }),
                    fontFamily: font.body,
                    fontSize: CHROME_CAPTION,
                    gap: CHROME_GEOM.bookmarksGap + 2,
                    textAlign: 'left',
                  } as CSSProperties}
                  title={`Restore ${set.tabs.length} tab${set.tabs.length !== 1 ? 's' : ''}`}
                >
                  <FiCornerUpLeft size={10} style={{ color: colors.cyan }} />
                  <span className="flex-1 truncate">{set.name}</span>
                  <span className="shrink-0" style={{ opacity: 0.7 }}>
                    {set.tabs.length} tab{set.tabs.length !== 1 ? 's' : ''}
                  </span>
                  <span
                    onClick={(e) => handleDeleteSet(set.name, e)}
                    className={armedDelete === set.name ? '' : 'opacity-0 group-hover:opacity-100'}
                    style={
                      armedDelete === set.name
                        ? { background: dangerWash(colors), color: colors.danger, borderRadius: radius.xs, padding: 2 }
                        : { color: colors.textMuted, borderRadius: radius.xs, padding: 2 }
                    }
                    title={armedDelete === set.name ? 'Click again to delete' : 'Delete set'}
                    onMouseEnter={(e) => {
                      if (armedDelete === set.name) return;
                      e.currentTarget.style.background = colors.fillHover;
                    }}
                    onMouseLeave={(e) => {
                      if (armedDelete === set.name) return;
                      e.currentTarget.style.background = 'transparent';
                    }}
                  >
                    <FiTrash2 size={10} />
                  </span>
                </Button>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}
