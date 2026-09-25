import { useEffect, useMemo, useState, useRef, type CSSProperties } from 'react';
import { FiPlus, FiTrash2, FiMessageSquare, FiX, FiSearch, FiGitBranch } from 'react-icons/fi';
import { useCommandCenter, type SessionState } from '../../lib/store';
import { api } from '../../lib/api';
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

import { Tooltip } from '../common/Tooltip';
/** `42300` → `"42.3K"`, `1204000` → `"1.2M"` — the same compact form the rest
 *  of the app's stat tiles use for a large running total. */
function formatCompact(n: number): string {
  return new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(n);
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
    label: sessionDisplayLabel(src),
    kind: classifySession(src),
    project: sessionProjectName(src),
    haystack: sessionSearchText(src),
  };
}

function EditableName({ value, onSave }: { value: string; onSave: (name: string) => void }) {
  const { colors } = useTheme();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  const commit = () => {
    setEditing(false);
    const trimmed = draft.trim();
    if (trimmed && trimmed !== value) onSave(trimmed);
    else setDraft(value);
  };

  if (!editing) {
    return (
      <Tooltip content="Click to rename">
        <span
          className="truncate cursor-pointer transition"
          onClick={(e) => { e.stopPropagation(); setEditing(true); }}
          onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; }}
          onMouseLeave={e => { e.currentTarget.style.color = ''; }}
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
          setDraft(value);
          setEditing(false);
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
  const [kind, setKind] = useState<SessionKind>('conversation');
  const [query, setQuery] = useState('');
  const [insights, setInsights] = useState<{ total_sessions: number; total_tokens: number } | null>(null);
  // Session ids the full-transcript search (S3) found that the client-side
  // label/first-message filter below would have missed — a match on turn 40,
  // say. `null` means "no server search has run for this query yet or it
  // failed"; failing never removes a match the client filter already found.
  const [deepMatches, setDeepMatches] = useState<Set<string> | null>(null);
  const [forking, setForking] = useState<string | null>(null);

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
              <span style={tabularNums}>{insights.total_sessions.toLocaleString()}</span> total ·{' '}
              <span style={tabularNums}>{formatCompact(insights.total_tokens)}</span> tokens
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

      <div className="flex-1 overflow-y-auto">
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
              onClick={() => handleSelect(s.id)}
              // R4: a row that looks clickable is keyboard-operable too — Enter
              // and Space activate it exactly like a click, the same contract
              // the shared Button gives every other control on this screen.
              onKeyDown={(e) => {
                if (e.target !== e.currentTarget) return; // let the inner rename input/buttons handle their own keys
                if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); handleSelect(s.id); }
              }}
              className={`group flex items-center gap-3 px-4 py-3 cursor-pointer transition ${
                isActive ? '' : 'hover:bg-white/[0.03]'
              }`}
              style={{
                borderBottom: `1px solid ${colors.border}`,
                backgroundColor: isActive ? `${colors.cyan}0D` : undefined,
                borderLeft: isActive ? `2px solid ${colors.cyan}` : undefined,
              }}
            >
              <div className="flex-1 min-w-0">
                <div className="flex items-baseline gap-2">
                  <span
                    className="truncate flex-1"
                    data-testid="session-row-title"
                    style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 500, color: colors.text }}
                  >
                    <EditableName
                      value={label}
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
