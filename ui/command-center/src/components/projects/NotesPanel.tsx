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
 * Dictation (voice → note text): the mic button records a short clip via the
 * Web Audio API, encodes it to WAV in-browser, and POSTs it to
 * `/api/dictation/transcribe` for local (on-device) Whisper transcription; the
 * returned text is appended to the composer so the user can speak a note
 * instead of typing it (see useDictation). If dictation isn't set up on the
 * install the endpoint answers 503 and the panel shows a gentle setup hint.
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiTrash2, FiMic, FiSquare, FiLoader, FiExternalLink, FiCopy, FiCheck, FiChevronRight } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { useDictation } from '../../hooks/useDictation';
import { font, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import type { Project, ProjectNote } from './types';

export function NotesPanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const focusBrainMemory = useCommandCenter(s => s.focusBrainMemory);
  const [notes, setNotes] = useState<ProjectNote[]>([]);
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  // Notes collapse to their title row so long notes can't turn the panel into
  // an infinite scroll; expansion is per-note.
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  // "Note saved" deep link: expand + scroll + briefly highlight the target
  // note once a loaded list actually contains it (the pending id is held
  // until then — the toast can win the race with the daemon's refetch bump).
  const pendingNote = useCommandCenter(s => s.pendingNoteNavigation);
  const clearPendingNoteNavigation = useCommandCenter(s => s.clearPendingNoteNavigation);
  const [highlightId, setHighlightId] = useState<string | null>(null);
  const rowEls = useRef(new Map<string, HTMLDivElement>());
  useEffect(() => {
    if (!pendingNote || pendingNote.projectId !== project.id) return;
    const { noteId } = pendingNote;
    if (!notes.some(n => n.id === noteId)) return; // hold until it loads
    setExpanded(prev => new Set(prev).add(noteId));
    setHighlightId(noteId);
    clearPendingNoteNavigation();
    requestAnimationFrame(() => {
      rowEls.current.get(noteId)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    });
    const t = setTimeout(() => setHighlightId(null), 2600);
    return () => clearTimeout(t);
  }, [pendingNote, notes, project.id, clearPendingNoteNavigation]);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const loadGeneration = useRef(0);

  useEffect(() => () => { if (copiedTimer.current) clearTimeout(copiedTimer.current); }, []);

  const toggleExpanded = (id: string) => {
    setExpanded(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  // One-click copy of the full note (title + body) — for pasting straight
  // into a coding agent. Mirrors the backend's note_memory_content shape.
  const copyNote = async (note: ProjectNote) => {
    const text = note.title ? `${note.title}\n\n${note.body}` : note.body;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedId(note.id);
      if (copiedTimer.current) clearTimeout(copiedTimer.current);
      copiedTimer.current = setTimeout(() => setCopiedId(null), 1500);
      return true;
    } catch {
      setError("Couldn't copy note to clipboard");
      return false;
    }
  };

  // Dictation: append transcribed speech to the composer body.
  const appendDictation = useCallback((text: string) => {
    setBody(prev => (prev.trim() ? `${prev.trim()} ${text}` : text));
  }, []);
  const { state: dictation, error: dictationError, toggle: toggleDictation } = useDictation(appendDictation);

  // #629 multi-client liveness: `project_changed` (change=notes) from another
  // device refetches this list.
  const projectsRev = useCommandCenter(s => s.projectsRev);

  // Resolves `false` when the load failed (or was superseded) so the retry
  // button can only tick over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    try {
      const nextNotes = await api.listProjectNotes(project.id);
      if (generation !== loadGeneration.current) return false;
      if (!Array.isArray(nextNotes)) throw new Error('Invalid notes response');
      setNotes(nextNotes);
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      setStatus('error');
      return false;
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load, projectsRev]);

  // Resolves `false` on failure: the error is surfaced inline, so the Save
  // button must not tick over a note that never persisted.
  const save = useCallback(async () => {
    const trimmed = body.trim();
    if (!trimmed || saving) return false;
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
      return true;
    } catch (e) {
      setError(`Couldn't save note: ${(e as Error).message || 'request failed'}`);
      return false;
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
      return true;
    } catch (e) {
      setError(`Couldn't delete note: ${(e as Error).message || 'request failed'}`);
      return false;
    }
  };

  // Close the loop: focus this note's Brain memory. The note's own text is the
  // preview so the Brain renders it even before the Librarian enriches it (fresh,
  // description-less writes aren't in the graph yet); the live graph copy wins
  // once present. `text` mirrors the backend's note_memory_content(title, body).
  const viewInBrain = (note: ProjectNote) => {
    if (!note.memory_key) return;
    const text = note.title ? `${note.title}\n\n${note.body}` : note.body;
    focusBrainMemory({ key: note.memory_key, preview: { text, description: null, timestamp: note.created_at } });
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
            fontSize: textSize.caption, padding: '6px 9px', borderRadius: 7,
            background: colors.inputBg, border: `1px solid ${colors.border}`,
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
            fontSize: textSize.caption, padding: '7px 9px', borderRadius: 7, resize: 'vertical', minHeight: 56,
            background: colors.inputBg, border: `1px solid ${colors.border}`,
            color: colors.text, fontFamily: font.body, lineHeight: 1.5, outline: 'none',
          }}
        />
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <Button
            colors={colors}
            variant="ghostOn"
            onClick={save}
            disabled={!body.trim() || saving}
            style={{
              '--pa-btn-bg': colors.cyanSoft,
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-pad': '6px 14px',
              '--pa-btn-radius': '7px',
              '--pa-btn-weight': 600,
              fontFamily: font.body,
              fontSize: textSize.caption,
            } as CSSProperties}
          >
            {saving ? 'Saving…' : 'Save note'}
          </Button>
          <Button
            colors={colors}
            variant="ghost"
            onClick={toggleDictation}
            disabled={dictation === 'transcribing'}
            title={
              dictation === 'recording' ? 'Stop and transcribe'
                : dictation === 'transcribing' ? 'Transcribing…'
                : 'Dictate a note'
            }
            aria-label="Dictate a note"
            style={{
              '--pa-btn-bg': dictation === 'recording' ? colors.danger : rowVeil,
              '--pa-btn-fg': dictation === 'recording' ? colors.textOnAccent : colors.textDim,
              '--pa-btn-border': dictation === 'recording' ? colors.danger : colors.border,
              '--pa-btn-bg-hover': dictation === 'recording' ? colors.danger : colors.surfaceHi,
              '--pa-btn-fg-hover': dictation === 'recording' ? colors.textOnAccent : colors.text,
              '--pa-btn-border-hover': dictation === 'recording' ? colors.danger : colors.borderHi,
              '--pa-btn-pad': '0',
              '--pa-btn-radius': '7px',
              width: 30,
              height: 30,
            } as CSSProperties}
          >
            {dictation === 'transcribing'
              ? <FiLoader size={13} className="pa-spin" />
              : dictation === 'recording'
                ? <FiSquare size={12} />
                : <FiMic size={13} />}
          </Button>
          <span style={{ fontSize: 10, color: colors.textDim }}>
            {dictation === 'recording' ? 'Recording — tap to stop'
              : dictation === 'transcribing' ? 'Transcribing…'
              : '⌘↵ to save'}
          </span>
        </div>
      </div>

      {(error || dictationError) && (
        <div style={{ fontSize: textSize.micro, color: colors.danger, marginBottom: 8 }}>{error || dictationError}</div>
      )}

      {status === 'loading' && (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>Loading notes…</div>
      )}

      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: textSize.micro, color: colors.danger }}>Couldn't load notes.</span>
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={load}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              fontFamily: font.body,
              fontSize: textSize.micro,
            } as CSSProperties}
          >
            Retry
          </Button>
        </div>
      )}

      {status === 'ready' && notes.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {notes.map(note => {
            const isOpen = expanded.has(note.id);
            return (
              <div
                key={note.id}
                ref={el => {
                  if (el) rowEls.current.set(note.id, el);
                  else rowEls.current.delete(note.id);
                }}
                style={{
                  borderRadius: 7, background: rowVeil,
                  border: `1px solid ${highlightId === note.id ? colors.cyan : colors.border}`,
                  transition: 'border-color 600ms',
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = highlightId === note.id ? colors.cyan : colors.border; }}
              >
                {/* Title row — the whole row toggles; icon buttons stop propagation. */}
                <div
                  role="button"
                  aria-expanded={isOpen}
                  onClick={() => toggleExpanded(note.id)}
                  onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleExpanded(note.id); } }}
                  tabIndex={0}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 7, padding: '8px 10px',
                    cursor: 'pointer', userSelect: 'none',
                  }}
                >
                  <FiChevronRight
                    size={12}
                    style={{
                      color: colors.textDim, flexShrink: 0,
                      transform: isOpen ? 'rotate(90deg)' : 'none', transition: 'transform 150ms',
                    }}
                  />
                  <div style={{
                    flex: 1, minWidth: 0, fontSize: textSize.caption, fontWeight: 600, color: colors.text,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                  }}>
                    {note.title || firstLine(note.body)}
                  </div>
                  <span style={{ fontSize: 10, color: colors.textDim, flexShrink: 0 }}>{relativeTime(note.created_at)}</span>
                  <Button
                    colors={colors}
                    variant="bare"
                    // Deliberately does not hand the promise back: this button
                    // already answers with its own ✓/copy icon swap, so a
                    // spinner and a tick over the top would say it twice.
                    onClick={e => { e.stopPropagation(); copyNote(note); }}
                    title="Copy note"
                    aria-label="Copy note"
                    style={{
                      '--pa-btn-fg': copiedId === note.id ? colors.cyan : colors.textDim,
                      '--pa-btn-fg-hover': colors.cyan,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-pad': '2px',
                      flexShrink: 0,
                    } as CSSProperties}
                  >
                    {copiedId === note.id ? <FiCheck size={13} /> : <FiCopy size={13} />}
                  </Button>
                  <Button
                    colors={colors}
                    variant="bare"
                    onClick={e => { e.stopPropagation(); return remove(note); }}
                    title="Delete note"
                    aria-label="Delete note"
                    style={{
                      '--pa-btn-fg': colors.textDim,
                      '--pa-btn-fg-hover': colors.danger,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-pad': '2px',
                      flexShrink: 0,
                    } as CSSProperties}
                  >
                    <FiTrash2 size={13} />
                  </Button>
                </div>
                {isOpen && (
                  <div style={{ padding: '0 10px 8px 29px' }}>
                    <div style={{
                      fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5, whiteSpace: 'pre-wrap',
                      overflowWrap: 'anywhere',
                    }}>
                      {note.body}
                    </div>
                    {note.memory_key && (
                      <div style={{ marginTop: 4 }}>
                        <Button
                          colors={colors}
                          variant="bare"
                          // `contents` dissolves Button's `.pa-btn__label`
                          // wrapper so the label and its icon stay the
                          // button's own centred flex row, gap and all.
                          className="hover:underline"
                          onClick={() => viewInBrain(note)}
                          title="View this note in your Brain"
                          style={{
                            '--pa-btn-fg': colors.cyan,
                            '--pa-btn-bg-hover': 'transparent',
                            '--pa-btn-pad': '0',
                            '--pa-btn-weight': 600,
                            gap: 4,
                            fontFamily: font.body,
                            fontSize: 10,
                          } as CSSProperties}
                        >
                          View in Brain <FiExternalLink size={9} />
                        </Button>
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </Panel>
  );
}

/** Collapsed-row label for an untitled note: its first non-empty line. */
function firstLine(body: string): string {
  const line = body.split('\n').find(l => l.trim());
  return line ? line.trim() : '(empty note)';
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
