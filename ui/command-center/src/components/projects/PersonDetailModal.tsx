/**
 * PersonDetailModal (CRM epic slice 2a read view + slice 2b manual edit) — the
 * detail view for one person, opened from a project's Overview People panel or
 * from the global people directory.
 *
 * Read view renders the person's typed CRM fields (role / company / email /
 * phone / birthday / relationship / how-met / notes). When opened from a
 * project it also shows that project's role and association time, carried in
 * the `association` prop the panel already fetched; from the directory there is
 * no project context, so those rows and the Remove-from-project action are
 * absent rather than blank.
 *
 * Slice 2b (#495) adds an **Edit** mode: the typed fields become inputs and Save
 * writes them via `PATCH /api/people/{entity_uuid}/fields`. Those land in the
 * authoritative graph with **manual provenance** (`FieldSource::Manual`), so a
 * later Enricher pass can never clobber them. The edit is optimistic — inputs
 * apply to the view immediately and roll back on a non-2xx — and the response is
 * the re-overlaid person, i.e. the graph's own truth, which we merge back in.
 * On success we bump the store's people revision so the decoupled panel refetches.
 *
 * "Refresh enrichment" (#495 slice 4) mutates nothing here: it copies a prepared
 * prompt and navigates to chat; enriched writes happen only after the user
 * approves the resulting Decision Inbox proposal. Disassociate (DELETE
 * /api/projects/{id}/people/{entity_uuid}, #530) is the other mutation.
 *
 * PersonDetailModalHost is mounted once at the app root and renders whenever the
 * store's `personDetail` target is set.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { FiBookOpen, FiCheckSquare, FiFileText, FiPlus, FiTrash2 } from 'react-icons/fi';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DetailModal } from '../common/DetailModal';
import type { Person, PersonActivity, PersonAssociation, PersonRelationship } from './types';

function fmtTime(iso: string | null): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  return Number.isFinite(t) ? new Date(t).toLocaleString() : iso;
}

/** The person fields this modal lets the user edit, in display order. `notes`
 *  renders as a textarea; the rest as single-line inputs. Field names match the
 *  backend `PERSON_FIELD_NAMES` vocabulary exactly (the wire keys). */
const EDITABLE_FIELDS: { key: EditableKey; label: string; multiline?: boolean; placeholder?: string }[] = [
  { key: 'role', label: 'Role' },
  { key: 'company', label: 'Company' },
  { key: 'email', label: 'Email', placeholder: 'name@example.com' },
  { key: 'phone', label: 'Phone' },
  { key: 'birthday', label: 'Birthday', placeholder: 'YYYY-MM-DD' },
  { key: 'relationship_strength', label: 'Relationship' },
  { key: 'how_met', label: 'How met' },
  { key: 'notes', label: 'Notes', multiline: true },
];

type EditableKey =
  | 'role'
  | 'company'
  | 'email'
  | 'phone'
  | 'birthday'
  | 'relationship_strength'
  | 'how_met'
  | 'notes';

type Draft = Record<EditableKey, string>;

function draftFrom(p: Person): Draft {
  return {
    role: p.role ?? '',
    company: p.company ?? '',
    email: p.email ?? '',
    phone: p.phone ?? '',
    birthday: p.birthday ?? '',
    relationship_strength: p.relationship_strength ?? '',
    how_met: p.how_met ?? '',
    notes: p.notes ?? '',
  };
}

export function PersonDetailModal({
  projectId,
  person,
  association,
  onClose,
}: {
  /** Null when opened from the global directory — there is no project context. */
  projectId: string | null;
  person: Person;
  /** Null outside a project; gates the project-role badge and Disassociate. */
  association?: PersonAssociation | null;
  onClose: () => void;
}) {
  const { colors } = useTheme();
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [promptCopied, setPromptCopied] = useState(false);

  // Local view of the person: seeded from the prop, updated optimistically on a
  // field edit and reconciled from the PATCH response (the graph's own truth).
  const [view, setView] = useState<Person>(person);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>(() => draftFrom(person));
  const [saving, setSaving] = useState(false);
  const [relationships, setRelationships] = useState<PersonRelationship[]>([]);
  const [activity, setActivity] = useState<PersonActivity[]>([]);
  const [allPeople, setAllPeople] = useState<Person[]>([]);
  const [relatedStatus, setRelatedStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [activityStatus, setActivityStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [addingRelationship, setAddingRelationship] = useState(false);
  const [targetId, setTargetId] = useState('');
  const [predicate, setPredicate] = useState('related_to');
  const relationshipsGeneration = useRef(0);
  const activityGeneration = useRef(0);

  const loadRelationships = useCallback(async () => {
    const generation = ++relationshipsGeneration.current;
    setRelatedStatus('loading');
    try {
      const [edges, people] = await Promise.all([
        apiFetch<PersonRelationship[]>(`/api/people/${encodeURIComponent(view.entity_uuid)}/relationships`),
        apiFetch<Person[]>('/api/people'),
      ]);
      if (generation !== relationshipsGeneration.current) return;
      if (!Array.isArray(edges) || !Array.isArray(people)) throw new Error('Invalid relationships response');
      setRelationships(edges);
      setAllPeople(people.filter(p => p.entity_uuid !== view.entity_uuid));
      setRelatedStatus('ready');
    } catch {
      if (generation === relationshipsGeneration.current) setRelatedStatus('error');
    }
  }, [view.entity_uuid]);

  const loadActivity = useCallback(async () => {
    const generation = ++activityGeneration.current;
    setActivityStatus('loading');
    try {
      const nextActivity = await apiFetch<PersonActivity[]>(`/api/people/${encodeURIComponent(view.entity_uuid)}/activity`);
      if (generation !== activityGeneration.current) return;
      if (!Array.isArray(nextActivity)) throw new Error('Invalid activity response');
      setActivity(nextActivity);
      setActivityStatus('ready');
    } catch {
      if (generation === activityGeneration.current) setActivityStatus('error');
    }
  }, [view.entity_uuid]);

  useEffect(() => { loadRelationships(); loadActivity(); }, [loadRelationships, loadActivity]);

  const addRelationship = async () => {
    if (!targetId || !predicate.trim()) return;
    setError(null);
    try {
      await apiFetch(`/api/people/${encodeURIComponent(view.entity_uuid)}/relationships`, {
        method: 'POST', body: JSON.stringify({ target_entity_uuid: targetId, predicate: predicate.trim() }),
      });
      setAddingRelationship(false); setTargetId(''); await loadRelationships();
    } catch (e) { setError(`Couldn't add relationship: ${(e as Error).message}`); }
  };

  const removeRelationship = async (edge: PersonRelationship) => {
    setError(null);
    try {
      await apiFetch(`/api/people/${encodeURIComponent(edge.from_entity_uuid)}/relationships/${encodeURIComponent(edge.to_entity_uuid)}/${encodeURIComponent(edge.predicate)}`, { method: 'DELETE' });
      await loadRelationships();
    } catch (e) { setError(`Couldn't remove relationship: ${(e as Error).message}`); }
  };

  // The Enricher (#495 slice 4), prepared-prompt pattern: copy the enrichment
  // request to the clipboard and take the user to chat — the agent runs
  // enrich_person → researches with its web tools → propose_enrichment, and
  // findings wait in the Decision Inbox for approval. Nothing here writes.
  const requestEnrichment = () => {
    const prompt =
      `Refresh enrichment for ${view.display_name}: call enrich_person with ` +
      `person "${view.display_name}", research the enrichable fields with your ` +
      `web tools, then call propose_enrichment so I can review the findings in ` +
      `the Decision Inbox.`;
    navigator.clipboard?.writeText(prompt).catch(() => {});
    setPromptCopied(true);
    navigateToTool('chat');
  };

  const startEdit = () => {
    setError(null);
    setDraft(draftFrom(view));
    setEditing(true);
  };

  const cancelEdit = () => {
    setEditing(false);
    setError(null);
  };

  const saveEdit = async () => {
    // Only send fields that actually changed; a null value and an empty draft
    // are equal (no-op), so clearing a blank field never writes an empty string.
    const changed: Partial<Record<EditableKey, string>> = {};
    for (const { key } of EDITABLE_FIELDS) {
      const current = (view[key] ?? '') as string;
      const next = draft[key];
      if (next !== current) changed[key] = next;
    }
    if (Object.keys(changed).length === 0) {
      setEditing(false);
      return;
    }

    const prior = view;
    // Optimistic: reflect the edit immediately; roll back on failure.
    const optimistic: Person = { ...view };
    for (const [k, v] of Object.entries(changed)) {
      (optimistic as unknown as Record<string, string | null>)[k] = v === '' ? null : v;
    }
    setView(optimistic);
    setSaving(true);
    setError(null);

    try {
      const updated = await apiFetch<Person>(
        `/api/people/${encodeURIComponent(view.entity_uuid)}/fields`,
        { method: 'PATCH', body: JSON.stringify({ fields: changed }) },
      );
      // Reconcile with the graph's authoritative truth (the response is the
      // re-overlaid person), keeping this project's join columns.
      setView(prev => ({
        ...prev,
        role: updated.role,
        company: updated.company,
        email: updated.email,
        phone: updated.phone,
        notes: updated.notes,
        last_contact_at: updated.last_contact_at,
        birthday: updated.birthday,
        relationship_strength: updated.relationship_strength,
        how_met: updated.how_met,
      }));
      setEditing(false);
      // Decoupled panel has no people event stream yet — nudge it to refetch.
      bumpPeople();
    } catch (e) {
      // Roll the optimistic view back and keep edit mode open with the reason.
      setView(prior);
      const err = e as Error & { status?: number };
      const code = err.status ? `${err.status} ` : '';
      setError(`Couldn't save changes: ${code}${err.message || 'request failed'}`);
    } finally {
      setSaving(false);
    }
  };

  const doDisassociate = async () => {
    // Unreachable from the directory — the control that sets `confirming` is
    // only rendered when a project scope exists. Belt-and-braces so a future
    // caller cannot turn a null scope into a request to `/api/projects/null/...`.
    if (!projectId) return;
    setRemoving(true);
    setError(null);
    try {
      await apiFetch(
        `/api/projects/${encodeURIComponent(projectId)}/people/${encodeURIComponent(view.entity_uuid)}`,
        { method: 'DELETE' },
      );
      bumpPeople();
      onClose();
    } catch {
      setError("Couldn't remove this person from the project. Please try again.");
      setRemoving(false);
    }
  };

  const badge = association?.project_role
    ? { label: association.project_role, color: colors.cyan, bg: colors.cyanSoft }
    : null;

  const footer = editing ? (
    <>
      <span style={{ flex: 1 }} />
      <button onClick={cancelEdit} disabled={saving} style={ghostBtn(colors)}>
        Cancel
      </button>
      <button onClick={saveEdit} disabled={saving} style={primaryBtn(colors)}>
        {saving ? 'Saving…' : 'Save'}
      </button>
    </>
  ) : confirming ? (
    <>
      <span style={{ flex: 1, fontSize: 12, color: colors.textMuted }}>
        Remove {view.display_name} from this project?
      </span>
      <button onClick={() => setConfirming(false)} disabled={removing} style={ghostBtn(colors)}>
        Keep
      </button>
      <button onClick={doDisassociate} disabled={removing} style={dangerBtn(colors)}>
        {removing ? 'Removing…' : 'Confirm remove'}
      </button>
    </>
  ) : (
    <>
      <button onClick={startEdit} style={ghostBtn(colors)}>
        Edit fields
      </button>
      <button onClick={requestEnrichment} style={ghostBtn(colors)}>
        {promptCopied ? 'Prompt copied — paste it in chat' : 'Refresh enrichment'}
      </button>
      <span style={{ flex: 1 }} />
      {projectId && (
        <button onClick={() => setConfirming(true)} style={dangerBtn(colors)}>
          Remove from project
        </button>
      )}
    </>
  );

  return (
    <DetailModal title={view.display_name} badge={badge} onClose={onClose} footer={footer}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        {editing ? (
          <EditForm colors={colors} draft={draft} onChange={(k, v) => setDraft(d => ({ ...d, [k]: v }))} />
        ) : (
          <>
            <ReadView colors={colors} person={view} association={association} />
            <RelatedPeople colors={colors} rows={relationships} people={allPeople} status={relatedStatus}
              adding={addingRelationship} targetId={targetId} predicate={predicate}
              onStart={() => setAddingRelationship(true)} onCancel={() => setAddingRelationship(false)}
              onTarget={setTargetId} onPredicate={setPredicate} onAdd={addRelationship} onRemove={removeRelationship} />
            <PersonActivityTimeline colors={colors} rows={activity} status={activityStatus} onRetry={loadActivity} />
          </>
        )}

        {error && (
          <div style={{
            fontSize: 12, color: colors.danger,
            borderRadius: radius.md, border: `1px solid ${colors.danger}`,
            background: colors.danger + '14', padding: '8px 12px',
          }}>
            {error}
          </div>
        )}
      </div>
    </DetailModal>
  );
}

function RelatedPeople({ colors, rows, people, status, adding, targetId, predicate, onStart, onCancel, onTarget, onPredicate, onAdd, onRemove }: {
  colors: ReturnType<typeof useTheme>['colors']; rows: PersonRelationship[]; people: Person[];
  status: 'loading' | 'ready' | 'error'; adding: boolean; targetId: string; predicate: string;
  onStart: () => void; onCancel: () => void; onTarget: (v: string) => void; onPredicate: (v: string) => void;
  onAdd: () => void; onRemove: (r: PersonRelationship) => void;
}) {
  return <section>
    <div style={{ display: 'flex', alignItems: 'center', marginBottom: 7 }}>
      <SectionLabel colors={colors}>Related people</SectionLabel><span style={{ flex: 1 }} />
      {!adding && <button aria-label="Add related person" onClick={onStart} style={miniBtn(colors)}><FiPlus size={12} /> Add</button>}
    </div>
    {status === 'loading' && <Small colors={colors}>Loading relationships…</Small>}
    {status === 'error' && <Small colors={colors}>Couldn't load relationships.</Small>}
    {status === 'ready' && rows.length === 0 && !adding && <Small colors={colors}>No related people yet.</Small>}
    {rows.map(row => <div key={`${row.from_entity_uuid}-${row.to_entity_uuid}-${row.predicate}`} style={{ display: 'flex', gap: 8, alignItems: 'center', padding: '6px 0', borderBottom: `1px solid ${colors.border}` }}>
      <span style={{ color: colors.text, fontSize: 12, fontWeight: 600 }}>{row.other_person.display_name}</span>
      <span style={{ color: colors.textDim, fontSize: 11 }}>{row.predicate.replace(/_/g, ' ')}</span><span style={{ flex: 1 }} />
      <button aria-label={`Remove ${row.other_person.display_name} relationship`} onClick={() => onRemove(row)} style={iconBtn(colors)}><FiTrash2 size={12} /></button>
    </div>)}
    {adding && <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 6, marginTop: 8 }}>
      <select aria-label="Related person" value={targetId} onChange={e => onTarget(e.target.value)} style={control(colors)}><option value="">Choose person…</option>{people.map(p => <option key={p.entity_uuid} value={p.entity_uuid}>{p.display_name}</option>)}</select>
      <input aria-label="Relationship type" value={predicate} onChange={e => onPredicate(e.target.value)} placeholder="related_to" style={control(colors)} />
      <div style={{ display: 'flex', gap: 4 }}><button onClick={onCancel} style={miniBtn(colors)}>Cancel</button><button onClick={onAdd} disabled={!targetId || !predicate.trim()} style={miniBtn(colors)}>Add</button></div>
    </div>}
  </section>;
}

function PersonActivityTimeline({ colors, rows, status, onRetry }: { colors: ReturnType<typeof useTheme>['colors']; rows: PersonActivity[]; status: 'loading'|'ready'|'error'; onRetry: () => void }) {
  const icon = (kind: PersonActivity['kind']) => kind === 'memory' ? <FiBookOpen size={12} /> : kind === 'note' ? <FiFileText size={12} /> : <FiCheckSquare size={12} />;
  return <section><SectionLabel colors={colors}>Recent activity</SectionLabel>
    {status === 'loading' && <Small colors={colors}>Loading activity…</Small>}
    {status === 'error' && <Small colors={colors}>Couldn't load activity. <button onClick={onRetry} style={linkBtn(colors)}>Retry</button></Small>}
    {status === 'ready' && rows.length === 0 && <Small colors={colors}>No activity referencing this person yet.</Small>}
    {rows.map(row => <div key={row.id} style={{ display: 'flex', gap: 8, padding: '7px 0', borderBottom: `1px solid ${colors.border}` }}><span style={{ color: colors.cyan, marginTop: 2 }}>{icon(row.kind)}</span><div style={{ minWidth: 0 }}><div style={{ fontSize: 12, color: colors.text, fontWeight: 600 }}>{row.title}</div><div style={{ fontSize: 11, color: colors.textMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{row.detail}</div><div style={{ fontSize: 10, color: colors.textDim }}>{fmtTime(row.timestamp)}</div></div></div>)}
  </section>;
}

function SectionLabel({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: ReactNode }) { return <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, textTransform: 'uppercase', letterSpacing: '.04em' }}>{children}</div>; }
function Small({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: ReactNode }) { return <div style={{ fontSize: 11, color: colors.textDim, marginTop: 6 }}>{children}</div>; }

function ReadView({ colors, person, association }: {
  colors: ReturnType<typeof useTheme>['colors'];
  person: Person;
  association?: PersonAssociation | null;
}) {
  const rows = useMemo<[string, ReactNode][]>(() => [
    ['Role', person.role || '—'],
    ['Company', person.company || '—'],
    ['Email', person.email
      ? <a href={`mailto:${person.email}`} style={{ color: colors.cyan, textDecoration: 'none' }}>{person.email}</a>
      : '—'],
    ['Phone', person.phone
      ? <a href={`tel:${person.phone}`} style={{ color: colors.cyan, textDecoration: 'none' }}>{person.phone}</a>
      : '—'],
    ['Birthday', person.birthday || '—'],
    ['Relationship', person.relationship_strength || '—'],
    ['How met', person.how_met || '—'],
    // Project-scoped rows only exist when opened from a project's People panel.
    ...(association
      ? ([
          ['Project role', association.project_role || '—'],
          ['Associated', fmtTime(association.associated_at)],
        ] as [string, ReactNode][])
      : []),
    ['Last contact', fmtTime(person.last_contact_at)],
  ], [colors, person, association]);

  return (
    <>
      <MetaGrid colors={colors} rows={rows} />
      {person.notes && (
        <div>
          <div style={{
            fontSize: 11, color: colors.textDim, fontFamily: font.mono,
            textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 4,
          }}>
            Notes
          </div>
          <div style={{
            fontSize: 13, color: colors.textMuted, lineHeight: 1.5,
            whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
          }}>
            {person.notes}
          </div>
        </div>
      )}
    </>
  );
}

function EditForm({ colors, draft, onChange }: {
  colors: ReturnType<typeof useTheme>['colors'];
  draft: Draft;
  onChange: (key: EditableKey, value: string) => void;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {EDITABLE_FIELDS.map(({ key, label, multiline, placeholder }) => (
        <label key={key} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{
            fontSize: 11, color: colors.textDim, fontFamily: font.mono,
            textTransform: 'uppercase', letterSpacing: '0.04em',
          }}>
            {label}
          </span>
          {multiline ? (
            <textarea
              value={draft[key]}
              onChange={e => onChange(key, e.target.value)}
              rows={3}
              placeholder={placeholder}
              style={{ ...inputStyle(colors), resize: 'vertical', lineHeight: 1.5 }}
            />
          ) : (
            <input
              value={draft[key]}
              onChange={e => onChange(key, e.target.value)}
              placeholder={placeholder}
              style={inputStyle(colors)}
            />
          )}
        </label>
      ))}
    </div>
  );
}

function MetaGrid({ colors, rows }: {
  colors: ReturnType<typeof useTheme>['colors'];
  rows: [string, ReactNode][];
}) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 14px' }}>
      {rows.map(([k, v]) => (
        <div key={k} style={{ display: 'contents' }}>
          <span style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'nowrap' }}>
            {k}
          </span>
          <span style={{ fontSize: 12, color: colors.text, wordBreak: 'break-word', userSelect: 'text' }}>
            {v}
          </span>
        </div>
      ))}
    </div>
  );
}

function inputStyle(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    fontSize: 12, padding: '6px 9px', borderRadius: radius.md,
    background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
    color: colors.text, fontFamily: font.body, outline: 'none', width: '100%',
    boxSizing: 'border-box',
  };
}

function control(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return { ...inputStyle(colors), padding: '5px 7px' };
}

function miniBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return { display: 'inline-flex', alignItems: 'center', gap: 3, padding: '4px 7px', borderRadius: radius.md,
    border: `1px solid ${colors.border}`, background: 'none', color: colors.textMuted,
    fontFamily: font.body, fontSize: 11, cursor: 'pointer' };
}

function iconBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return { border: 'none', background: 'none', color: colors.textDim, cursor: 'pointer', padding: 2, display: 'flex' };
}

function linkBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return { border: 'none', background: 'none', color: colors.cyan, cursor: 'pointer', padding: 0, fontFamily: font.body, fontSize: 11 };
}

function ghostBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '6px 14px', borderRadius: radius.md,
    border: `1px solid ${colors.border}`, background: 'none',
    fontFamily: font.body, fontSize: 12, color: colors.textMuted, cursor: 'pointer',
  };
}

function primaryBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '6px 14px', borderRadius: radius.md,
    border: `1px solid ${colors.cyan}`, background: colors.cyanSoft,
    fontFamily: font.body, fontSize: 12, fontWeight: 500, color: colors.cyan, cursor: 'pointer',
  };
}

function dangerBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '6px 14px', borderRadius: radius.md,
    border: `1px solid ${colors.danger}`, background: colors.danger + '14',
    fontFamily: font.body, fontSize: 12, fontWeight: 500, color: colors.danger, cursor: 'pointer',
  };
}

/** Mounted once at the app root — renders the modal for the active target. */
export function PersonDetailModalHost() {
  const personDetail = useCommandCenter(s => s.personDetail);
  const closePersonDetail = useCommandCenter(s => s.closePersonDetail);
  if (!personDetail) return null;
  return (
    <PersonDetailModal
      // Remount on target change so local edit/view state resets cleanly.
      key={personDetail.person.entity_uuid}
      projectId={personDetail.projectId}
      person={personDetail.person}
      association={personDetail.association}
      onClose={closePersonDetail}
    />
  );
}
