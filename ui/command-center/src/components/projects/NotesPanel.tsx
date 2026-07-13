/**
 * NotesPanel — the Notes panel on a project's Overview.
 *
 * A freeform note composer (optional title + body + Save) over the project's
 * note list (`GET /api/projects/{id}/notes`). Saving POSTs the note, which the
 * backend indexes into the Brain (best-effort) so it becomes recallable +
 * Librarian-enriched, scoped to the project — the same pipeline a dropped
 * document rides. Delete removes the row and best-effort disassociates its
 * Brain memory.
 *
 * Observability (per the #568 empty-body lesson): save/delete/list failures
 * surface inline rather than a silent catch. Styled strictly with the shared
 * Panel shell + theme tokens to match the surrounding Overview panels.
 *
 * Dictation (voice → note text) is a deliberate follow-up: the only voice input
 * today is the push-to-talk chat loop (a full STT→reply→TTS round-trip in
 * useVoice), which isn't a reusable "give me back text" primitive, and no HTTP
 * transcription endpoint is exposed to the frontend yet. Rather than ship a
 * fragile new audio pipeline, this panel ships the write path; a Dictate button
 * lands once a plain transcribe endpoint exists.
 */

import { useCallback, useEffect, useState } from 'react';
import { FiTrash2 } from 'react-icons/fi';
import { api } from '../../lib/api';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Panel } from './Panel';
import type { Project, ProjectNote } from './types';

export function NotesPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const [notes, setNotes] = useState<ProjectNote[]>([]);
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');

  const load = useCallback(async () => {
    try {
      setNotes(await api.listProjectNotes(project.id));
      setStatus('ready');
    } catch {
      setStatus('error');
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  const save = useCallback(async () => {
    const trimmed = body.trim();
    if (!trimmed || saving) return;
    setError(null);
    setSaving(true);
    try {
      const created = await api.createProjectNote(project.id, {
        title: title.trim() || undefined,
        body: trimmed,
      });
      // Optimistic prepend (newest first), then clear the composer.
      setNotes(prev => [created, ...prev]);
      setTitle('');
      setBody('');
    } catch (e) {
      setError(`Couldn't save note: ${(e as Error).message || 'request failed'}`);
    } finally {
      setSaving(false);
    }
  }, [project.id, title, body, saving]);

  const remove = async (note: ProjectNote) => {
    setError(null);
    try {
      const res = await api.deleteProjectNote(project.id, note.id);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setNotes(prev => prev.filter(n => n.id !== note.id));
    } catch (e) {
      setError(`Couldn't delete note: ${(e as Error).message || 'request failed'}`);
    }
  };

  return (
    <Panel
      title="Notes"
      action={<span style={{ fontSize: 10, color: colors.textDim }}>{notes.length} note{notes.length !== 1 ? 's' : ''}</span>}
    >
      {/* Composer */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginBottom: notes.length ? 10 : 0 }}>
        <input
          value={title}
          onChange={e => setTitle(e.target.value)}
          placeholder="Title (optional)"
          style={{
            fontSize: 12, padding: '6px 9px', borderRadius: 7,
            background: 'rgba(255,255,255,0.02)', border: `1px solid ${colors.border}`,
            color: colors.text, fontFamily: font.body, outline: 'none',
          }}
        />
        <textarea
          value={body}
          onChange={e => setBody(e.target.value)}
          onKeyDown={e => {
            // Cmd/Ctrl+Enter saves — quick capture without reaching for the mouse.
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); save(); }
          }}
          placeholder="Write a note… it lands in your project's Brain."
          rows={3}
          style={{
            fontSize: 12, padding: '7px 9px', borderRadius: 7, resize: 'vertical', minHeight: 56,
            background: 'rgba(255,255,255,0.02)', border: `1px solid ${colors.border}`,
            color: colors.text, fontFamily: font.body, lineHeight: 1.5, outline: 'none',
          }}
        />
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <button
            onClick={save}
            disabled={!body.trim() || saving}
            style={{
              fontSize: 12, fontWeight: 600, padding: '6px 14px', borderRadius: 7,
              cursor: (!body.trim() || saving) ? 'default' : 'pointer',
              opacity: (!body.trim() || saving) ? 0.5 : 1,
              background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
              color: colors.cyan, fontFamily: font.body,
            }}
          >
            {saving ? 'Saving…' : 'Save note'}
          </button>
          <span style={{ fontSize: 10, color: colors.textDim }}>⌘↵ to save</span>
        </div>
      </div>

      {error && (
        <div style={{ fontSize: 11, color: colors.danger, marginBottom: 8 }}>{error}</div>
      )}

      {status === 'loading' && (
        <div style={{ fontSize: 11, color: colors.textDim }}>Loading notes…</div>
      )}

      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: 11, color: colors.danger }}>Couldn't load notes.</span>
          <button
            onClick={load}
            style={{
              fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
              cursor: 'pointer', fontFamily: font.body, padding: 0, fontWeight: 600,
            }}
          >
            Retry
          </button>
        </div>
      )}

      {status === 'ready' && notes.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {notes.map(note => (
            <div
              key={note.id}
              style={{
                display: 'flex', alignItems: 'flex-start', gap: 8, padding: '8px 10px',
                borderRadius: 7, background: 'rgba(255,255,255,0.02)', border: `1px solid ${colors.border}`,
                transition: 'border-color 150ms',
              }}
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                {note.title && (
                  <div style={{ fontSize: 12, fontWeight: 600, color: colors.text, marginBottom: 2 }}>
                    {note.title}
                  </div>
                )}
                <div style={{
                  fontSize: 12, color: colors.textMuted, lineHeight: 1.5, whiteSpace: 'pre-wrap',
                  overflowWrap: 'anywhere',
                }}>
                  {note.body}
                </div>
                <div style={{ fontSize: 10, color: colors.textDim, marginTop: 4 }}>
                  {relativeTime(note.created_at)}
                </div>
              </div>
              <button
                onClick={() => remove(note)}
                title="Delete note"
                style={{ background: 'none', border: 'none', color: colors.textDim, cursor: 'pointer', padding: 2, display: 'flex', flexShrink: 0 }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.color = colors.danger; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.color = colors.textDim; }}
              >
                <FiTrash2 size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

/** Compact relative time ("just now", "3h ago", "2d ago"), falling back to a
 *  date for older notes. The backend stores an ISO-8601 UTC timestamp. */
function relativeTime(iso: string): string {
  const then = new Date(iso.endsWith('Z') || iso.includes('+') ? iso : `${iso}Z`).getTime();
  if (Number.isNaN(then)) return iso;
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 45) return 'just now';
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 7) return `${days}d ago`;
  try {
    return new Date(then).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}
