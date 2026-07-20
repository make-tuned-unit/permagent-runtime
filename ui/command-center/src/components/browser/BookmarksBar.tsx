import { useCallback, useEffect, useRef, useState } from 'react';
import { FiStar, FiX, FiLayers, FiTrash2, FiCornerUpLeft } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
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
        if (!cancelled) setBookmarks(r.bookmarks);
      })
      .catch((err) => console.error('[bookmarks] load failed:', err));
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
      <button
        onClick={handleStar}
        disabled={!canStar}
        className="p-1 rounded transition-colors hover:bg-white/5 disabled:opacity-40"
        style={{ color: starred ? colors.cyan : colors.textMuted }}
        title={
          !canStar
            ? 'Open a page to bookmark it'
            : starred
              ? 'Remove bookmark'
              : 'Bookmark this page'
        }
      >
        <FiStar size={13} style={starred ? { fill: colors.cyan } : undefined} />
      </button>

      {/* Bookmark chips */}
      <div className="flex flex-1 items-center gap-1 overflow-x-auto min-w-0">
        {bookmarks.length === 0 ? (
          <span
            className="text-[10px] px-1 select-none"
            style={{ fontFamily: font.body, color: colors.textMuted, opacity: 0.6 }}
          >
            No bookmarks yet
          </span>
        ) : (
          bookmarks.map((b) => (
            <button
              key={b.url}
              onClick={() => onNavigate(b.url)}
              className="group flex items-center gap-1 px-2 py-0.5 rounded text-[10px] transition-colors hover:bg-white/5 shrink-0"
              style={{ fontFamily: font.body, color: colors.textMuted }}
              onMouseEnter={(e) => {
                e.currentTarget.style.color = colors.text;
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.color = colors.textMuted;
              }}
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
            </button>
          ))
        )}
      </div>

      {/* Saved tab sets */}
      <div className="relative shrink-0" ref={menuRef}>
        <button
          onClick={() => {
            setSetsOpen((o) => !o);
            setArmedDelete(null);
          }}
          className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] transition-colors hover:bg-white/5"
          style={{
            fontFamily: font.body,
            color: setsOpen ? colors.cyan : colors.textMuted,
          }}
          title="Saved tab sets"
        >
          <FiLayers size={11} />
          <span>Tabs{tabSets.length > 0 ? ` (${tabSets.length})` : ''}</span>
        </button>

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
              <button
                onClick={handleSaveTabs}
                disabled={currentTabs.length === 0 || !saveName.trim()}
                className="px-1.5 py-0.5 rounded text-[10px] transition-colors hover:bg-white/5 disabled:opacity-40"
                style={{ fontFamily: font.body, color: colors.cyan }}
              >
                Save
              </button>
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
                <button
                  key={set.name}
                  onClick={() => handleRestore(set)}
                  className="group flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-[10px] transition-colors hover:bg-white/5"
                  style={{ fontFamily: font.body, color: colors.textMuted }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.color = colors.text;
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.color = colors.textMuted;
                  }}
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
                </button>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}
