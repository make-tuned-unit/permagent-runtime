import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiStar, FiX, FiLayers, FiTrash2, FiCornerUpLeft } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { pressable } from '../common/pressable';
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

import { Tooltip } from '../common/Tooltip';
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
    // The remove button is its own real, sibling `<button>` now (not nested
    // inside the row's), so ITS OWN onClick stops propagation before calling
    // this — nothing here needs a mouse event to do it too.
    (url: string) => {
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
    // The delete button is its own real, sibling `<button>` (see
    // `handleRemoveBookmark`); it stops its own propagation.
    (name: string) => {
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
      <Tooltip content={!canStar
            ? 'Open a page to bookmark it'
            : starred
              ? 'Remove bookmark'
              : 'Bookmark this page'}>
        <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
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
          >
            <FiStar size={13} style={starred ? { fill: colors.cyan } : undefined} />
          </Button>
        </span>
      </Tooltip>

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
            // A `<button>` cannot contain interactive content (HTML5 content
            // model; screen readers also announce a nested control as one
            // control, not two) — so the chip is a `pressable` ROW and the
            // remove affordance is its own real sibling `<button>`, not a
            // span nested inside the row's button. `.pa-btn` on the row
            // reproduces the exact look `Button` gave it (same
            // `chromeBareVars` vars); `.pa-pressable` (from `pressable()`)
            // layers the row's new press/focus feedback on top.
            <Tooltip content={b.url}>
              <div
                key={b.url}
                {...pressable(() => onNavigate(b.url), {
                  label: `Open bookmark: ${b.title || b.url}`,
                  className: 'pa-btn group shrink-0',
                })}
                style={{
                  ...chromeBareVars(colors, {
                    pad: chipPad,
                    radiusPx: CHIP_RADIUS,
                  }),
                  fontFamily: font.body,
                  fontSize: CHROME_CAPTION,
                  gap: CHROME_GEOM.chipPadY * 2,
                } as CSSProperties}
              >
                <span className="truncate max-w-[140px]">{b.title || b.url}</span>
                <Tooltip content="Remove bookmark">
                  <Button
                    colors={colors}
                    variant="bare"
                    type="button"
                    aria-label={`Remove bookmark: ${b.title || b.url}`}
                    // Nested visually inside the row, but a real SIBLING
                    // element now (not inside another button) — the click
                    // still must stop here or it also fires the row's
                    // onNavigate underneath it.
                    onClick={(e) => { e.stopPropagation(); handleRemoveBookmark(b.url); }}
                    className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                    style={{
                      '--pa-btn-pad': '2px',
                      '--pa-btn-radius': `${radius.xs}px`,
                      '--pa-btn-bg': 'transparent',
                      '--pa-btn-fg': colors.textMuted,
                      '--pa-btn-bg-hover': colors.fillHover,
                    } as CSSProperties}
                  >
                    <FiX size={9} />
                  </Button>
                </Tooltip>
              </div>
            </Tooltip>
          ))
        )}
      </div>

      {/* Saved tab sets */}
      <div className="relative shrink-0" ref={menuRef}>
        <Tooltip content="Saved tab sets">
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
          >
            <FiLayers size={11} />
            <span>Tabs{tabSets.length > 0 ? ` (${tabSets.length})` : ''}</span>
          </Button>
        </Tooltip>

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
                // Same restructure as the bookmark chips above: a `pressable`
                // row (not a `<Button>`) plus a real sibling `<button>` for
                // delete, so the two controls are never nested inside one
                // another.
                <Tooltip content={`Restore ${set.tabs.length} tab${set.tabs.length !== 1 ? 's' : ''}`}>
                  <div
                    key={set.name}
                    {...pressable(() => handleRestore(set), {
                      label: `Restore tab set: ${set.name} (${set.tabs.length} tab${set.tabs.length !== 1 ? 's' : ''})`,
                      className: 'pa-btn group w-full',
                    })}
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
                  >
                    <FiCornerUpLeft size={10} style={{ color: colors.cyan }} />
                    <span className="flex-1 truncate">{set.name}</span>
                    <span className="shrink-0" style={{ opacity: 0.7 }}>
                      {set.tabs.length} tab{set.tabs.length !== 1 ? 's' : ''}
                    </span>
                    <Tooltip content={armedDelete === set.name ? 'Click again to delete' : 'Delete set'}>
                      <Button
                        colors={colors}
                        variant="bare"
                        type="button"
                        aria-label={armedDelete === set.name
                          ? `Confirm delete of tab set: ${set.name}`
                          : `Delete tab set: ${set.name}`}
                        onClick={(e) => { e.stopPropagation(); handleDeleteSet(set.name); }}
                        className={armedDelete === set.name ? '' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}
                        style={{
                          '--pa-btn-pad': '2px',
                          '--pa-btn-radius': `${radius.xs}px`,
                          '--pa-btn-bg': armedDelete === set.name ? dangerWash(colors) : 'transparent',
                          '--pa-btn-fg': armedDelete === set.name ? colors.danger : colors.textMuted,
                          '--pa-btn-bg-hover': armedDelete === set.name ? dangerWash(colors) : colors.fillHover,
                        } as CSSProperties}
                      >
                        <FiTrash2 size={10} />
                      </Button>
                    </Tooltip>
                  </div>
                </Tooltip>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}
