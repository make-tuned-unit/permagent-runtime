import { useEffect, useLayoutEffect, useMemo, useState, useRef, type CSSProperties, type RefObject } from 'react';
import { FiPlus, FiTrash2, FiMessageSquare, FiX, FiSearch, FiGitBranch } from 'react-icons/fi';
import { useCommandCenter, type SessionState } from '../../lib/store';
import { api, extractText, type Session } from '../../lib/api';
import {
  classifySession,
  labelSource,
  sessionDisplayLabel,
  sessionProjectName,
  sessionSearchText,
  type SessionKind,
} from '../../lib/sessionLabel';
import { font, radius, space, tabularNums, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Chip } from '../common/Chip';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { StateBlock } from '../common/StateBlock';

import { Tooltip } from '../common/Tooltip';
/** `42300` → `"42.3K"`, `1204000` → `"1.2M"` — the same compact form the rest
 *  of the app's stat tiles use for a large running total. */
function formatCompact(n: number): string {
  return new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(n);
}

/** ≥1600px, History becomes a Linear/Mail-style list + preview split (the
 *  coordinator's R1 ruling 2026-09-25) instead of one long single column —
 *  the width is otherwise wasted void or an over-stretched row. Below it, a
 *  single list capped at a readable width (`NARROW_LIST_MAX_WIDTH`). */
const WIDE_BREAKPOINT = 1600;
/** The list column's width at the split, and the single list's cap below it.
 *  `min(720px, 45%)` so the list never eats the majority of an ultra-wide
 *  screen just because it's there. */
const LIST_COLUMN_WIDTH = 'min(720px, 45%)';
const NARROW_LIST_MAX_WIDTH = 880;
/** Hover must LINGER before it steals the preview from the selected row —
 *  otherwise every mouse pass while scrolling to the row you actually want
 *  flickers the pane through everything in between. */
const HOVER_PREVIEW_DELAY_MS = 300;

function useWideLayout(): boolean {
  const [wide, setWide] = useState(() => typeof window !== 'undefined' && window.innerWidth >= WIDE_BREAKPOINT);
  useEffect(() => {
    const onResize = () => setWide(window.innerWidth >= WIDE_BREAKPOINT);
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return wide;
}

/** Whether `text`, laid out in `ref`'s element, overflows the box (the
 *  `scrollHeight > clientHeight` idiom already used for the chat launcher's
 *  scroll-clearance test) — i.e. whether a CSS line-clamp on it is actually
 *  cutting something, which is the one case a title needs its full text on a
 *  Tooltip (R3: never a silent cut with nothing to explain it). jsdom never
 *  lays out real box sizes, so this is always `false` in tests by
 *  construction — exercised for real in the harness, at real widths. */
function useClamped<T extends HTMLElement>(text: string): [RefObject<T>, boolean] {
  const ref = useRef<T>(null);
  const [clamped, setClamped] = useState(false);
  const measure = () => {
    const el = ref.current;
    if (el) setClamped(el.scrollHeight > el.clientHeight + 1);
  };
  useLayoutEffect(measure, [text]);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return [ref, clamped];
}

function timeAgo(dateStr: string): string {
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  const diff = now - then;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

/** History's three kinds of session, in chip order. Conversations are the
 *  default: they are what a person means by "my history". Agent runs are the
 *  coding and goal workers; System is scheduled jobs and liveness probes. */
export const SESSION_FILTERS: ReadonlyArray<{ kind: SessionKind; label: string; empty: string }> = [
  { kind: 'conversation', label: 'Conversations', empty: 'No conversations yet.' },
  { kind: 'agent_run', label: 'Agent runs', empty: 'No agent runs yet.' },
  { kind: 'system', label: 'System', empty: 'No system sessions.' },
];

interface SessionRow {
  session: SessionState;
  label: string;
  kind: SessionKind;
  project: string | null;
  haystack: string;
}

function toRow(session: SessionState): SessionRow {
  const src = labelSource(session);
  return {
    session,
    // History is roomy: show the whole first-message title (wrapped to 2
    // lines by CSS below), not the one-line 80-char cut Home Recent and the
    // chat picker use — this row has ~1500px of empty space to its right at
    // 2560px, and cutting a title with room still on the row is the R3
    // violation the coordinator's 2026-09-25 pass flagged.
    label: sessionDisplayLabel(src, { maxChars: Infinity }),
    kind: classifySession(src),
    project: sessionProjectName(src),
    haystack: sessionSearchText(src),
  };
}

/**
 * The row's title/rename field. `editing` is CONTROLLED by the parent
 * (`renamingId`, on `SessionsList`) rather than owned here — rename can now
 * be started from three places (double-click, F2, and the preview pane's
 * Rename button), and only one of them (double-click) is even on this
 * component, so one shared switch is simpler than three ways to flip a local
 * boolean.
 *
 * A single click does NOT start a rename (2026-09-25 coordinator finding:
 * clicking a row's title to select it silently turned it into an editable
 * field instead). The title has no click handler of its own any more, so a
 * plain click falls through to the row's own onClick exactly like clicking
 * anywhere else on the row — select/open, never rename.
 */
function EditableName({ value, editing, onStartEdit, onStopEditing, onSave }: {
  value: string;
  editing: boolean;
  onStartEdit: () => void;
  onStopEditing: () => void;
  onSave: (name: string) => void;
}) {
  const { colors } = useTheme();
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);
  // R3: the label is shown in full (SessionsList passes { maxChars: Infinity
  // }), wrapped to 2 lines instead of cut — so the ONLY case that still needs
  // the full text on a tooltip is a label long enough to overflow even that.
  const [labelRef, clamped] = useClamped<HTMLSpanElement>(value);

  // Reset `draft` only on the false→true transition of `editing`, never on a
  // `value` change while already editing. The row's `label` is recomputed
  // every render from the live session (toRow → sessionDisplayLabel), and a
  // `session_changed` refresh (e.g. the daemon's own auto-namer still
  // retitling a running agent session) can change it at any moment. Keying
  // an effect on `[editing, value]` re-fires on THAT change too — not only
  // when editing starts — silently overwriting whatever the user had typed
  // and re-selecting the input (REVIEW_CONFIRMED #12). `valueRef` always
  // holds the latest label so the transition still picks up a fresh value.
  const valueRef = useRef(value);
  valueRef.current = value;
  const wasEditingRef = useRef(editing);
  useEffect(() => {
    if (editing && !wasEditingRef.current) {
      setDraft(valueRef.current);
      inputRef.current?.focus();
      inputRef.current?.select();
    }
    wasEditingRef.current = editing;
  }, [editing]);

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== value) onSave(trimmed);
    onStopEditing();
  };

  if (!editing) {
    return (
      <Tooltip content={clamped ? value : 'Double-click, F2, or Rename to edit'}>
        <span
          ref={labelRef}
          data-testid="session-row-title-text"
          className="transition"
          // Double-click is the ONLY pointer path into rename. Its two
          // preceding single clicks still reach the row's onClick (open) —
          // harmless (opening what you're about to rename) and unavoidable
          // without inventing a click-timing heuristic of its own; F2 and the
          // preview's Rename button are the reliable, single-action paths.
          onDoubleClick={(e) => { e.stopPropagation(); onStartEdit(); }}
          style={{
            display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
            overflow: 'hidden', wordBreak: 'break-word', cursor: 'inherit',
          } as CSSProperties}
        >
          {value}
        </span>
      </Tooltip>
    );
  }

  return (
    <input
      ref={inputRef}
      value={draft}
      onChange={e => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={e => {
        if (e.key === 'Enter') commit();
        if (e.key === 'Escape') {
          // C5: cancel ONLY the rename — without stopPropagation the overlay's
          // window-level Escape handler also fires and closes the whole
          // surface on the same keypress (the confirmDelete guard is the
          // sibling pattern; editing state lives here, so stop the event).
          e.stopPropagation();
          onStopEditing();
        }
      }}
      onClick={e => e.stopPropagation()}
      className="bg-transparent outline-none w-full"
      style={{ borderBottom: `1px solid ${colors.cyan}80`, color: colors.text }}
    />
  );
}

/**
 * Sessions history. Browse / switch / rename / delete past conversations.
 * Hosted as the `activePanel:'sessions'` overlay — when hosted as an overlay,
 * `onClose` is provided so it offers a Close button + Escape to dismiss back to
 * the chat (mirrors SkillsPanel/InboxPanel). Selecting a session loads it into
 * the chat dock so the conversation is immediately visible.
 */
export function SessionsList({ onClose }: { onClose?: () => void } = {}) {
  const { colors } = useTheme();
  const sessions = useCommandCenter(s => s.sessions);
  const sessionsError = useCommandCenter(s => s.sessionsError);
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const loadSessions = useCommandCenter(s => s.loadSessions);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const deleteSession = useCommandCenter(s => s.deleteSession);
  const renameSession = useCommandCenter(s => s.renameSession);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  // Controlled from three places: double-click on the title, F2 while a row
  // is focused, and the preview pane's Rename button — none of them own a
  // local boolean, they all just point this at a session id (or clear it).
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [kind, setKind] = useState<SessionKind>('conversation');
  const [query, setQuery] = useState('');
  const [insights, setInsights] = useState<{ total_sessions: number; total_tokens: number } | null>(null);
  // Session ids the full-transcript search (S3) found that the client-side
  // label/first-message filter below would have missed — a match on turn 40,
  // say. `null` means "no server search has run for this query yet or it
  // failed"; failing never removes a match the client filter already found.
  const [deepMatches, setDeepMatches] = useState<Set<string> | null>(null);
  const [forking, setForking] = useState<string | null>(null);
  // The list/preview split (≥1600px): `selectedId` follows focus (click, or
  // Enter/↑/↓ from the keyboard) and persists; `hoverId` is a lingering
  // (HOVER_PREVIEW_DELAY_MS) peek that overrides it while the pointer sits on
  // a different row, and reverts the moment it leaves. Below the breakpoint
  // neither renders anything — tracked unconditionally anyway since it's free
  // and means turning the window wider doesn't lose the selection.
  const wide = useWideLayout();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => { loadSessions(); }, [loadSessions]);

  useEffect(() => {
    api.getSessionInsights().then(setInsights).catch(() => setInsights(null));
  }, [sessions.length]);

  // Debounced server-side search over full message content (api.searchSessions
  // hits /api/sessions/search). The client-side substring filter below runs
  // synchronously on every keystroke regardless — this only ever ADDS matches
  // once the daemon answers, so typing never waits on the network and an
  // offline daemon degrades to exactly the old label-only search.
  useEffect(() => {
    const q = query.trim();
    if (!q) { setDeepMatches(null); return; }
    let cancelled = false;
    const t = setTimeout(() => {
      api.searchSessions(q).then(results => {
        if (!cancelled) setDeepMatches(new Set(results.map(r => r.id)));
      }).catch(() => { if (!cancelled) setDeepMatches(null); });
    }, 250);
    return () => { cancelled = true; clearTimeout(t); };
  }, [query]);

  // One label, one classification per session (lib/sessionLabel.ts). The
  // list is the whole of GET /api/sessions — it is not paginated — so the
  // search and the counts run over every session the daemon lists.
  const rows = useMemo(() => sessions.map(toRow), [sessions]);
  const needle = query.trim().toLowerCase();
  const matching = useMemo(
    () => (needle ? rows.filter(r => r.haystack.includes(needle) || deepMatches?.has(r.session.id)) : rows),
    [rows, needle, deepMatches],
  );
  const counts = useMemo(() => {
    const c: Record<SessionKind, number> = { conversation: 0, agent_run: 0, system: 0 };
    for (const r of matching) c[r.kind] += 1;
    return c;
  }, [matching]);
  const visible = matching.filter(r => r.kind === kind);
  const activeFilter = SESSION_FILTERS.find(f => f.kind === kind) ?? SESSION_FILTERS[0];
  const previewId = hoverId ?? selectedId;
  const previewRow = visible.find(r => r.session.id === previewId);

  // A pending hover timer must not fire into an unmounted list (StrictMode
  // double-invokes effects, and closing the overlay mid-hover is routine).
  useEffect(() => () => { if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current); }, []);

  // Overlay dismissal — Escape closes back to chat, but only when hosted as an
  // overlay (onClose provided). If a delete confirmation is open, Escape clears
  // that first instead of dismissing the whole surface.
  useEffect(() => {
    if (!onClose) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      if (confirmDelete) setConfirmDelete(null);
      else onClose();
    };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [onClose, confirmDelete]);

  // Resolves `false` on failure so the Button contract never ticks on one: a
  // create that threw must not look like a create that landed.
  const handleNewSession = async () => {
    try {
      const session = await api.createSession();
      await switchToSession(session.id);
      openChatDock();
      setActivePanel('chat');
      return true;
    } catch (e) {
      console.error('Failed to create session:', e);
      return false;
    }
  };

  const handleSelect = async (sessionId: string) => {
    await switchToSession(sessionId);
    openChatDock();
    setActivePanel('chat');
  };

  // The failure sentence is `ConfirmDialog`'s now, not a toast's: the daemon
  // refusing (or never hearing) the delete leaves the session on screen, and
  // the dialog that asked the question is where that belongs. It stays open,
  // saying why, instead of closing as though the delete had happened.
  const handleDelete = async (sessionId: string) => {
    await deleteSession(sessionId);
    setConfirmDelete(null);
  };

  // Fork copies the whole conversation into a new session and opens it — a
  // safe branch point (H3 user control), never a mutation of the session the
  // row still shows. Failure stays on the row (button reverts) rather than a
  // silent no-op: nothing here pretends a fork happened when it didn't.
  const handleFork = async (sessionId: string) => {
    setForking(sessionId);
    try {
      const { sessionId: forkedId } = await api.forkSession(sessionId);
      await loadSessions();
      await handleSelect(forkedId);
    } catch (e) {
      console.error('Failed to fork session:', e);
    } finally {
      setForking(null);
    }
  };

  return (
    <div className="flex flex-col h-full" style={{ backgroundColor: colors.bg, color: colors.text }}>
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        <span className="flex items-baseline" style={{ gap: space.sm }}>
          <span
            className="text-[11px] uppercase tracking-wider"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            Sessions
          </span>
          {/* S3 compact header — a total the row-level filters below don't
              otherwise show (a search narrows what's listed; this doesn't). */}
          {insights && (
            <span
              data-testid="sessions-insights"
              style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.body }}
            >
              <span style={tabularNums}>{(insights.total_sessions ?? 0).toLocaleString()}</span> total ·{' '}
              <span style={tabularNums}>{formatCompact(insights.total_tokens ?? 0)}</span> tokens
            </span>
          )}
        </span>
        <div className="flex items-center gap-1">
          <Button
            colors={colors}
            variant="bare"
            onClick={handleNewSession}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': colors.cyanSoft,
              '--pa-btn-pad': '4px 8px',
              '--pa-btn-radius': `${radius.xs}px`,
              fontFamily: font.mono,
              fontSize: 10,
              gap: space.xs,
            } as CSSProperties}
          >
            <FiPlus size={12} /> New
          </Button>
          {onClose && (
            <Tooltip content="Close (Esc)">
              <Button
                colors={colors}
                variant="bare"
                onClick={onClose}
                aria-label="Close"
                style={{
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-bg-hover': colors.border,
                  '--pa-btn-pad': '4px',
                  '--pa-btn-radius': `${radius.xs}px`,
                } as CSSProperties}
              >
                <FiX size={14} />
              </Button>
            </Tooltip>
          )}
        </div>
      </div>

      {sessions.length > 0 && (
        <div
          data-testid="sessions-toolbar"
          style={{
            display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md,
            padding: `${space.md}px ${space.xxl}px`, borderBottom: `1px solid ${colors.border}`,
          }}
        >
          <label style={{
            display: 'flex', alignItems: 'center', gap: space.sm, flex: '1 1 200px', minWidth: 0, maxWidth: 360,
            padding: `${space.xs}px ${space.lg}px`, borderRadius: radius.sm,
            border: `1px solid ${colors.border}`, color: colors.textMuted,
          }}>
            <FiSearch size={12} style={{ flexShrink: 0 }} />
            <input
              type="search"
              data-testid="sessions-search"
              aria-label="Search sessions"
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder="Search sessions…"
              style={{
                flex: 1, minWidth: 0, fontSize: textSize.caption, fontFamily: font.body,
                background: 'transparent', border: 'none', outline: 'none', color: colors.text,
              }}
            />
          </label>
          <div role="group" aria-label="Session kind" style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm }}>
            {SESSION_FILTERS.map(f => (
              <Chip
                key={f.kind}
                kind="filter"
                tone="accent"
                pressed={kind === f.kind}
                onClick={() => setKind(f.kind)}
                data-testid={`sessions-filter-${f.kind}`}
              >
                {f.label}
                <span style={{ fontFamily: font.mono, opacity: 0.8 }}>{counts[f.kind]}</span>
              </Chip>
            ))}
          </div>
        </div>
      )}

      {/* R1 (coordinator ruling, 2026-09-25): ≥1600px is a list/preview split
          (Linear/Mail pattern) — the list column is min(720px, 45%) so it
          never eats most of an ultra-wide screen, and the rest goes to a
          preview pane instead of sitting as void or over-stretching every
          row. Below the breakpoint the list alone is capped at a readable
          width, centred, rather than spanning the full window. */}
      <div className="flex-1 flex overflow-hidden">
        <div
          ref={listRef}
          className="overflow-y-auto"
          style={wide
            ? { width: LIST_COLUMN_WIDTH, flexShrink: 0, borderRight: `1px solid ${colors.border}` }
            : { flex: 1 }}
        >
        {/* Below the breakpoint each block centres itself at a readable width
            instead of the list wrapping in an extra div — a wrapper here
            would need its own explicit height for the states' `h-full`
            centering to keep working, and this reaches the same result
            without it. */}
        {/* C6 (#568 empty-body lesson, mirrors MemoriesPanel): a failed load is
            NOT "no sessions yet" — surface the failure inline with a retry. */}
        {sessionsError && (
          <div
            className="flex flex-col items-center justify-center h-full text-xs gap-2 p-4 text-center"
            style={{ fontFamily: font.mono }}
          >
            <span style={{ color: colors.danger }}>Couldn't load sessions — the daemon may be unreachable.</span>
            <Button
              colors={colors}
              variant="bare"
              className="hover:underline"
              onClick={() => loadSessions()}
              style={{
                '--pa-btn-fg': colors.cyan,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-weight': 600,
                fontSize: 'inherit',
              } as CSSProperties}
            >
              Retry
            </Button>
          </div>
        )}

        {!sessionsError && sessions.length === 0 && (
          <div
            className="flex flex-col items-center justify-center h-full text-xs gap-2 p-4 text-center"
            style={{ fontFamily: font.mono, color: colors.textMuted }}
          >
            <FiMessageSquare size={20} className="opacity-30" />
            <div>No sessions yet.</div>
            <Button
              colors={colors}
              variant="bare"
              className="hover:underline"
              onClick={handleNewSession}
              style={{
                '--pa-btn-fg': colors.cyan,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                fontSize: 'inherit',
              } as CSSProperties}
            >
              Click + New to start a chat.
            </Button>
          </div>
        )}

        {!sessionsError && sessions.length > 0 && visible.length === 0 && (
          <div
            data-testid="sessions-filter-empty"
            className="flex flex-col items-center justify-center text-center"
            style={{ padding: space.huge, gap: space.sm, fontSize: textSize.caption, color: colors.textMuted }}
          >
            {needle ? `No ${activeFilter.label.toLowerCase()} match “${query.trim()}”.` : activeFilter.empty}
          </div>
        )}

        {visible.map(({ session: s, label, project }) => {
          const isActive = s.id === chatSessionId;
          const goalProjectId = s.goal_project_id ?? s.project_id ?? null;
          return (
            <div
              key={s.id}
              data-testid="session-row"
              role="button"
              tabIndex={0}
              aria-current={isActive ? 'true' : undefined}
              aria-selected={wide && previewId === s.id ? 'true' : undefined}
              onClick={() => handleSelect(s.id)}
              // Selection (for the ≥1600px preview pane) follows focus, so a
              // click, a Tab and an arrow-key move all agree on what's
              // selected without three separate code paths.
              onFocus={() => setSelectedId(s.id)}
              // A lingering peek: hovering a DIFFERENT row schedules the
              // preview to follow it after HOVER_PREVIEW_DELAY_MS; leaving
              // that row cancels the timer and, if it had already won,
              // reverts the preview to the selected row. Only one timer is
              // ever live — entering a new row clears whatever the last one
              // scheduled, so a fast pass over many rows never flickers.
              onMouseEnter={() => {
                if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
                hoverTimerRef.current = setTimeout(() => setHoverId(s.id), HOVER_PREVIEW_DELAY_MS);
              }}
              onMouseLeave={() => {
                if (hoverTimerRef.current) { clearTimeout(hoverTimerRef.current); hoverTimerRef.current = null; }
                setHoverId(h => (h === s.id ? null : h));
              }}
              // R4: a row that looks clickable is keyboard-operable too — Enter
              // and Space activate it exactly like a click, the same contract
              // the shared Button gives every other control on this screen.
              // ↑/↓ move focus (= selection) to the next/previous row without
              // opening it — only Enter/Space open, per the coordinator's
              // 2026-09-25 ruling.
              onKeyDown={(e) => {
                if (e.target !== e.currentTarget) return; // let the inner rename input/buttons handle their own keys
                if (e.key === 'F2') { e.preventDefault(); setRenamingId(s.id); return; }
                if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); handleSelect(s.id); return; }
                if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
                e.preventDefault();
                const rowEls = Array.from(listRef.current?.querySelectorAll<HTMLElement>('[data-testid="session-row"]') ?? []);
                const idx = rowEls.indexOf(e.currentTarget);
                rowEls[idx + (e.key === 'ArrowDown' ? 1 : -1)]?.focus();
              }}
              // R8: pa-pressable gives the row the shared hover lift / press dip.
              className={`pa-pressable group flex items-center gap-3 px-4 py-3 cursor-pointer transition ${
                isActive ? '' : 'hover:bg-white/[0.03]'
              }`}
              style={{
                borderBottom: `1px solid ${colors.border}`,
                // isActive (open in chat right now) and "selected" (the
                // preview pane's target) get the SAME strong treatment — the
                // Settings nav rail's cyanSoft fill + a left accent bar,
                // instead of the barely-visible neutral fillHover selection
                // previously had (2026-09-25 coordinator finding).
                backgroundColor: isActive || (wide && previewId === s.id) ? colors.cyanSoft : undefined,
                borderLeft: isActive || (wide && previewId === s.id) ? `2px solid ${colors.cyan}` : undefined,
                // Below the split, the row centres at a readable width
                // instead of a single line stretching the full window (R1).
                ...(wide ? null : { maxWidth: NARROW_LIST_MAX_WIDTH, margin: '0 auto' }),
              }}
            >
              <div className="flex-1 min-w-0">
                {/* items-start, not items-baseline: the title can now wrap to
                    2 lines (R3 — the full label, not an 80-char cut with
                    empty row space beside it), and baseline alignment would
                    have floated the timestamp against the title's first line
                    while its second line hung below alone. */}
                <div className="flex items-start gap-2">
                  <span
                    className="flex-1"
                    data-testid="session-row-title"
                    style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 500, color: colors.text }}
                  >
                    <EditableName
                      value={label}
                      editing={renamingId === s.id}
                      onStartEdit={() => setRenamingId(s.id)}
                      onStopEditing={() => setRenamingId(null)}
                      onSave={(name) => renameSession(s.id, name)}
                    />
                  </span>
                  {s.updated_at && (
                    <span
                      className="shrink-0"
                      style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted }}
                    >
                      {timeAgo(s.updated_at)}
                    </span>
                  )}
                </div>
                {/* Row metadata: project, goal, size. Readable size and tone —
                    it was 10px mono at textMuted, the faintest text on the
                    screen, for the only facts that tell two rows apart. */}
                <div
                  data-testid="session-row-meta"
                  style={{
                    display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.sm,
                    marginTop: space.xs, fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted,
                  }}
                >
                  {project && (
                    <Chip kind="static" quiet title={`Project: ${project}`} data-testid="session-row-project">
                      {project}
                    </Chip>
                  )}
                  {s.goal_id && goalProjectId && (
                    // The row opens the session; the goal chip opens the goal,
                    // so its click must not also reach the row.
                    <span onClick={e => e.stopPropagation()} style={{ display: 'inline-flex', minWidth: 0 }}>
                      <Chip
                        kind="link"
                        tone="accent"
                        quiet
                        title="Open this goal"
                        onClick={() => openGoalDetail(goalProjectId, s.goal_id!)}
                        data-testid="session-row-goal"
                      >
                        {/* Truncate on an inner block span, not the Chip's own
                            flex shell: text-overflow needs a block box, and
                            the shared .pa-btn rule centers flex content, so
                            clipping the shell itself cuts BOTH ends of the
                            text instead of ellipsizing the tail. */}
                        <span style={{
                          maxWidth: 220, overflow: 'hidden', textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap', display: 'inline-block', verticalAlign: 'bottom',
                        }}>
                          Goal: {s.goal_title?.trim() || 'open'}
                        </span>
                      </Chip>
                    </span>
                  )}
                  <span>{s.message_count} message{s.message_count !== 1 ? 's' : ''}</span>
                </div>
              </div>

              <div className="flex items-center opacity-0 group-hover:opacity-100" style={{ gap: space.xxs }}>
                <Tooltip content="Fork — copy into a new session">
                  <Button
                    colors={colors}
                    variant="bare"
                    disabled={forking === s.id}
                    onClick={(e) => { e.stopPropagation(); handleFork(s.id); }}
                    aria-label="Fork session"
                    data-testid="session-row-fork"
                    style={{
                      '--pa-btn-fg': colors.textMuted,
                      '--pa-btn-fg-hover': colors.cyan,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-pad': '4px',
                      '--pa-btn-radius': `${radius.xs}px`,
                    } as CSSProperties}
                  >
                    <FiGitBranch size={12} />
                  </Button>
                </Tooltip>
                <Tooltip content="Delete session">
                  <Button
                    colors={colors}
                    variant="bare"
                    onClick={(e) => { e.stopPropagation(); setConfirmDelete(s.id); }}
                    aria-label="Delete session"
                    style={{
                      '--pa-btn-fg': colors.textMuted,
                      '--pa-btn-fg-hover': colors.danger,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-pad': '4px',
                      '--pa-btn-radius': `${radius.xs}px`,
                    } as CSSProperties}
                  >
                    <FiTrash2 size={12} />
                  </Button>
                </Tooltip>
              </div>
            </div>
          );
        })}
        </div>
        {wide && (
          <div className="flex-1 overflow-y-auto">
            <SessionPreviewPane
              row={previewRow}
              onOpen={handleSelect}
              onFork={handleFork}
              onDelete={setConfirmDelete}
              onRename={() => previewRow && setRenamingId(previewRow.session.id)}
              onOpenGoal={openGoalDetail}
              forking={forking}
            />
          </div>
        )}
      </div>

      {/* Deleting a conversation has no undo, which is the tier that earns a
          full-attention modal. It used to be hand-rolled chrome with no focus
          trap, no dialog role and no way back to the button that opened it. */}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete session?"
          consequence="This permanently deletes the conversation and every message in it. There is no undo."
          confirmLabel="Delete"
          failureLabel="Couldn't delete the session"
          onConfirm={() => handleDelete(confirmDelete)}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}

// ── the ≥1600px preview pane ─────────────────────────────────────────────────

/** `GET /api/sessions/{id}` for the preview pane, cached per session id so
 *  re-hovering a row already seen doesn't refetch, and race-safe: a response
 *  for an id that isn't the current one any more (rapid hover, or the user
 *  switched rows before the daemon answered) is dropped. */
const PREVIEW_CACHE_TTL_MS = 15_000;

function useSessionDetail(id: string | null) {
  const cache = useRef<Map<string, { at: number; session: Session }>>(new Map());
  const [reloadKey, setReloadKey] = useState(0);
  const [state, setState] = useState<{ data: Session | null; loading: boolean; error: boolean }>({
    data: null, loading: false, error: false,
  });

  useEffect(() => {
    if (!id) { setState({ data: null, loading: false, error: false }); return; }
    // A cached copy older than the TTL is refetched: another device (or a
    // server-side truncation) may have changed the session since it was seen.
    const cached = cache.current.get(id);
    if (cached && Date.now() - cached.at < PREVIEW_CACHE_TTL_MS) { setState({ data: cached.session, loading: false, error: false }); return; }
    let cancelled = false;
    setState({ data: null, loading: true, error: false });
    api.getSession(id).then(s => {
      if (cancelled) return;
      cache.current.set(id, { at: Date.now(), session: s });
      setState({ data: s, loading: false, error: false });
    }).catch(() => {
      if (!cancelled) setState({ data: null, loading: false, error: true });
    });
    return () => { cancelled = true; };
  }, [id, reloadKey]);

  const retry = () => {
    if (id) cache.current.delete(id);
    setReloadKey(k => k + 1);
  };

  return { ...state, retry };
}

/** The first user-visible user message and the last agent-visible assistant
 *  reply — the two lines that tell you what a session was actually about
 *  without opening it. Hidden preambles (a coding directive's boilerplate,
 *  liveness-probe scaffolding) are skipped the same way the daemon's own
 *  `first_user_text` column does. */
function firstUserText(s: Session): string | null {
  const m = (s.conversation ?? []).find(m => m.role === 'user' && m.metadata?.userVisible !== false);
  return m ? extractText(m) || null : null;
}

function lastAssistantText(s: Session): string | null {
  const conv = s.conversation ?? [];
  for (let i = conv.length - 1; i >= 0; i--) {
    const m = conv[i];
    if (m.role === 'assistant' && m.metadata?.agentVisible !== false) {
      const t = extractText(m);
      if (t) return t;
    }
  }
  return null;
}

function PreviewSkeleton() {
  const { colors } = useTheme();
  return (
    <div aria-hidden="true" data-testid="session-preview-skeleton" style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
      {[95, 70, 85, 55].map((w, i) => (
        <div key={i} style={{ height: 12, width: `${w}%`, borderRadius: radius.xs, background: colors.border }} />
      ))}
    </div>
  );
}

function PreviewMessage({ label, text }: { label: string; text: string | null }) {
  const { colors } = useTheme();
  if (!text) return null;
  return (
    <div>
      <div style={{
        fontSize: textSize.micro, fontWeight: 600, color: colors.textMuted,
        marginBottom: space.xxs, textTransform: 'uppercase', letterSpacing: '0.06em',
      }}>
        {label}
      </div>
      <div style={{
        fontSize: textSize.caption, color: colors.text, lineHeight: 1.5, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
        display: '-webkit-box', WebkitLineClamp: 6, WebkitBoxOrient: 'vertical', overflow: 'hidden',
      } as CSSProperties}>
        {text}
      </div>
    </div>
  );
}

/** The right-hand pane of the ≥1600px split: everything the row itself has no
 *  room for — the whole label, both link chips, an absolute created/updated
 *  time on hover, message/token counts, the first and last message, and the
 *  row's own actions (Open/Fork/Rename/Delete) so opening or acting on a
 *  session never requires going back to the list. Covers all five R2 states:
 *  empty (nothing selected/hovered), loading (skeleton, not a spinner),
 *  error (with retry), and populated. */
function SessionPreviewPane({ row, onOpen, onFork, onDelete, onRename, onOpenGoal, forking }: {
  row: SessionRow | undefined;
  onOpen: (id: string) => void;
  onFork: (id: string) => void;
  onDelete: (id: string) => void;
  onRename: () => void;
  onOpenGoal: (projectId: string, cardId: string) => void;
  forking: string | null;
}) {
  const { colors } = useTheme();
  const id = row?.session.id ?? null;
  const { data, loading, error, retry } = useSessionDetail(id);

  if (!row) {
    return (
      <div style={{ padding: space.xxl, height: '100%', display: 'flex', alignItems: 'center' }}>
        <StateBlock
          tone="empty"
          title="Select a session to preview"
          detail="Click a row, or arrow through the list, to see its detail here."
        />
      </div>
    );
  }

  const s = row.session;
  const goalProjectId = s.goal_project_id ?? s.project_id ?? null;
  const kindLabel = SESSION_FILTERS.find(f => f.kind === row.kind)?.label ?? 'Session';
  const tokens = data ? (data.accumulated_total_tokens ?? data.total_tokens ?? null) : null;

  return (
    <div data-testid="session-preview" style={{
      padding: space.xxl, display: 'flex', flexDirection: 'column', gap: space.lg,
      height: '100%', boxSizing: 'border-box', overflowY: 'auto',
    }}>
      {/* A readable measure, not the full pane width — at 2560px the pane is
          well over 1300px wide, and running text edge-to-edge at that width
          is unreadable regardless of how good the content is (2026-09-25
          coordinator finding). The chips/title/actions share it too, so the
          column reads as one thing instead of a wide header over a narrow
          column of text. */}
      <div style={{ maxWidth: '72ch', display: 'flex', flexDirection: 'column', gap: space.lg }}>
        <div>
          <div style={{ display: 'flex', alignItems: 'center', gap: space.sm, flexWrap: 'wrap' }}>
            <Chip kind="static" quiet>{kindLabel}</Chip>
            {row.project && <Chip kind="static" quiet title={`Project: ${row.project}`}>{row.project}</Chip>}
            {s.goal_id && goalProjectId && (
              <Chip kind="link" tone="accent" quiet title="Open this goal" onClick={() => onOpenGoal(goalProjectId, s.goal_id!)}>
                Goal: {s.goal_title?.trim() || 'open'}
              </Chip>
            )}
          </div>
          <h2 data-testid="session-preview-title" style={{
            marginTop: space.sm, marginBottom: 0, fontFamily: font.body, fontSize: textSize.heading,
            fontWeight: 600, color: colors.text, lineHeight: 1.35,
          }}>
            {row.label}
          </h2>
          <div style={{ display: 'flex', gap: space.sm, marginTop: space.xxs, fontSize: textSize.micro, color: colors.textMuted }}>
            {s.created_at && (
              <Tooltip content={new Date(s.created_at).toLocaleString()}>
                <span>Created {timeAgo(s.created_at)}</span>
              </Tooltip>
            )}
            {s.updated_at && (
              <Tooltip content={new Date(s.updated_at).toLocaleString()}>
                <span>· Updated {timeAgo(s.updated_at)}</span>
              </Tooltip>
            )}
          </div>

          {/* Actions directly under the title, not ~900px below the content
              at 2560px (2026-09-25 coordinator finding) — Open is the one
              tinted primary action (D8), Fork and Rename are neutral, Delete
              reads as danger at rest (not only on hover) since it's now a
              standing header control rather than a faint row-hover icon, and
              still goes through the same ConfirmDialog before anything
              happens. */}
          <div style={{ display: 'flex', gap: space.sm, marginTop: space.md }}>
            <Button
              colors={colors}
              variant="primary"
              onClick={() => onOpen(s.id)}
              style={{ '--pa-btn-pad': '6px 14px', '--pa-btn-radius': `${radius.sm}px`, fontSize: textSize.caption } as CSSProperties}
            >
              Open
            </Button>
            <Button
              colors={colors}
              variant="bare"
              disabled={forking === s.id}
              onClick={() => onFork(s.id)}
              style={{
                '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.cyan, '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-border': colors.border, '--pa-btn-border-hover': colors.borderHi,
                '--pa-btn-pad': '6px 14px', '--pa-btn-radius': `${radius.sm}px`, fontSize: textSize.caption,
              } as CSSProperties}
            >
              <FiGitBranch size={12} /> Fork
            </Button>
            <Button
              colors={colors}
              variant="bare"
              onClick={onRename}
              style={{
                '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.cyan, '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-border': colors.border, '--pa-btn-border-hover': colors.borderHi,
                '--pa-btn-pad': '6px 14px', '--pa-btn-radius': `${radius.sm}px`, fontSize: textSize.caption,
              } as CSSProperties}
            >
              Rename
            </Button>
            <Button
              colors={colors}
              variant="bare"
              onClick={() => onDelete(s.id)}
              style={{
                '--pa-btn-fg': colors.danger, '--pa-btn-bg-hover': `${colors.danger}1A`, '--pa-btn-bg-active': `${colors.danger}26`,
                '--pa-btn-border': `${colors.danger}40`, '--pa-btn-border-hover': colors.danger,
                '--pa-btn-pad': '6px 14px', '--pa-btn-radius': `${radius.sm}px`, fontSize: textSize.caption,
              } as CSSProperties}
            >
              <FiTrash2 size={12} /> Delete
            </Button>
          </div>
        </div>

        {loading && <PreviewSkeleton />}
        {error && <StateBlock tone="error" title="Couldn't load this session" detail="The daemon didn't answer." onRetry={retry} compact />}
        {data && !loading && !error && (
          <>
            <div style={{ display: 'flex', gap: space.lg, fontSize: textSize.micro, color: colors.textMuted, ...tabularNums }}>
              <span>{data.message_count} message{data.message_count !== 1 ? 's' : ''}</span>
              {typeof tokens === 'number' && <span>{formatCompact(tokens)} tokens</span>}
            </div>
            <PreviewMessage label="First message" text={firstUserText(data)} />
            <PreviewMessage label="Last reply" text={lastAssistantText(data)} />
          </>
        )}
      </div>
    </div>
  );
}
