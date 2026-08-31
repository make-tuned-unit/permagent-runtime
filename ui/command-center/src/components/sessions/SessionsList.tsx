import { useEffect, useState, useRef, type CSSProperties } from 'react';
import { FiPlus, FiTrash2, FiMessageSquare, FiX } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { toast } from '../../lib/notifications';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

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
      <span
        className="truncate cursor-pointer transition"
        onClick={(e) => { e.stopPropagation(); setEditing(true); }}
        onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; }}
        onMouseLeave={e => { e.currentTarget.style.color = ''; }}
        title="Click to rename"
      >
        {value}
      </span>
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
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  useEffect(() => { loadSessions(); }, [loadSessions]);

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

  const handleDelete = async (sessionId: string) => {
    setConfirmDelete(null);
    try {
      await deleteSession(sessionId);
    } catch (e) {
      // The daemon refused (or never heard) the delete — the session still
      // exists and the store kept the open conversation intact; say so.
      toast("Couldn't delete session", e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="flex flex-col h-full" style={{ backgroundColor: colors.bg, color: colors.text }}>
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        <span
          className="text-[11px] uppercase tracking-wider"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Sessions
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
              gap: 4,
            } as CSSProperties}
          >
            <FiPlus size={12} /> New
          </Button>
          {onClose && (
            <Button
              colors={colors}
              variant="bare"
              onClick={onClose}
              title="Close (Esc)"
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
          )}
        </div>
      </div>

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

        {sessions.map(s => {
          const isActive = s.id === chatSessionId;
          return (
            <div
              key={s.id}
              onClick={() => handleSelect(s.id)}
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
                  <span className="text-[12px] truncate flex-1" style={{ fontFamily: font.mono, color: colors.text }}>
                    <EditableName
                      value={s.name || s.id.slice(0, 12)}
                      onSave={(name) => renameSession(s.id, name)}
                    />
                  </span>
                  {s.updated_at && (
                    <span
                      className="text-[9px] shrink-0"
                      style={{ fontFamily: font.mono, color: colors.textMuted, opacity: 0.7 }}
                    >
                      {timeAgo(s.updated_at)}
                    </span>
                  )}
                </div>
                <div className="text-[10px] mt-0.5" style={{ fontFamily: font.mono, color: colors.textMuted }}>
                  {s.message_count} message{s.message_count !== 1 ? 's' : ''}
                </div>
              </div>

              <Button
                colors={colors}
                variant="bare"
                className="opacity-0 group-hover:opacity-100"
                onClick={(e) => { e.stopPropagation(); setConfirmDelete(s.id); }}
                title="Delete session"
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
            </div>
          );
        })}
      </div>

      {/* Delete confirmation */}
      {confirmDelete && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => setConfirmDelete(null)}>
          <div
            className="rounded-xl p-5 max-w-sm"
            style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.cardShadow }}
            onClick={e => e.stopPropagation()}
          >
            <h3 className="mb-2" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>Delete session?</h3>
            <p className="text-xs mb-4" style={{ fontFamily: font.body, color: colors.textMuted }}>This will permanently delete this session and its messages.</p>
            <div className="flex justify-end gap-2">
              <Button
                colors={colors}
                onClick={() => setConfirmDelete(null)}
                style={{
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-border': colors.border,
                  '--pa-btn-border-hover': colors.border,
                  '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': `${radius.xs}px`,
                  fontSize: 14,
                } as CSSProperties}
              >
                Cancel
              </Button>
              <Button
                colors={colors}
                onClick={() => handleDelete(confirmDelete)}
                style={{
                  '--pa-btn-bg': `${colors.danger}33`,
                  '--pa-btn-fg': colors.danger,
                  '--pa-btn-border': 'transparent',
                  '--pa-btn-bg-hover': `${colors.danger}4D`,
                  '--pa-btn-border-hover': 'transparent',
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': `${radius.xs}px`,
                  fontSize: 14,
                } as CSSProperties}
              >
                Delete
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
