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

  return (
    <div
      className="flex items-center gap-1 px-3 py-1"
      style={{ backgroundColor: colors.surface, borderBottom: `1px solid ${colors.border}` }}
    >
      {/* Star: bookmark the current page */}
      <Button
        colors={colors}
        variant="bare"
        onClick={handleStar}
        disabled={!canStar}
        aria-label="Bookmark this page"
        style={{
          '--pa-btn-fg': starred ? colors.cyan : colors.textMuted,
          '--pa-btn-fg-hover': starred ? colors.cyan : colors.text,
          '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
          '--pa-btn-bg-active': 'rgba(255,255,255,0.09)',
          '--pa-btn-pad': '4px',
          '--pa-btn-radius': `${radius.xs}px`,
        } as CSSProperties}
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
      <div className="flex flex-1 items-center gap-1 overflow-x-auto min-w-0">
        {bookmarks.length === 0 ? (
          <span
            className="text-[10px] px-1 select-none"
            role={loadFailed ? 'alert' : undefined}
            style={{
              fontFamily: font.body,
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
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
                '--pa-btn-bg-active': 'rgba(255,255,255,0.09)',
                '--pa-btn-pad': '2px 8px',
                '--pa-btn-radius': `${radius.xs}px`,
                fontFamily: font.body, fontSize: 10, gap: 4,
              } as CSSProperties}
              title={b.url}
            >
              <span className="truncate max-w-[140px]">{b.title || b.url}</span>
              <span
                onClick={(e) => handleRemoveBookmark(b.url, e)}
                className="rounded p-0.5 opacity-0 group-hover:opacity-100 hover:bg-white/10 transition-colors"
                style={{ color: colors.textMuted }}
                title="Remove bookmark"
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
            '--pa-btn-fg': setsOpen ? colors.cyan : colors.textMuted,
            '--pa-btn-fg-hover': setsOpen ? colors.cyan : colors.text,
            '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
            '--pa-btn-bg-active': 'rgba(255,255,255,0.09)',
            '--pa-btn-pad': '2px 8px',
            '--pa-btn-radius': `${radius.xs}px`,
            fontFamily: font.body, fontSize: 10, gap: 4,
          } as CSSProperties}
          title="Saved tab sets"
        >
          <FiLayers size={11} />
          <span>Tabs{tabSets.length > 0 ? ` (${tabSets.length})` : ''}</span>
        </Button>

        {setsOpen && (
          <div
            className="absolute right-0 top-full mt-1 w-64 rounded-md shadow-lg z-50 py-1"
            style={{ backgroundColor: colors.bgDeeper, border: `1px solid ${colors.border}` }}
          >
            {/* Save the currently open tabs as a named set */}
            <div className="flex items-center gap-1 px-2 pb-1" style={{ borderBottom: `1px solid ${colors.border}` }}>
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
                className="bookmarks-set-name flex-1 bg-transparent text-[10px] py-1 px-1 outline-none rounded"
                style={{ fontFamily: font.mono, color: colors.text }}
              />
              <style>{`.bookmarks-set-name::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
              <Button
                colors={colors}
                variant="bare"
                onClick={handleSaveTabs}
                disabled={currentTabs.length === 0 || !saveName.trim()}
                style={{
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
                  '--pa-btn-bg-active': 'rgba(255,255,255,0.09)',
                  '--pa-btn-pad': '2px 6px',
                  '--pa-btn-radius': `${radius.xs}px`,
                  fontFamily: font.body, fontSize: 10,
                } as CSSProperties}
              >
                Save
              </Button>
            </div>

            {tabSets.length === 0 ? (
              <div
                className="px-3 py-2 text-[10px]"
                style={{ fontFamily: font.body, color: colors.textMuted, opacity: 0.6 }}
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
                    '--pa-btn-fg': colors.textMuted,
                    '--pa-btn-fg-hover': colors.text,
                    '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
                    '--pa-btn-bg-active': 'rgba(255,255,255,0.09)',
                    '--pa-btn-pad': '6px 12px',
                    '--pa-btn-radius': '0',
                    fontFamily: font.body, fontSize: 10, gap: 6, textAlign: 'left',
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
                    className={`rounded p-0.5 transition-colors ${
                      armedDelete === set.name
                        ? 'bg-red-500/20 text-red-400'
                        : 'opacity-0 group-hover:opacity-100 hover:bg-white/10'
                    }`}
                    style={armedDelete === set.name ? undefined : { color: colors.textMuted }}
                    title={armedDelete === set.name ? 'Click again to delete' : 'Delete set'}
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
