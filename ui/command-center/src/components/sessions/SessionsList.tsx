import { useEffect, useState, useRef } from 'react';
import { FiPlus, FiTrash2, FiMessageSquare } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';

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
        className="truncate cursor-pointer hover:text-accent transition"
        onClick={(e) => { e.stopPropagation(); setEditing(true); }}
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
      onKeyDown={e => { if (e.key === 'Enter') commit(); if (e.key === 'Escape') { setDraft(value); setEditing(false); } }}
      onClick={e => e.stopPropagation()}
      className="bg-transparent border-b border-accent/50 outline-none text-dark-text w-full"
    />
  );
}

export function SessionsList() {
  const sessions = useCommandCenter(s => s.sessions);
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const loadSessions = useCommandCenter(s => s.loadSessions);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const deleteSession = useCommandCenter(s => s.deleteSession);
  const renameSession = useCommandCenter(s => s.renameSession);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  useEffect(() => { loadSessions(); }, [loadSessions]);

  const handleNewSession = async () => {
    try {
      const session = await api.createSession();
      await switchToSession(session.id);
      setActivePanel('chat');
    } catch (e) {
      console.error('Failed to create session:', e);
    }
  };

  const handleSelect = async (sessionId: string) => {
    await switchToSession(sessionId);
    setActivePanel('chat');
  };

  const handleDelete = async (sessionId: string) => {
    setConfirmDelete(null);
    await deleteSession(sessionId);
  };

  return (
    <div className="flex flex-col h-full bg-[#0B1120] text-dark-text">
      <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
        <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">Sessions</span>
        <button
          onClick={handleNewSession}
          className="flex items-center gap-1 text-[10px] font-mono text-accent hover:text-accent/80 transition px-2 py-1 rounded hover:bg-accent/10"
        >
          <FiPlus size={12} /> New
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {sessions.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-dark-muted text-xs font-mono gap-2 p-4 text-center">
            <FiMessageSquare size={20} className="opacity-30" />
            <div>No sessions yet.</div>
            <button
              onClick={handleNewSession}
              className="text-accent hover:underline"
            >
              Click + New to start a chat.
            </button>
          </div>
        )}

        {sessions.map(s => {
          const isActive = s.id === chatSessionId;
          return (
            <div
              key={s.id}
              onClick={() => handleSelect(s.id)}
              className={`group flex items-center gap-3 px-4 py-3 cursor-pointer border-b border-dark-border/50 transition ${
                isActive ? 'bg-accent/5 border-l-2 border-l-accent' : 'hover:bg-white/[0.03]'
              }`}
            >
              <div className="flex-1 min-w-0">
                <div className="flex items-baseline gap-2">
                  <span className="text-[12px] font-mono text-dark-text truncate flex-1">
                    <EditableName
                      value={s.name || s.id.slice(0, 12)}
                      onSave={(name) => renameSession(s.id, name)}
                    />
                  </span>
                  {s.updated_at && (
                    <span className="text-[9px] font-mono text-dark-muted/50 shrink-0">
                      {timeAgo(s.updated_at)}
                    </span>
                  )}
                </div>
                <div className="text-[10px] font-mono text-dark-muted mt-0.5">
                  {s.message_count} message{s.message_count !== 1 ? 's' : ''}
                </div>
              </div>

              <button
                onClick={(e) => { e.stopPropagation(); setConfirmDelete(s.id); }}
                className="opacity-0 group-hover:opacity-100 text-dark-muted hover:text-red-400 transition p-1 rounded"
                title="Delete session"
              >
                <FiTrash2 size={12} />
              </button>
            </div>
          );
        })}
      </div>

      {/* Delete confirmation */}
      {confirmDelete && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => setConfirmDelete(null)}>
          <div className="bg-[#111827] rounded-xl border border-dark-border p-5 max-w-sm shadow-2xl" onClick={e => e.stopPropagation()}>
            <h3 className="font-semibold mb-2">Delete session?</h3>
            <p className="text-xs text-dark-muted mb-4">This will permanently delete this session and its messages.</p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setConfirmDelete(null)} className="px-3 py-1.5 text-sm rounded border border-dark-border text-dark-muted hover:bg-white/5 transition">Cancel</button>
              <button onClick={() => handleDelete(confirmDelete)} className="px-3 py-1.5 text-sm rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 transition">Delete</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
