/**
 * World W1 — per-place drafts and field notes.
 *
 * Contract (Astra ruling §1, "State, persistence, and approval rules"):
 * - A draft is per place + project + agent context. Reopening the World
 *   restores it WITHOUT sending it. Nothing in this module sends anything.
 * - A field note stays local until an explicit Save to an explicitly chosen
 *   project (`saveFieldNoteToProject`, the only function here that writes to
 *   the daemon, and only through the `WorldNoteWriter` it is handed).
 * - Demo answers are never written to the Brain from here.
 * - Replay-safe: parsing the same stored payload twice yields deep-equal state;
 *   a corrupt or future-version payload is reported and ignored, never thrown.
 *
 * Storage: localStorage keys under `permagent.world.*`, each a versioned
 * envelope `{ v: 1, … }`. Storage can throw (private mode, blocked site data)
 * — every access falls back to an in-memory value, as worldRoute.ts does.
 */
import type { WorldNoteWriter } from './evidenceClient';
import { isWorldPlaceId, type WorldPlaceId } from './identity';
import { parseRecordLinkKey } from './recordLinks';

export const DRAFTS_STORAGE_KEY = 'permagent.world.drafts';
export const FIELD_NOTES_STORAGE_KEY = 'permagent.world.fieldNotes';
export const DRAFTS_VERSION = 1;

/** Bounds for persisted user text. Generous for writing, bounded for storage. */
export const DRAFT_BOUNDS = {
  briefChars: 4000,
  constraintChars: 300,
  constraints: 8,
  acceptanceChars: 1000,
  evidenceRefs: 12,
  /** Drafts kept; the least recently edited is dropped first. */
  drafts: 24,
  noteChars: 2000,
  /** Local field notes kept; the oldest unsaved note is dropped first. */
  notes: 50,
} as const;

export interface PlaceDraft {
  placeId: WorldPlaceId;
  projectId: string | null;
  /** World roster id of the agent selected for context (never an assignment). */
  agentWorldId: string;
  /** The outcome wanted, in the user's words. */
  brief: string;
  constraints: string[];
  /** What would count as done. */
  acceptance: string;
  /** recordLinkKey()s of evidence the user attached. */
  evidenceRefs: string[];
  updatedAt: string;
  /** Set only by markDraftSent after an explicit send was acknowledged. */
  sent: { at: string; acknowledgement: string } | null;
}

export interface FieldNote {
  id: string;
  placeId: WorldPlaceId;
  body: string;
  evidenceRefs: string[];
  createdAt: string;
  /** null until an explicit Save to a chosen project succeeded. */
  saved: { projectId: string; noteId: string; savedAt: string } | null;
}

export type LoadIssue = 'none' | 'empty' | 'corrupt' | 'future_version' | 'storage_unavailable';

export interface DraftState {
  drafts: Record<string, PlaceDraft>;
  issue: LoadIssue;
}

export interface FieldNoteState {
  notes: FieldNote[];
  issue: LoadIssue;
}

/** Context key: place | project | agent. `-` marks "no project chosen". */
export function draftKey(placeId: WorldPlaceId, projectId: string | null, agentWorldId: string): string {
  return `${placeId}|${projectId ?? '-'}|${agentWorldId}`;
}

const clip = (s: unknown, max: number): string => (typeof s === 'string' ? Array.from(s).slice(0, max).join('') : '');
const refs = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((r): r is string => typeof r === 'string' && parseRecordLinkKey(r) !== null).slice(0, DRAFT_BOUNDS.evidenceRefs) : [];

function sanitizeDraft(raw: unknown): PlaceDraft | null {
  if (!raw || typeof raw !== 'object') return null;
  const d = raw as Record<string, unknown>;
  if (typeof d.placeId !== 'string' || !isWorldPlaceId(d.placeId) || typeof d.agentWorldId !== 'string' || typeof d.updatedAt !== 'string') return null;
  const sent = d.sent && typeof d.sent === 'object' && typeof (d.sent as Record<string, unknown>).at === 'string'
    ? { at: (d.sent as Record<string, string>).at, acknowledgement: clip((d.sent as Record<string, unknown>).acknowledgement, 500) }
    : null;
  return {
    placeId: d.placeId,
    projectId: typeof d.projectId === 'string' && d.projectId ? d.projectId : null,
    agentWorldId: d.agentWorldId,
    brief: clip(d.brief, DRAFT_BOUNDS.briefChars),
    constraints: Array.isArray(d.constraints)
      ? d.constraints.filter((c): c is string => typeof c === 'string' && c.trim() !== '').slice(0, DRAFT_BOUNDS.constraints).map((c) => clip(c, DRAFT_BOUNDS.constraintChars))
      : [],
    acceptance: clip(d.acceptance, DRAFT_BOUNDS.acceptanceChars),
    evidenceRefs: refs(d.evidenceRefs),
    updatedAt: d.updatedAt,
    sent,
  };
}

function sanitizeNote(raw: unknown): FieldNote | null {
  if (!raw || typeof raw !== 'object') return null;
  const n = raw as Record<string, unknown>;
  if (typeof n.id !== 'string' || typeof n.placeId !== 'string' || !isWorldPlaceId(n.placeId) || typeof n.createdAt !== 'string') return null;
  const s = n.saved as Record<string, unknown> | null | undefined;
  return {
    id: n.id,
    placeId: n.placeId,
    body: clip(n.body, DRAFT_BOUNDS.noteChars),
    evidenceRefs: refs(n.evidenceRefs),
    createdAt: n.createdAt,
    saved: s && typeof s.projectId === 'string' && typeof s.noteId === 'string' && typeof s.savedAt === 'string'
      ? { projectId: s.projectId, noteId: s.noteId, savedAt: s.savedAt }
      : null,
  };
}

function parseEnvelope(raw: string | null): { body: Record<string, unknown> | null; issue: LoadIssue } {
  if (raw === null || raw === '') return { body: null, issue: 'empty' };
  let parsed: unknown;
  try { parsed = JSON.parse(raw); } catch { return { body: null, issue: 'corrupt' }; }
  if (!parsed || typeof parsed !== 'object' || typeof (parsed as { v?: unknown }).v !== 'number') return { body: null, issue: 'corrupt' };
  const v = (parsed as { v: number }).v;
  if (v > DRAFTS_VERSION) return { body: null, issue: 'future_version' };
  if (v !== DRAFTS_VERSION) return { body: null, issue: 'corrupt' };
  return { body: parsed as Record<string, unknown>, issue: 'none' };
}

/** Pure: stored string → draft state. Deterministic (replay gate). */
export function parseDrafts(raw: string | null): DraftState {
  const { body, issue } = parseEnvelope(raw);
  const drafts: Record<string, PlaceDraft> = {};
  const src = body?.drafts;
  if (src && typeof src === 'object') {
    for (const value of Object.values(src as Record<string, unknown>)) {
      const d = sanitizeDraft(value);
      if (d) drafts[draftKey(d.placeId, d.projectId, d.agentWorldId)] = d;
    }
  }
  return { drafts, issue };
}

/** Pure: draft state → stored string, keeping at most DRAFT_BOUNDS.drafts. */
export function serializeDrafts(state: Pick<DraftState, 'drafts'>): string {
  const kept = Object.values(state.drafts)
    .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : a.updatedAt > b.updatedAt ? -1 : 0))
    .slice(0, DRAFT_BOUNDS.drafts);
  const drafts: Record<string, PlaceDraft> = {};
  for (const d of kept.sort((a, b) => draftKey(a.placeId, a.projectId, a.agentWorldId).localeCompare(draftKey(b.placeId, b.projectId, b.agentWorldId)))) {
    drafts[draftKey(d.placeId, d.projectId, d.agentWorldId)] = d;
  }
  return JSON.stringify({ v: DRAFTS_VERSION, drafts });
}

export function parseFieldNotes(raw: string | null): FieldNoteState {
  const { body, issue } = parseEnvelope(raw);
  const notes = Array.isArray(body?.notes) ? (body!.notes as unknown[]).map(sanitizeNote).filter((n): n is FieldNote => n !== null) : [];
  return { notes, issue };
}

export function serializeFieldNotes(state: Pick<FieldNoteState, 'notes'>): string {
  let notes = [...state.notes];
  if (notes.length > DRAFT_BOUNDS.notes) {
    // Drop oldest unsaved local notes first: a saved note also lives on the daemon.
    const saved = notes.filter((n) => n.saved);
    const local = notes.filter((n) => !n.saved).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    while (saved.length + local.length > DRAFT_BOUNDS.notes && saved.length > 0) saved.sort((a, b) => a.createdAt.localeCompare(b.createdAt)).shift();
    while (saved.length + local.length > DRAFT_BOUNDS.notes) local.shift();
    notes = [...saved, ...local];
  }
  notes.sort((a, b) => a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id));
  return JSON.stringify({ v: DRAFTS_VERSION, notes });
}

// ── Storage ────────────────────────────────────────────────────────────────

export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** localStorage when usable, else an in-memory map (never throws). */
export function worldStorage(): KeyValueStorage & { persistent: boolean } {
  try {
    const ls = globalThis.localStorage;
    if (ls) {
      const probe = '__permagent.world.probe__';
      ls.setItem(probe, '1');
      ls.removeItem(probe);
      return { getItem: (k) => { try { return ls.getItem(k); } catch { return null; } }, setItem: (k, v) => { try { ls.setItem(k, v); } catch { /* full or blocked: keep in memory */ } }, persistent: true };
    }
  } catch { /* fall through */ }
  const mem = new Map<string, string>();
  return { getItem: (k) => mem.get(k) ?? null, setItem: (k, v) => { mem.set(k, v); }, persistent: false };
}

export interface DraftEdit {
  brief?: string;
  constraints?: string[];
  acceptance?: string;
  evidenceRefs?: string[];
}

/**
 * Draft store over a storage. Every method is local: none takes a client, so
 * none can send. W2 wires `restore` to place selection and `update` to typing.
 */
export function createDraftStore(storage: KeyValueStorage, clock: () => string = () => new Date().toISOString()) {
  const read = (): DraftState => parseDrafts(storage.getItem(DRAFTS_STORAGE_KEY));
  const write = (s: Pick<DraftState, 'drafts'>) => storage.setItem(DRAFTS_STORAGE_KEY, serializeDrafts(s));
  const readNotes = (): FieldNoteState => parseFieldNotes(storage.getItem(FIELD_NOTES_STORAGE_KEY));
  const writeNotes = (s: Pick<FieldNoteState, 'notes'>) => storage.setItem(FIELD_NOTES_STORAGE_KEY, serializeFieldNotes(s));

  return {
    /** Selecting a place restores its draft. Reads only; never sends. */
    restore(placeId: WorldPlaceId, projectId: string | null, agentWorldId: string): PlaceDraft | null {
      return read().drafts[draftKey(placeId, projectId, agentWorldId)] ?? null;
    },
    update(placeId: WorldPlaceId, projectId: string | null, agentWorldId: string, edit: DraftEdit): PlaceDraft {
      const state = read();
      const key = draftKey(placeId, projectId, agentWorldId);
      const prev = state.drafts[key];
      const next = sanitizeDraft({
        placeId, projectId, agentWorldId,
        brief: edit.brief ?? prev?.brief ?? '',
        constraints: edit.constraints ?? prev?.constraints ?? [],
        acceptance: edit.acceptance ?? prev?.acceptance ?? '',
        evidenceRefs: edit.evidenceRefs ?? prev?.evidenceRefs ?? [],
        updatedAt: clock(),
        // Editing after a send starts a new, unsent draft.
        sent: null,
      })!;
      write({ drafts: { ...state.drafts, [key]: next } });
      return next;
    },
    /** Record an acknowledged explicit send. Call only after the send resolved. */
    markSent(placeId: WorldPlaceId, projectId: string | null, agentWorldId: string, acknowledgement: string): PlaceDraft | null {
      const state = read();
      const key = draftKey(placeId, projectId, agentWorldId);
      const prev = state.drafts[key];
      if (!prev) return null;
      const next: PlaceDraft = { ...prev, sent: { at: clock(), acknowledgement: clip(acknowledgement, 500) } };
      write({ drafts: { ...state.drafts, [key]: next } });
      return next;
    },
    discard(placeId: WorldPlaceId, projectId: string | null, agentWorldId: string): void {
      const state = read();
      const drafts = { ...state.drafts };
      delete drafts[draftKey(placeId, projectId, agentWorldId)];
      write({ drafts });
    },
    all(): DraftState { return read(); },

    notes(): FieldNoteState { return readNotes(); },
    /** Keep a local note. Local only: nothing reaches the Brain or a project. */
    addNote(placeId: WorldPlaceId, body: string, evidenceRefs: string[], id: string): FieldNote {
      const state = readNotes();
      const note = sanitizeNote({ id, placeId, body, evidenceRefs, createdAt: clock(), saved: null })!;
      writeNotes({ notes: [...state.notes.filter((n) => n.id !== id), note] });
      return note;
    },
    recordSaved(noteId: string, projectId: string, savedNoteId: string): FieldNote | null {
      const state = readNotes();
      let hit: FieldNote | null = null;
      const notes = state.notes.map((n) => (n.id === noteId ? (hit = { ...n, saved: { projectId, noteId: savedNoteId, savedAt: clock() } }) : n));
      if (hit) writeNotes({ notes });
      return hit;
    },
  };
}

export type WorldDraftStore = ReturnType<typeof createDraftStore>;

/**
 * Compose the explicit request text from a draft, for `askFromForum(question,
 * agentName, criteria, …)`. Pure: composing is not sending.
 */
export function composeBriefRequest(draft: Pick<PlaceDraft, 'brief' | 'constraints' | 'acceptance' | 'evidenceRefs'>): { question: string; criteria: string } {
  const parts = [draft.brief.trim()];
  const constraints = draft.constraints.map((c) => c.trim()).filter(Boolean);
  if (constraints.length) parts.push(`Constraints:\n${constraints.map((c) => `- ${c}`).join('\n')}`);
  if (draft.evidenceRefs.length) parts.push(`Evidence I selected (World record references):\n${draft.evidenceRefs.map((r) => `- ${r}`).join('\n')}`);
  return { question: parts.filter(Boolean).join('\n\n'), criteria: draft.acceptance.trim() };
}

/**
 * The ONE write: save a local field note to an explicitly chosen project.
 * Requires a non-empty project id; never called by selection, restore or load.
 */
export async function saveFieldNoteToProject(
  store: WorldDraftStore,
  writer: WorldNoteWriter,
  note: FieldNote,
  projectId: string,
): Promise<{ ok: true; note: FieldNote } | { ok: false; reason: string }> {
  if (!projectId) return { ok: false, reason: 'Choose a project to save this note to.' };
  if (!note.body.trim()) return { ok: false, reason: 'Write the note first.' };
  if (note.saved) return { ok: true, note };
  try {
    const refsText = note.evidenceRefs.length ? `\n\nEvidence: ${note.evidenceRefs.join(', ')}` : '';
    const saved = await writer.createProjectNote(projectId, { title: 'World field note', body: `${note.body}${refsText}` });
    const recorded = store.recordSaved(note.id, projectId, saved.id);
    return recorded ? { ok: true, note: recorded } : { ok: false, reason: 'Saved to the project, but the local note was no longer present.' };
  } catch (error) {
    return { ok: false, reason: `Not saved: ${error instanceof Error ? error.message : String(error)}. Your note is still here.` };
  }
}
