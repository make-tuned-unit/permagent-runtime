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
 * Every typed field is an input, always — fill in what enrichment missed
 * without flipping into a separate edit mode. Save (or closing the panel)
 * writes via `PATCH /api/people/{entity_uuid}/fields`. Those land in the
 * authoritative graph with **manual provenance** (`FieldSource::Manual`), so a
 * later Enricher pass can never clobber them. The edit is optimistic — inputs
 * apply to the view immediately and roll back on a non-2xx — and the response is
 * the re-overlaid person, i.e. the graph's own truth, which we merge back in.
 * On success we bump the store's people revision so the decoupled panel refetches.
 *
 * "Run enrichment" sends a message to the live chat session — the agent
 * calls enrich_person with this person's entity_uuid, researches, then files
 * propose_enrichment. Findings wait in the Decision Inbox. Nothing here writes
 * the profile, and nothing is copied to the clipboard. Disassociate (DELETE
 * /api/projects/{id}/people/{entity_uuid}, #530) is the other mutation.
 *
 * On the People tab the panel docks inline (graph/list shrinks). From a
 * project's People list it docks on the right of the window. Same body.
 *
 * PersonDetailModalHost is mounted once at the app root and renders the overlay
 * dock when the target was opened from a project (PeopleView owns the inline
 * case so the graph stays visible beside it).
 *
 * The panel itself is a `common/DetailModal` (R12). It used to be
 * `PersonDetailShell`, 108 lines of second modal: its own header/badge/close
 * row, its own scrollable body, its own footer, its own Escape handler and its
 * own hand-rolled glass, each of them a re-implementation of the shared shell
 * that then drifted from it. All that is left here is WHERE the panel sits —
 * `PersonDetailDock`, below — because that is the only thing about this
 * surface that was ever actually different.
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { FiBookOpen, FiCalendar, FiCheck, FiCheckSquare, FiExternalLink, FiFileText, FiPlus, FiTrash2 } from 'react-icons/fi';
import { apiFetch } from '../../lib/api';
import { hapticSuccess } from '../../lib/haptic';
import { navigateToTool, useCommandCenter } from '../../lib/store';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { TITLEBAR_HEIGHT } from '../../lib/windowChrome';
import { Button } from '../common/Button';
import { Chip } from '../common/Chip';
import { DetailModal } from '../common/DetailModal';
import type { DeleteReport, MergeReport, Person, PersonActivity, PersonAssociation, PersonMeeting, PersonProject, PersonRelationship, UndoReport } from './types';
import { PersonFace } from '../people/PersonFace';
import { MergePersonPanel } from '../people/MergePersonPanel';

import { Tooltip } from '../common/Tooltip';
/**
 * The relationships people actually record, offered by name. The graph stores
 * a free-form predicate and will take anything, so this is a set of
 * suggestions rather than a closed list — a `<select>` here would quietly stop
 * being true the first time someone needed a word that is not on it.
 */
const RELATIONSHIP_PREDICATES: Array<{ value: string; label: string }> = [
  { value: 'works_with', label: 'Works with' },
  { value: 'reports_to', label: 'Reports to' },
  { value: 'manages', label: 'Manages' },
  { value: 'introduced_by', label: 'Introduced by' },
  { value: 'friend_of', label: 'Friend of' },
  { value: 'family_of', label: 'Family of' },
  { value: 'invested_in', label: 'Invested in' },
  { value: 'related_to', label: 'Related to (unspecified)' },
];

function pad2(n: number): string { return String(n).padStart(2, '0'); }

function localDateTimeValue(d = new Date()): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}T${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function plusDaysLocal(local: string, days: number): string {
  const d = new Date(local);
  if (!Number.isFinite(d.getTime())) return local;
  d.setDate(d.getDate() + days);
  return localDateTimeValue(d);
}

function localToRfc3339(local: string): string {
  const d = new Date(local);
  if (!Number.isFinite(d.getTime())) throw new Error('Invalid time');
  return d.toISOString();
}

function fmtTime(iso: string | null): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  return Number.isFinite(t) ? new Date(t).toLocaleString() : iso;
}

/** The person fields this modal lets the user edit, in display order. `notes`
 *  renders as a textarea; the rest as single-line inputs. Field names match the
 *  backend `PERSON_FIELD_NAMES` vocabulary exactly (the wire keys). */
const EDITABLE_FIELDS: { key: EditableKey; label: string; multiline?: boolean; placeholder?: string; link?: boolean }[] = [
  { key: 'photo_url', label: 'Photo', placeholder: 'https://…/photo.jpg', link: true },
  { key: 'role', label: 'Role' },
  { key: 'company', label: 'Company' },
  { key: 'email', label: 'Email', placeholder: 'name@example.com' },
  { key: 'phone', label: 'Phone' },
  { key: 'linkedin', label: 'LinkedIn', placeholder: 'https://www.linkedin.com/in/…', link: true },
  { key: 'x_handle', label: 'X', placeholder: '@handle' },
  { key: 'facebook', label: 'Facebook', placeholder: 'https://www.facebook.com/…', link: true },
  { key: 'instagram', label: 'Instagram', placeholder: 'https://www.instagram.com/…', link: true },
  { key: 'personal_site', label: 'Site', placeholder: 'https://…', link: true },
  { key: 'birthday', label: 'Birthday', placeholder: 'YYYY-MM-DD' },
  { key: 'last_contact_at', label: 'Last contact', placeholder: 'YYYY-MM-DD or a date you remember' },
  { key: 'relationship_strength', label: 'Relationship' },
  { key: 'how_met', label: 'How met' },
  { key: 'find_online_hints', label: 'Find online', multiline: true, placeholder: 'Company, LinkedIn, city — anything that helps the agent find them' },
  { key: 'notes', label: 'Notes', multiline: true },
];

/**
 * Only http(s) links become a click target. The value can arrive from the
 * Enricher, which never passes through the PATCH validator, so the scheme is
 * re-checked here before it is handed to the in-app browser — a `javascript:`
 * or `file:` value renders as plain text instead.
 */
function safeLink(url: string | null): string | null {
  if (!url) return null;
  const trimmed = url.trim();
  return /^https?:\/\//i.test(trimmed) ? trimmed : null;
}

type EditableKey =
  | 'role'
  | 'company'
  | 'email'
  | 'phone'
  | 'birthday'
  | 'relationship_strength'
  | 'how_met'
  | 'linkedin'
  | 'x_handle'
  | 'facebook'
  | 'instagram'
  | 'personal_site'
  | 'photo_url'
  | 'last_contact_at'
  | 'find_online_hints'
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
    linkedin: p.linkedin ?? '',
    x_handle: p.x_handle ?? '',
    facebook: p.facebook ?? '',
    instagram: p.instagram ?? '',
    personal_site: p.personal_site ?? '',
    photo_url: p.photo_url ?? '',
    last_contact_at: p.last_contact_at ?? '',
    find_online_hints: p.find_online_hints ?? '',
    notes: p.notes ?? '',
  };
}

/** Message sent to the agent when the user clicks Run enrichment.
 *  Uses entity_uuid (strict resolve) and any find-online hints already on file.
 *  Exported so tests can pin that this is a run instruction, not a copy-paste prompt. */
export function buildEnrichmentMessage(person: Person): string {
  const bits = [
    `Run enrichment for ${person.display_name}.`,
    `Call enrich_person with person "${person.entity_uuid}" (the directory entity_uuid — do not search by name).`,
  ];
  if (person.company) bits.push(`Company on file: ${person.company}.`);
  if (person.role) bits.push(`Role on file: ${person.role}.`);
  const hints = person.find_online_hints?.trim();
  if (hints) bits.push(`How to find them online: ${hints}.`);
  bits.push(
    'Research the enrichable fields with your web tools, then call propose_enrichment so I can review the findings here in chat. Do not wait for a prompt from me — run it now.',
  );
  return bits.join(' ');
}

export function PersonDetailModal({
  projectId,
  person,
  association,
  onClose,
  variant = 'overlay',
}: {
  /** Null when opened from the global directory — there is no project context. */
  projectId: string | null;
  person: Person;
  /** Null outside a project; gates the project-role badge and Disassociate. */
  association?: PersonAssociation | null;
  onClose: () => void;
  /** `inline` docks inside PeopleView; `overlay` is a right-side drawer. */
  variant?: 'inline' | 'overlay';
}) {
  const { colors } = useTheme();
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const patchPersonDetail = useCommandCenter(s => s.patchPersonDetail);
  const sendMessage = useCommandCenter(s => s.sendMessage);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  const setPendingProjectNavigation = useCommandCenter(s => s.setPendingProjectNavigation);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [enriching, setEnriching] = useState(false);

  // Merge (duplicate-cleanup epic): `merging` swaps the modal body for
  // MergePersonPanel; on success the panel hands back a MergeReport, which
  // renders as a persistent summary card (with Undo) ABOVE the normal body —
  // not a replacement — so the user can keep working the profile after seeing
  // it. Undo is only ever a component-state affordance: it does not need to
  // survive a close, per the spec.
  const [merging, setMerging] = useState(false);
  const [mergeReport, setMergeReport] = useState<MergeReport | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [undoReport, setUndoReport] = useState<UndoReport | null>(null);
  const [undoError, setUndoError] = useState<string | null>(null);

  // Delete: a real two-step confirm (`deleteStep`) naming the person and
  // listing counts already loaded in this modal (meetings.length,
  // personProjects.length) — never invented numbers. On success the body
  // swaps to a terminal "deleted" card showing `retained` verbatim; the
  // profile fields are moot once the person is gone, so nothing else renders.
  const [deleteStep, setDeleteStep] = useState<0 | 1>(0);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deletedReport, setDeletedReport] = useState<DeleteReport | null>(null);

  // Local view of the person: seeded from the prop, updated optimistically on a
  // field edit and reconciled from the PATCH response (the graph's own truth).
  const [view, setView] = useState<Person>(person);
  const [draft, setDraft] = useState<Draft>(() => draftFrom(person));
  const [saving, setSaving] = useState(false);
  const [relationships, setRelationships] = useState<PersonRelationship[]>([]);
  const [activity, setActivity] = useState<PersonActivity[]>([]);
  const [meetings, setMeetings] = useState<PersonMeeting[]>([]);
  const [allPeople, setAllPeople] = useState<Person[]>([]);
  const [relatedStatus, setRelatedStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [activityStatus, setActivityStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [meetingsStatus, setMeetingsStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [projectsStatus, setProjectsStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [addingRelationship, setAddingRelationship] = useState(false);
  const [addingMeeting, setAddingMeeting] = useState(false);
  const [meetingTitle, setMeetingTitle] = useState('');
  const [meetingStarts, setMeetingStarts] = useState(localDateTimeValue);
  const [meetingNotes, setMeetingNotes] = useState('');
  const [meetingProjectId, setMeetingProjectId] = useState(projectId ?? '');
  const [followUp, setFollowUp] = useState(true);
  const [followUpAt, setFollowUpAt] = useState(() => plusDaysLocal(localDateTimeValue(), 7));
  const [followUpNote, setFollowUpNote] = useState('');
  const [personProjects, setPersonProjects] = useState<PersonProject[]>([]);
  const [savingMeeting, setSavingMeeting] = useState(false);
  const [targetId, setTargetId] = useState('');
  const [predicate, setPredicate] = useState('related_to');
  const relationshipsGeneration = useRef(0);
  const activityGeneration = useRef(0);
  const meetingsGeneration = useRef(0);
  const projectsGeneration = useRef(0);

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

  const loadMeetings = useCallback(async () => {
    const generation = ++meetingsGeneration.current;
    setMeetingsStatus('loading');
    try {
      const next = await apiFetch<PersonMeeting[]>(`/api/people/${encodeURIComponent(view.entity_uuid)}/meetings`);
      if (generation !== meetingsGeneration.current) return;
      if (!Array.isArray(next)) throw new Error('Invalid meetings response');
      setMeetings(next);
      setMeetingsStatus('ready');
    } catch {
      if (generation === meetingsGeneration.current) setMeetingsStatus('error');
    }
  }, [view.entity_uuid]);

  /**
   * The fourth loader in this modal, and until now the only untracked one: a
   * failure emptied the list and said nothing, which mattered more here than
   * it looks. `personProjects.length` is one of the two counts the delete
   * confirmation quotes back — "this deletes N project links" — and that
   * sentence exists specifically so the numbers in it are real. A silently
   * emptied list turned it into "0 project links", which is a claim, not a
   * blank. Same shape as its three siblings, for the same reason.
   */
  const loadProjects = useCallback(async () => {
    const generation = ++projectsGeneration.current;
    setProjectsStatus('loading');
    try {
      const rows = await apiFetch<PersonProject[]>(`/api/people/${encodeURIComponent(view.entity_uuid)}/projects`);
      if (generation !== projectsGeneration.current) return;
      if (!Array.isArray(rows)) throw new Error('Invalid projects response');
      setPersonProjects(rows);
      setProjectsStatus('ready');
    } catch {
      // The last good list stays on screen; what changes is that the surface
      // stops claiming it is current.
      if (generation === projectsGeneration.current) setProjectsStatus('error');
    }
  }, [view.entity_uuid]);

  useEffect(() => { loadRelationships(); loadActivity(); loadMeetings(); loadProjects(); }, [loadRelationships, loadActivity, loadMeetings, loadProjects]);

  /**
   * Hop to a project from the person. Reuses `pendingProjectNavigation`, the
   * seam ProjectsView already self-heals when the target is missing from its
   * snapshot — the same path the agent's own deep-links take. The modal closes
   * first, because leaving it open over the destination would be a drawer
   * covering the thing it just navigated to.
   */
  const openProject = useCallback((id: string) => {
    setPendingProjectNavigation(id);
    navigateToTool('projects');
    onClose();
  }, [setPendingProjectNavigation, onClose]);

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

  const addMeeting = async () => {
    setError(null);
    setSavingMeeting(true);
    try {
      await apiFetch(`/api/people/${encodeURIComponent(view.entity_uuid)}/meetings`, {
        method: 'POST',
        body: JSON.stringify({
          title: meetingTitle.trim() || undefined,
          starts_at: localToRfc3339(meetingStarts),
          notes: meetingNotes.trim() || undefined,
          project_id: meetingProjectId || undefined,
          follow_up_at: followUp ? localToRfc3339(followUpAt) : undefined,
          follow_up_note: followUp && followUpNote.trim() ? followUpNote.trim() : undefined,
        }),
      });
      setAddingMeeting(false);
      setMeetingTitle('');
      setMeetingNotes('');
      setMeetingProjectId('');
      setFollowUp(true);
      setFollowUpNote('');
      setMeetingStarts(localDateTimeValue());
      setFollowUpAt(plusDaysLocal(localDateTimeValue(), 7));
      bumpPeople();
      await Promise.all([loadMeetings(), loadActivity()]);
    } catch (e) {
      setError(`Couldn't log meeting: ${(e as Error).message}`);
    } finally {
      setSavingMeeting(false);
    }
  };

  const markFollowUpDone = async (meeting: PersonMeeting) => {
    setError(null);
    try {
      await apiFetch(`/api/people/${encodeURIComponent(view.entity_uuid)}/meetings/${encodeURIComponent(meeting.id)}`, {
        method: 'PATCH',
        body: JSON.stringify({ follow_up_done: true }),
      });
      bumpPeople();
      await loadMeetings();
    } catch (e) {
      setError(`Couldn't close follow-up: ${(e as Error).message}`);
    }
  };

  // The Enricher: send the request to the live session. The agent runs
  // enrich_person → researches with its web tools → propose_enrichment, and
  // findings wait in the Decision Inbox for approval. Nothing here writes,
  // and nothing is copied to the clipboard.
  const requestEnrichment = async () => {
    if (enriching) return;
    setEnriching(true);
    setError(null);
    try {
      openChatDock();
      await sendMessage(buildEnrichmentMessage(view));
    } catch (e) {
      setError(`Couldn't start enrichment: ${(e as Error).message}`);
    } finally {
      setEnriching(false);
    }
  };

  const dirty = EDITABLE_FIELDS.some(({ key }) => draft[key] !== ((view[key] ?? '') as string));

  // Directory/graph refetch (peopleRev) updates the store person; merge it in
  // unless the user is mid-edit so a live enrichment doesn't clobber a draft.
  useEffect(() => {
    if (person.entity_uuid !== view.entity_uuid) {
      setView(person);
      setDraft(draftFrom(person));
      return;
    }
    if (dirty) return;
    setView(person);
    setDraft(draftFrom(person));
  }, [person]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveEdit = async (): Promise<boolean> => {
    // Only send fields that actually changed; a null value and an empty draft
    // are equal (no-op), so clearing a blank field never writes an empty string.
    const changed: Partial<Record<EditableKey, string>> = {};
    for (const { key } of EDITABLE_FIELDS) {
      const current = (view[key] ?? '') as string;
      const next = draft[key];
      if (next !== current) changed[key] = next;
    }
    if (Object.keys(changed).length === 0) return true;

    const prior = view;
    const priorDraft = draft;
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
      const next: Person = {
        ...optimistic,
        role: updated.role,
        company: updated.company,
        email: updated.email,
        phone: updated.phone,
        notes: updated.notes,
        last_contact_at: updated.last_contact_at,
        birthday: updated.birthday,
        relationship_strength: updated.relationship_strength,
        how_met: updated.how_met,
        linkedin: updated.linkedin,
        x_handle: updated.x_handle,
        facebook: updated.facebook,
        instagram: updated.instagram,
        personal_site: updated.personal_site,
        photo_url: updated.photo_url,
        find_online_hints: updated.find_online_hints,
      };
      setView(next);
      setDraft(draftFrom(next));
      patchPersonDetail(next);
      // Decoupled panel has no people event stream yet — nudge it to refetch.
      bumpPeople();
      hapticSuccess();
      return true;
    } catch (e) {
      // Roll the optimistic view back and keep the draft so nothing is lost.
      setView(prior);
      setDraft(priorDraft);
      const err = e as Error & { status?: number };
      const code = err.status ? `${err.status} ` : '';
      setError(`Couldn't save changes: ${code}${err.message || 'request failed'}`);
      return false;
    } finally {
      setSaving(false);
    }
  };

  const handleClose = async () => {
    // The person no longer exists once deleted — a field PATCH here would
    // just 404. Close directly (the DeletedCard's own Close button is the
    // normal path; this covers Escape / the shell's X button too).
    if (!deletedReport && dirty) {
      const ok = await saveEdit();
      if (!ok) return;
    }
    onClose();
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

  const handleMergeDone = (report: MergeReport) => {
    setMerging(false);
    setMergeReport(report);
    setUndoReport(null);
    setUndoError(null);
    bumpPeople();
  };

  const undoMerge = async () => {
    if (!mergeReport) return;
    setUndoing(true);
    setUndoError(null);
    try {
      const report = await apiFetch<UndoReport>(
        `/api/people/merges/${encodeURIComponent(mergeReport.merge_id)}/undo`,
        { method: 'POST' },
      );
      setUndoReport(report);
      bumpPeople();
    } catch (e) {
      const err = e as Error & { status?: number };
      setUndoError(`Couldn't undo: ${err.status ? `${err.status} ` : ''}${err.message || 'request failed'}`);
    } finally {
      setUndoing(false);
    }
  };

  const doDelete = async () => {
    setDeleting(true);
    setDeleteError(null);
    try {
      const report = await apiFetch<DeleteReport>(
        `/api/people/${encodeURIComponent(view.entity_uuid)}`,
        { method: 'DELETE', body: JSON.stringify({ confirm: true }) },
      );
      bumpPeople();
      setDeletedReport(report);
    } catch (e) {
      const err = e as Error & { status?: number };
      setDeleteError(`Couldn't delete ${view.display_name}: ${err.status ? `${err.status} ` : ''}${err.message || 'request failed'}`);
      setDeleting(false);
    }
  };

  const badge = association?.project_role
    ? { label: association.project_role, color: colors.cyan, bg: colors.cyanSoft }
    : null;

  // Terminal states (merging the panel, or the person just got deleted) take
  // over the whole modal — a Save/Remove footer over a gone-or-mid-merge
  // record would be dead UI, so the footer is entirely suppressed for them;
  // both MergePersonPanel and the deleted card carry their own actions.
  const footer = deletedReport ? undefined : merging ? undefined : confirming ? (
    <>
      <span style={{ flex: 1, fontSize: textSize.caption, color: colors.textMuted }}>
        Remove {view.display_name} from this project?
      </span>
      <Button colors={colors} onClick={() => setConfirming(false)} disabled={removing} style={ghostVars(colors)}>
        Keep
      </Button>
      <Button colors={colors} onClick={doDisassociate} pending={removing} disabled={removing} style={dangerVars(colors)}>
        {removing ? 'Removing…' : 'Confirm remove'}
      </Button>
    </>
  ) : deleteStep === 1 ? (
    <span style={{ flex: 1, fontSize: textSize.caption, color: colors.textMuted }}>
      Confirm below to delete {view.display_name}.
    </span>
  ) : (
    <>
      <Button colors={colors} onClick={requestEnrichment} pending={enriching} disabled={enriching} style={ghostVars(colors)}>
        {enriching ? 'Running enrichment…' : 'Run enrichment'}
      </Button>
      <Button colors={colors} onClick={() => void saveEdit()} pending={saving} disabled={saving || !dirty} style={primaryVars(colors)}>
        {saving ? 'Saving…' : 'Save'}
      </Button>
      <span style={{ flex: 1 }} />
      {view.entity_uuid && (
        <Button colors={colors} onClick={() => setMerging(true)} style={ghostVars(colors)}>
          Merge into…
        </Button>
      )}
      {projectId && (
        <Button colors={colors} onClick={() => setConfirming(true)} style={dangerVars(colors)}>
          Remove from project
        </Button>
      )}
      {view.entity_uuid && (
        <Button colors={colors} onClick={() => { setDeleteStep(1); setDeleteError(null); }} style={dangerVars(colors)}>
          Delete person
        </Button>
      )}
    </>
  );

  return (
    <PersonDetailDock variant={variant}>
      <DetailModal
        placement="contained"
        title={view.display_name}
        badge={badge}
        onClose={handleClose}
        footer={footer}
        bodyStyle={{ padding: space.xxl }}
        // Six actions in a 400px dock: they wrap, and they read left-to-right
        // from the primary pair rather than being right-aligned against the
        // destructive ones.
        footerStyle={{ flexWrap: 'wrap', justifyContent: 'flex-start', gap: space.md }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxl }}>
          {deletedReport ? (
            <DeletedCard colors={colors} report={deletedReport} onClose={onClose} />
          ) : merging ? (
            <MergePersonPanel person={view} onDone={handleMergeDone} onCancel={() => setMerging(false)} />
          ) : (
            <>
              {mergeReport && (
                <MergeResultCard
                  colors={colors}
                  report={mergeReport}
                  undoing={undoing}
                  undoReport={undoReport}
                  undoError={undoError}
                  onUndo={undoMerge}
                />
              )}
              {deleteStep === 1 && (
                <DeleteWarningCard
                  colors={colors}
                  name={view.display_name}
                  meetingsCount={meetings.length}
                  projectsCount={projectsStatus === 'error' ? null : personProjects.length}
                  deleting={deleting}
                  error={deleteError}
                  onCancel={() => { setDeleteStep(0); setDeleteError(null); }}
                  onConfirm={doDelete}
                />
              )}
              <EditForm
                colors={colors}
                personName={view.display_name}
                draft={draft}
                onChange={(k, v) => setDraft(d => ({ ...d, [k]: v }))}
              />
              {association && (
                <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono }}>
                  {association.project_role ? `${association.project_role} · ` : ''}Associated {fmtTime(association.associated_at)}
                </div>
              )}
              <PersonProjects
                colors={colors}
                rows={personProjects}
                status={projectsStatus}
                onRetry={loadProjects}
                onOpen={openProject}
              />
              <RelatedPeople colors={colors} rows={relationships} people={allPeople} status={relatedStatus}
                adding={addingRelationship} targetId={targetId} predicate={predicate}
                onStart={() => setAddingRelationship(true)} onCancel={() => setAddingRelationship(false)}
                onTarget={setTargetId} onPredicate={setPredicate} onAdd={addRelationship} onRemove={removeRelationship} />
              <MeetingsSection
                    colors={colors}
                    personName={view.display_name}
                    rows={meetings}
                    projects={personProjects}
                    status={meetingsStatus}
                    adding={addingMeeting}
                    title={meetingTitle}
                    starts={meetingStarts}
                    notes={meetingNotes}
                    projectId={meetingProjectId}
                    followUp={followUp}
                    followUpAt={followUpAt}
                    followUpNote={followUpNote}
                    saving={savingMeeting}
                    onStart={() => {
                      setAddingMeeting(true);
                      setFollowUpAt(plusDaysLocal(meetingStarts || localDateTimeValue(), 7));
                    }}
                    onCancel={() => setAddingMeeting(false)}
                    onTitle={setMeetingTitle}
                    onStarts={v => {
                      setMeetingStarts(v);
                      if (followUp) setFollowUpAt(plusDaysLocal(v, 7));
                    }}
                    onNotes={setMeetingNotes}
                    onProject={setMeetingProjectId}
                    onFollowUp={setFollowUp}
                    onFollowUpAt={setFollowUpAt}
                    onFollowUpNote={setFollowUpNote}
                    onAdd={addMeeting}
                    onRetry={loadMeetings}
                    onFollowUpDone={markFollowUpDone}
              />
              <PersonActivityTimeline colors={colors} rows={activity} status={activityStatus} onRetry={loadActivity} />

              {error && (
                <div style={{
                  fontSize: textSize.caption, color: colors.danger,
                  borderRadius: radius.md, border: `1px solid ${colors.danger}`,
                  background: colors.danger + '14', padding: '8px 12px',
                }}>
                  {error}
                </div>
              )}
            </>
          )}
        </div>
      </DetailModal>
    </PersonDetailDock>
  );
}

/** Merge success: the summary the daemon returned, plus Undo — live only as
 *  long as this component instance stays mounted (the spec's "undoable for
 *  the session"; it does not need to survive a close). */
function MergeResultCard({ colors, report, undoing, undoReport, undoError, onUndo }: {
  colors: ReturnType<typeof useTheme>['colors'];
  report: MergeReport;
  undoing: boolean;
  undoReport: UndoReport | null;
  undoError: string | null;
  onUndo: () => void;
}) {
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 6,
      borderRadius: radius.md, border: `1px solid ${colors.cyan}`,
      background: colors.cyanSoft, padding: '10px 12px',
    }}>
      <div style={{ fontSize: textSize.caption, color: colors.text }}>{report.summary}</div>
      {undoReport ? (
        <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>
          Undone — restored {undoReport.restored_name} ({undoReport.meetings_restored} meeting{undoReport.meetings_restored === 1 ? '' : 's'},{' '}
          {undoReport.project_links_restored} project link{undoReport.project_links_restored === 1 ? '' : 's'}).
          {undoReport.not_reverted.length > 0 && (
            <div style={{ marginTop: 4 }}>
              Not reverted: {undoReport.not_reverted.join(', ')}
            </div>
          )}
        </div>
      ) : (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <Button colors={colors} onClick={onUndo} pending={undoing} disabled={undoing} style={miniVars(colors)}>
            {undoing ? 'Undoing…' : 'Undo merge'}
          </Button>
          {undoError && <span style={{ fontSize: textSize.micro, color: colors.danger }}>{undoError}</span>}
        </div>
      )}
    </div>
  );
}

/** Two-step delete confirm, in the modal body (not the footer) so the "what
 *  will be deleted" list has room. Counts come from data this modal already
 *  loaded (meetings, personProjects) — never invented. */
function DeleteWarningCard({ colors, name, meetingsCount, projectsCount, deleting, error, onCancel, onConfirm }: {
  colors: ReturnType<typeof useTheme>['colors'];
  name: string;
  meetingsCount: number;
  /** `null` when the project list failed to load — the count is unknown, and
   *  a confirmation that quotes numbers back may not round unknown down to 0. */
  projectsCount: number | null;
  deleting: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 8,
      borderRadius: radius.md, border: `1px solid ${colors.danger}`,
      background: colors.danger + '14', padding: '10px 12px',
    }}>
      <div style={{ fontSize: textSize.caption, color: colors.text, fontWeight: 600 }}>
        Delete {name}? This can't be undone.
      </div>
      <div style={{ fontSize: textSize.micro, color: colors.textMuted }} data-testid="delete-warning-counts">
        This deletes {meetingsCount} logged meeting{meetingsCount === 1 ? '' : 's'} and{' '}
        {projectsCount == null
          ? "an unknown number of project links — that list didn't load"
          : `${projectsCount} project link${projectsCount === 1 ? '' : 's'}`}
        {' '}for {name}.
      </div>
      {error && <div style={{ fontSize: textSize.micro, color: colors.danger }}>{error}</div>}
      <div style={{ display: 'flex', gap: 8 }}>
        <Button colors={colors} onClick={onCancel} disabled={deleting} style={ghostVars(colors)}>Keep</Button>
        <Button colors={colors} onClick={onConfirm} pending={deleting} disabled={deleting} style={dangerVars(colors)}>
          {deleting ? 'Deleting…' : `Confirm delete ${name}`}
        </Button>
      </div>
    </div>
  );
}

/** Terminal delete state: the person is gone, so nothing else in this modal
 *  is meaningful — show what the daemon actually deleted plus `retained`
 *  verbatim, and let the user dismiss on their own terms. */
function DeletedCard({ colors, report, onClose }: {
  colors: ReturnType<typeof useTheme>['colors'];
  report: DeleteReport;
  onClose: () => void;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={{ fontSize: textSize.small, color: colors.text }}>
        Deleted {report.display_name}: {report.meetings_deleted} meeting{report.meetings_deleted === 1 ? '' : 's'},{' '}
        {report.project_links_deleted} project link{report.project_links_deleted === 1 ? '' : 's'},{' '}
        {report.graph_edges_deleted} graph edge{report.graph_edges_deleted === 1 ? '' : 's'},{' '}
        {report.aliases_deleted} alias{report.aliases_deleted === 1 ? '' : 'es'}.
      </div>
      {report.retained.length > 0 && (
        <div style={{
          fontSize: textSize.micro, color: colors.textMuted, borderRadius: radius.md,
          border: `1px solid ${colors.border}`, padding: '8px 10px',
        }}>
          <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '.04em', marginBottom: 4 }}>
            What stays put
          </div>
          {report.retained.map((line, i) => <div key={i}>{line}</div>)}
        </div>
      )}
      <div>
        <Button colors={colors} onClick={onClose} style={primaryVars(colors)}>Close</Button>
      </div>
    </div>
  );
}

/**
 * The projects this person is on, as somewhere to go.
 *
 * The list was already fetched and already on screen — as `<option>` values
 * inside the "log a meeting" form, and nowhere else. So the People graph's own
 * central premise, that people cluster by the projects they share, had no
 * expression at the one surface where you act on a person: you could see that
 * Jane exists, and not that she is on the deal you are looking at.
 *
 * Chips, because that is what a set of short labels is, and `kind="link"`
 * because each one goes somewhere. An error says so and offers the way back
 * rather than rendering as "no projects" — the same rule as the count in the
 * delete confirmation, for the same list.
 */
function PersonProjects({ colors, rows, status, onRetry, onOpen }: {
  colors: ReturnType<typeof useTheme>['colors'];
  rows: PersonProject[];
  status: 'loading' | 'ready' | 'error';
  onRetry: () => void;
  onOpen: (projectId: string) => void;
}) {
  return (
    <section data-testid="person-projects">
      <div style={{ display: 'flex', alignItems: 'center', marginBottom: 7 }}>
        <SectionLabel colors={colors}>Projects</SectionLabel>
      </div>
      {status === 'loading' && rows.length === 0 && <Small colors={colors}>Loading projects…</Small>}
      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <Small colors={colors}>Couldn't load this person's projects.</Small>
          <Button colors={colors} onClick={onRetry} style={miniVars(colors)}>Try again</Button>
        </div>
      )}
      {status === 'ready' && rows.length === 0 && (
        <Small colors={colors}>Not on any project yet — add them from a project's People panel.</Small>
      )}
      {rows.length > 0 && (
        <>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {rows.map(row => (
              <Chip
                key={row.project_id}
                kind="link"
                tone="accent"
                data-testid={`person-project-${row.project_id}`}
                title={`Open ${row.project_name}${row.role ? ` — ${row.role}` : ''}`}
                onClick={() => onOpen(row.project_id)}
              >
                {row.project_name}
              </Chip>
            ))}
          </div>
          {/* Where this person sits in the People graph, in the graph's own
              words. The graph groups by shared project and draws whoever is on
              several of them larger, as a bridge between those groups — its key
              says so, and this is the surface you land on from it, so the two
              have to agree. Only when the list actually loaded: a failed fetch
              gets the message above, never a claim about the picture. */}
          <div data-testid="person-projects-cluster" style={{ marginTop: 6 }}>
            <Small colors={colors}>
              {rows.length > 1
                ? `In the People graph they bridge these ${rows.length} groups.`
                : `In the People graph they sit with ${rows[0].project_name}.`}
            </Small>
          </div>
        </>
      )}
    </section>
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
      {!adding && <Button colors={colors} aria-label="Add related person" onClick={onStart} style={miniVars(colors)}><FiPlus size={12} />Add</Button>}
    </div>
    {status === 'loading' && <Small colors={colors}>Loading relationships…</Small>}
    {status === 'error' && <Small colors={colors}>Couldn't load relationships.</Small>}
    {status === 'ready' && rows.length === 0 && !adding && <Small colors={colors}>No related people yet.</Small>}
    {rows.map(row => <div key={`${row.from_entity_uuid}-${row.to_entity_uuid}-${row.predicate}`} style={{ display: 'flex', gap: 8, alignItems: 'center', padding: '6px 0', borderBottom: `1px solid ${colors.border}` }}>
      <span style={{ color: colors.text, fontSize: textSize.caption, fontWeight: 600 }}>{row.other_person.display_name}</span>
      <span style={{ color: colors.textDim, fontSize: textSize.micro }}>{row.predicate.replace(/_/g, ' ')}</span><span style={{ flex: 1 }} />
      <Button colors={colors} variant="bare" aria-label={`Remove ${row.other_person.display_name} relationship`} onClick={() => onRemove(row)} style={iconVars(colors)}><FiTrash2 size={12} /></Button>
    </div>)}
    {adding && <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 6, marginTop: 8 }}>
      <select aria-label="Related person" value={targetId} onChange={e => onTarget(e.target.value)} style={control(colors)}><option value="">Choose person…</option>{people.map(p => <option key={p.entity_uuid} value={p.entity_uuid}>{p.display_name}</option>)}</select>
      {/* The field used to be an empty box whose only hint was the raw graph
          predicate, `related_to` — a schema token asked of the user as though
          it were English. The common relationships are offered by name; the
          box stays for anything else, because the graph accepts any predicate
          and a fixed list would quietly stop being true. */}
      <input
        aria-label="Relationship type"
        list="person-relationship-predicates"
        value={predicate}
        onChange={e => onPredicate(e.target.value)}
        placeholder="How are they connected?"
        style={control(colors)}
      />
      <datalist id="person-relationship-predicates">
        {RELATIONSHIP_PREDICATES.map(p => <option key={p.value} value={p.value} label={p.label} />)}
      </datalist>
      <div style={{ display: 'flex', gap: 4 }}><Button colors={colors} onClick={onCancel} style={miniVars(colors)}>Cancel</Button><Button colors={colors} onClick={onAdd} disabled={!targetId || !predicate.trim()} style={miniVars(colors)}>Add</Button></div>
    </div>}
  </section>;
}

function MeetingsSection({ colors, personName, rows, projects, status, adding, title, starts, notes, projectId, followUp, followUpAt, followUpNote, saving, onStart, onCancel, onTitle, onStarts, onNotes, onProject, onFollowUp, onFollowUpAt, onFollowUpNote, onAdd, onRetry, onFollowUpDone }: {
  colors: ReturnType<typeof useTheme>['colors']; personName: string; rows: PersonMeeting[];
  projects: PersonProject[];
  status: 'loading' | 'ready' | 'error'; adding: boolean; title: string; starts: string; notes: string;
  projectId: string; followUp: boolean; followUpAt: string; followUpNote: string; saving: boolean;
  onStart: () => void; onCancel: () => void; onTitle: (v: string) => void; onStarts: (v: string) => void;
  onNotes: (v: string) => void; onProject: (v: string) => void; onFollowUp: (v: boolean) => void;
  onFollowUpAt: (v: string) => void; onFollowUpNote: (v: string) => void;
  onAdd: () => void; onRetry: () => void; onFollowUpDone: (m: PersonMeeting) => void;
}) {
  return <section>
    <div style={{ display: 'flex', alignItems: 'center', marginBottom: 7 }}>
      <SectionLabel colors={colors}>Meetings</SectionLabel><span style={{ flex: 1 }} />
      {!adding && <Button colors={colors} aria-label="Log a meeting" onClick={onStart} style={miniVars(colors)}><FiPlus size={12} />Add</Button>}
    </div>
    {status === 'loading' && <Small colors={colors}>Loading meetings…</Small>}
    {status === 'error' && <Small colors={colors}>Couldn't load meetings. <Button colors={colors} variant="bare" className="hover:underline" onClick={onRetry} style={linkVars(colors)}>Retry</Button></Small>}
    {status === 'ready' && rows.length === 0 && !adding && <Small colors={colors}>No meetings logged yet. Add one to put it on their profile and Apple Calendar.</Small>}
    {rows.map(row => <div key={row.id} style={{ display: 'flex', gap: 8, padding: '7px 0', borderBottom: `1px solid ${colors.border}` }}>
      <span style={{ color: colors.cyan, marginTop: 2 }}><FiCalendar size={12} /></span>
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ fontSize: textSize.caption, color: colors.text, fontWeight: 600 }}>{row.title}</div>
        <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>{fmtTime(row.starts_at)}{row.calendar_synced ? ' · Calendar' : ''}{row.calendar_uid ? ' · from iCal' : ''}</div>
        {row.notes ? <div style={{ fontSize: textSize.micro, color: colors.textDim, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{row.notes}</div> : null}
        {row.follow_up_at && !row.follow_up_done && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 4 }}>
            <span style={{ fontSize: textSize.micro, color: colors.cyan }}>Follow up {fmtTime(row.follow_up_at)}{row.follow_up_note ? ` · ${row.follow_up_note}` : ''}</span>
            <Button colors={colors} aria-label="Mark follow-up done" onClick={() => onFollowUpDone(row)} style={miniVars(colors)}><FiCheck size={12} />Done</Button>
          </div>
        )}
      </div>
    </div>)}
    {adding && <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginTop: 8 }}>
      <input aria-label="Meeting title" value={title} onChange={e => onTitle(e.target.value)} placeholder={`Meeting with ${personName}`} style={control(colors)} />
      <input aria-label="Meeting time" type="datetime-local" value={starts} onChange={e => onStarts(e.target.value)} style={control(colors)} />
      {projects.length > 0 && (
        <select aria-label="Meeting project" value={projectId} onChange={e => onProject(e.target.value)} style={control(colors)}>
          <option value="">No project</option>
          {projects.map(p => <option key={p.project_id} value={p.project_id}>{p.project_name}</option>)}
        </select>
      )}
      <textarea aria-label="Meeting notes" value={notes} onChange={e => onNotes(e.target.value)} placeholder="What you covered" rows={3} style={{ ...control(colors), resize: 'vertical' }} />
      <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: textSize.caption, color: colors.text }}>
        <input aria-label="Schedule a follow-up" type="checkbox" checked={followUp} onChange={e => onFollowUp(e.target.checked)} />
        Follow up in a week
      </label>
      {followUp && (
        <>
          <input aria-label="Follow-up time" type="datetime-local" value={followUpAt} onChange={e => onFollowUpAt(e.target.value)} style={control(colors)} />
          <input aria-label="Follow-up note" value={followUpNote} onChange={e => onFollowUpNote(e.target.value)} placeholder="Send recap, check in…" style={control(colors)} />
        </>
      )}
      <div style={{ display: 'flex', gap: 4 }}>
        <Button colors={colors} onClick={onCancel} style={miniVars(colors)}>Cancel</Button>
        <Button colors={colors} onClick={onAdd} pending={saving} disabled={saving || !starts} style={miniVars(colors)}>{saving ? 'Saving…' : 'Log meeting'}</Button>
      </div>
    </div>}
  </section>;
}

function PersonActivityTimeline({ colors, rows, status, onRetry }: { colors: ReturnType<typeof useTheme>['colors']; rows: PersonActivity[]; status: 'loading'|'ready'|'error'; onRetry: () => void }) {
  const icon = (kind: PersonActivity['kind']) => kind === 'memory' ? <FiBookOpen size={12} /> : kind === 'note' ? <FiFileText size={12} /> : kind === 'meeting' ? <FiCalendar size={12} /> : <FiCheckSquare size={12} />;
  return <section><SectionLabel colors={colors}>Recent activity</SectionLabel>
    {status === 'loading' && <Small colors={colors}>Loading activity…</Small>}
    {status === 'error' && <Small colors={colors}>Couldn't load activity. <Button colors={colors} variant="bare" className="hover:underline" onClick={onRetry} style={linkVars(colors)}>Retry</Button></Small>}
    {status === 'ready' && rows.length === 0 && <Small colors={colors}>No activity referencing this person yet.</Small>}
    {rows.map(row => <div key={row.id} style={{ display: 'flex', gap: 8, padding: '7px 0', borderBottom: `1px solid ${colors.border}` }}><span style={{ color: colors.cyan, marginTop: 2 }}>{icon(row.kind)}</span><div style={{ minWidth: 0 }}><div style={{ fontSize: textSize.caption, color: colors.text, fontWeight: 600 }}>{row.title}</div><div style={{ fontSize: textSize.micro, color: colors.textMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{row.detail}</div><div style={{ fontSize: 10, color: colors.textDim }}>{fmtTime(row.timestamp)}</div></div></div>)}
  </section>;
}

function SectionLabel({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: ReactNode }) { return <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono, textTransform: 'uppercase', letterSpacing: '.04em' }}>{children}</div>; }
function Small({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: ReactNode }) { return <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: 6 }}>{children}</div>; }

/** A profile link rendered as a button: clicking navigates the in-app browser
 *  on the Build tab. Not an <a href>, deliberately — an anchor would hand the
 *  URL to the host/system browser and leave the app. */
function LinkButton({ colors, label, title, onClick }: {
  colors: ReturnType<typeof useTheme>['colors'];
  label: string;
  title: string;
  onClick: () => void;
}) {
  return (
    <Tooltip content={title}>
      <Button
        colors={colors}
        variant="bare"
        className="hover:underline"
        type="button"
        onClick={e => { e.preventDefault(); e.stopPropagation(); onClick(); }}
        style={{
          '--pa-btn-fg': colors.cyan,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-bg-active': 'transparent',
          '--pa-btn-pad': '0',
          fontSize: 'inherit', fontFamily: 'inherit',
          gap: 4,
        } as CSSProperties}
      >
        <FiExternalLink size={11} />
        {label}
      </Button>
    </Tooltip>
  );
}

function EditForm({ colors, personName, draft, onChange }: {
  colors: ReturnType<typeof useTheme>['colors'];
  personName: string;
  draft: Draft;
  onChange: (key: EditableKey, value: string) => void;
}) {
  const openInBrowser = useBrowserNavigate();
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {EDITABLE_FIELDS.map(({ key, label, multiline, placeholder, link }) => {
        const href = link ? safeLink(draft[key]) : null;
        return (
          <label key={key} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{
              display: 'flex', alignItems: 'center', gap: 8,
              fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
              textTransform: 'uppercase', letterSpacing: '0.04em',
            }}>
              {label}
              {href && (
                <LinkButton
                    colors={colors}
                    label={key === 'personal_site' ? 'Open site' : key === 'photo_url' ? 'Open image' : 'Open profile'}
                    title={href}
                    onClick={() => openInBrowser(href)}
                />
              )}
            </span>
            {key === 'photo_url' && (
              <PersonFace name={personName} photoUrl={draft.photo_url || null} size={56} accent={colors.cyan} />
            )}
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
        );
      })}
    </div>
  );
}

/** The dock is 400px wide in both placements — wide enough for the widest
 *  thing in it (a datetime input beside its label) and narrow enough that the
 *  board or the graph it opens over is still legible beside it. */
const DOCK_WIDTH = 400;

/**
 * Where the person panel sits — and, after R12, the only thing this file still
 * decides about the panel's chrome.
 *
 * `overlay` pins it to the window's right edge (opened from a project's People
 * list, over the board). `inline` docks it into PeopleView's layout, where the
 * graph shrinks to make room and stays live beside it — which is exactly why
 * `DetailModal` is asked for `placement="contained"` and not for a modal: a
 * scrim would black out the graph this panel exists to explain, and a focus
 * trap would swallow Tab on a surface the user can still see and still click.
 *
 * `data-testid` rides here now because the dock IS the panel from the outside;
 * PeopleView's tests locate the open panel by it.
 */
function PersonDetailDock({ variant, children }: {
  variant: 'inline' | 'overlay';
  children: ReactNode;
}) {
  const { reduceMotion } = useTheme();

  if (variant === 'inline') {
    return (
      <div
        data-testid="person-detail-panel"
        style={{
          width: DOCK_WIDTH, flexShrink: 0, height: '100%', minHeight: 0,
          transition: reduceMotion ? 'none' : `width ${duration.smooth}ms ${ease.smooth}`,
        }}
      >
        {children}
      </div>
    );
  }

  return (
    <div
      data-testid="person-detail-panel"
      style={{
        // The overlay dock starts below the titlebar band rather than under
        // it. `TITLEBAR_HEIGHT` (lib/windowChrome.ts) IS `shell.titlebar`
        // (#1173) — the one source for that geometry, not a re-guessed local.
        position: 'fixed', top: TITLEBAR_HEIGHT, right: 0, bottom: 0,
        width: DOCK_WIDTH, zIndex: 80,
      }}
    >
      {children}
    </div>
  );
}

function inputStyle(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    fontSize: textSize.caption, padding: '6px 9px', borderRadius: radius.md,
    background: colors.fillSubtle, border: `1px solid ${colors.border}`,
    color: colors.text, fontFamily: font.body, outline: 'none', width: '100%',
    boxSizing: 'border-box',
  };
}

function control(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return { ...inputStyle(colors), padding: '5px 7px' };
}

/**
 * The panel's five button shapes, as `Button` custom properties rather than as
 * inline `color`/`background`/`border`. An inline declaration outranks
 * `.pa-btn:hover` in the cascade, so writing the look directly on the element
 * is exactly what leaves a button with no hover, no press and no focus ring —
 * which is what these five used to do.
 */
type Vars = ReturnType<typeof useTheme>['colors'];

/** `.pa-btn`'s own `gap` sits between the spinner and the label, not inside the
 *  label, so a leading glyph carries the gap it used to get from flex. */

function miniVars(colors: Vars): CSSProperties {
  return {
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border': colors.border,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-pad': '4px 7px',
    '--pa-btn-radius': `${radius.md}px`,
    fontFamily: font.body, fontSize: textSize.micro,
  } as CSSProperties;
}

function iconVars(colors: Vars): CSSProperties {
  return {
    '--pa-btn-fg': colors.textDim,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
    '--pa-btn-pad': '2px',
  } as CSSProperties;
}

function linkVars(colors: Vars): CSSProperties {
  return {
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
    '--pa-btn-pad': '0',
    fontFamily: font.body, fontSize: textSize.micro,
  } as CSSProperties;
}

function ghostVars(colors: Vars): CSSProperties {
  return {
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border': colors.border,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    fontFamily: font.body, fontSize: textSize.caption,
  } as CSSProperties;
}

function primaryVars(colors: Vars): CSSProperties {
  return {
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-fg-hover': colors.cyan,
    '--pa-btn-bg': colors.cyanSoft,
    '--pa-btn-bg-hover': colors.cyanSoft,
    '--pa-btn-bg-active': colors.cyanGlow,
    '--pa-btn-border': colors.cyan,
    '--pa-btn-border-hover': colors.cyan,
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    fontFamily: font.body, fontSize: textSize.caption,
  } as CSSProperties;
}

function dangerVars(colors: Vars): CSSProperties {
  return {
    '--pa-btn-fg': colors.danger,
    '--pa-btn-fg-hover': colors.danger,
    '--pa-btn-bg': colors.danger + '14',
    '--pa-btn-bg-hover': colors.danger + '26',
    '--pa-btn-bg-active': colors.danger + '33',
    '--pa-btn-border': colors.danger,
    '--pa-btn-border-hover': colors.danger,
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    fontFamily: font.body, fontSize: textSize.caption,
  } as CSSProperties;
}

/** Mounted once at the app root — overlay dock for a person opened from a
 *  project. The People tab renders the same panel inline so the graph stays up. */
export function PersonDetailModalHost() {
  const personDetail = useCommandCenter(s => s.personDetail);
  const closePersonDetail = useCommandCenter(s => s.closePersonDetail);
  if (!personDetail || personDetail.projectId == null) return null;
  return (
    <PersonDetailModal
      // Remount on target change so local edit/view state resets cleanly.
      key={personDetail.person.entity_uuid}
      variant="overlay"
      projectId={personDetail.projectId}
      person={personDetail.person}
      association={personDetail.association}
      onClose={closePersonDetail}
    />
  );
}
