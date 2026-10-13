/**
 * World W2 — a field note: kept on this device as you type, saved to a project
 * ONLY when you choose a project and press "Save field note to project".
 * Uses W1's `saveFieldNoteToProject`, the one write the World performs.
 */
import { useId, useMemo, useState } from 'react';
import { Button } from '../../../common/Button';
import type { ThemeColors } from '../../../../styles/tokens';
import { saveFieldNoteToProject, type FieldNote, type WorldDraftStore } from '../data/drafts';
import type { WorldNoteWriter } from '../data/evidenceClient';
import type { WorldPlaceId } from '../data/identity';
import type { WorldRecordLink } from '../data/recordLinks';
import { ageLabel } from './format';
import type { PlaceEvent, PlaceProject } from './integration';
import type { PlaceStyles } from './placeStyles';

export interface FieldNoteEditorProps {
  placeId: WorldPlaceId;
  /** Record link keys this note is about. */
  evidenceRefs: readonly string[];
  drafts: WorldDraftStore;
  noteWriter: WorldNoteWriter;
  projects: readonly PlaceProject[];
  defaultProjectId: string | null;
  newId(): string;
  onEvent?(event: PlaceEvent): void;
  onOpenLink(link: WorldRecordLink): void;
  now: number;
  colors: ThemeColors;
  styles: PlaceStyles;
}

function latestUnsaved(drafts: WorldDraftStore, placeId: WorldPlaceId): FieldNote | null {
  const mine = drafts.notes().notes.filter((n) => n.placeId === placeId && !n.saved);
  return mine.length ? mine[mine.length - 1] : null;
}

export function FieldNoteEditor({ placeId, evidenceRefs, drafts, noteWriter, projects, defaultProjectId, newId, onEvent, onOpenLink, now, colors, styles }: FieldNoteEditorProps) {
  const ids = { body: useId(), project: useId() };
  // Reopening the World continues the last unsaved note rather than losing it.
  const [note, setNote] = useState<FieldNote | null>(() => latestUnsaved(drafts, placeId));
  const [body, setBody] = useState(() => note?.body ?? '');
  const [noteId, setNoteId] = useState(() => note?.id ?? newId());
  const [target, setTarget] = useState<string>(defaultProjectId ?? '');
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [savedTick, setSavedTick] = useState(0);
  const refs = Array.from(new Set([...(note?.evidenceRefs ?? []), ...evidenceRefs]));

  const history = useMemo(() => drafts.notes().notes.filter((n) => n.placeId === placeId).slice(-4).reverse(), [drafts, placeId, savedTick, note]);

  const keep = (text: string) => {
    setBody(text);
    setMessage(null);
    if (!text.trim() && !note) return;
    setNote(drafts.addNote(placeId, text, [...refs], noteId));
  };

  const save = async () => {
    if (!note || !target || saving) return;
    setSaving(true);
    const result = await saveFieldNoteToProject(drafts, noteWriter, note, target);
    setSaving(false);
    if (result.ok) {
      const projectName = projects.find((p) => p.id === target)?.name ?? target;
      setMessage({ ok: true, text: `Saved to ${projectName}.` });
      onEvent?.({ type: 'note_saved', placeId, projectId: target, noteId: result.note.saved!.noteId });
      setNote(null);
      setBody('');
      setNoteId(newId());
      setSavedTick((t) => t + 1);
    } else {
      setMessage({ ok: false, text: result.reason });
      onEvent?.({ type: 'note_save_failed', placeId, reason: result.reason });
    }
  };

  const disabledReason = !body.trim() ? 'Write the note first.' : !target ? 'Choose a project to save to.' : null;

  return (
    <div style={styles.field} data-field-note="true">
      <label htmlFor={ids.body} style={styles.label}>Field note — your own inference, kept on this device until you save it</label>
      <textarea id={ids.body} rows={3} style={styles.input} value={body} onChange={(e) => keep(e.target.value)} />
      {refs.length > 0 && <span style={styles.optionMeta}>About: {refs.join(', ')}</span>}
      <div style={styles.actions}>
        <label htmlFor={ids.project} style={styles.label}>Save to</label>
        <select id={ids.project} style={{ ...styles.input, width: 'auto' }} value={target} onChange={(e) => setTarget(e.target.value)}>
          <option value="">Choose a project…</option>
          {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
        <Button colors={colors} variant="ghostOn" flashSuccess={false} minPendingMs={0} pending={saving} data-action="save_field_note" disabled={!!disabledReason} onClick={() => { void save(); }}>Save field note to project</Button>
      </div>
      {disabledReason && body.trim() && <span style={styles.note}>{disabledReason}</span>}
      {projects.length === 0 && <span style={styles.note}>No projects to save to yet. The note stays on this device.</span>}
      {message && <p style={message.ok ? styles.para : styles.warn} role={message.ok ? 'status' : 'alert'}>{message.text}</p>}
      {history.length > 0 && (
        <ul style={styles.list} aria-label="Your field notes here">
          {history.map((n) => (
            <li key={n.id} style={styles.row} data-note={n.id} data-saved={n.saved ? 'true' : 'false'}>
              <span style={styles.para}>{n.body || '(empty)'}</span>
              <span style={styles.optionMeta}>
                {n.saved ? `Saved to ${projects.find((p) => p.id === n.saved!.projectId)?.name ?? n.saved.projectId} ${ageLabel(n.saved.savedAt, now)}` : `On this device only · ${ageLabel(n.createdAt, now)}`}
              </span>
              {n.saved && <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => onOpenLink({ to: 'project_note', projectId: n.saved!.projectId, noteId: n.saved!.noteId })}>Open note</Button></span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
