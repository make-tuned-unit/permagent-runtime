import { useEffect, useRef, useState } from 'react';
import { apiFetch, type MeetingTriageReceipt } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import type { ProjectNote } from './types';

const id = (value: unknown): string | null => typeof value === 'string' && value.trim() ? value.trim() : null;
interface SourceResult {
  note: ProjectNote | null;
  noteProblem: string | null;
  receipt: MeetingTriageReceipt | null;
  triageProblem: string | null;
}
/** Response of GET .../recordings/{recording_id}/triage/session, not a guessed session id. */
interface TriageSessionView { sessionId: string; projectId: string; recordingId: string; noteId: string }

/** Durable provenance on an existing card. Fetch only after a user asks to inspect it. */
export function MeetingCardSource({ projectId, metadata, onNavigate }: {
  projectId: string;
  metadata: Record<string, unknown>;
  onNavigate: () => void;
}) {
  const { colors } = useTheme();
  const noteId = id(metadata.source_note_id);
  const recordingId = id(metadata.source_recording_id);
  const triageId = id(metadata.source_triage_id);
  const [expanded, setExpanded] = useState(false);
  const sourceGeneration = useRef(0);
  useEffect(() => {
    sourceGeneration.current += 1;
    return () => { sourceGeneration.current += 1; };
  }, [projectId, noteId, recordingId, triageId]);
  const [revision, setRevision] = useState(0);
  const [result, setResult] = useState<SourceResult | null>(null);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const base = `/api/projects/${encodeURIComponent(projectId)}`;
  const triagePath = recordingId ? `${base}/recordings/${encodeURIComponent(recordingId)}/triage` : null;

  useEffect(() => {
    if (!expanded) return;
    let live = true;
    setResult(null);
    setOpenError(null);
    const noteRequest = noteId ? apiFetch<ProjectNote[]>(`${base}/notes`) : Promise.resolve(null);
    const receiptRequest = triagePath && triageId && noteId
      ? apiFetch<MeetingTriageReceipt>(triagePath) : Promise.resolve(null);
    void Promise.allSettled([noteRequest, receiptRequest]).then(([notes, triage]) => {
      if (!live) return;
      const note = notes.status === 'fulfilled' && Array.isArray(notes.value)
        ? notes.value.find(n => n.id === noteId && n.project_id === projectId) ?? null : null;
      const candidate = triage.status === 'fulfilled' ? triage.value : null;
      const receipt = candidate && candidate.id === triageId && candidate.projectId === projectId
        && candidate.recordingId === recordingId && candidate.noteId === noteId ? candidate : null;
      setResult({
        note,
        noteProblem: note ? null : !noteId ? 'This card has no source note reference.'
          : notes.status === 'rejected' ? 'Could not check the source note. Try again.'
          : 'The source note is no longer available in this project.',
        receipt,
        triageProblem: receipt ? null : !triagePath || !triageId || !noteId
          ? 'This card has an incomplete triage reference.'
          : triage.status === 'rejected' ? 'Could not load meeting triage. It may be unavailable; try again.'
          : 'The triage record does not match this card’s meeting source.',
      });
    });
    return () => { live = false; };
  }, [expanded, revision, base, triagePath, triageId, recordingId, noteId, projectId]);

  const openSession = async (): Promise<boolean> => {
    const receipt = result?.receipt;
    if (!receipt?.sessionId || !triagePath) return false;
    const generation = sourceGeneration.current;
    setOpening(true); setOpenError(null);
    try {
      // A durable receipt can outlive a deleted session. Resolve the real view first.
      const session = await apiFetch<TriageSessionView>(`${triagePath}/session`);
      if (generation !== sourceGeneration.current) return false;
      if (session.sessionId !== receipt.sessionId || session.projectId !== projectId
        || session.recordingId !== recordingId || session.noteId !== noteId) {
        throw new Error('The triage session does not match this meeting.');
      }
      const store = useCommandCenter.getState();
      await store.switchToSession(session.sessionId);
      // switchToSession disowns a deleted id instead of throwing on its 404.
      if (useCommandCenter.getState().chatSessionId !== session.sessionId) {
        throw new Error('The triage session is no longer available.');
      }
      if (generation !== sourceGeneration.current) return false;
      useCommandCenter.getState().openChatDock();
      onNavigate();
      return true;
    } catch {
      if (generation !== sourceGeneration.current) return false;
      setOpenError('Could not open this meeting’s triage session. Refresh the source and try again.');
      return false;
    } finally { if (generation === sourceGeneration.current) setOpening(false); }
  };

  return (
    <section aria-label="Meeting source" style={{ border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: space.lg, display: 'flex', flexDirection: 'column', gap: space.md }}>
      <div style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600, color: colors.text }}>Source meeting</div>
      <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>Created from meeting triage on this project. Reviewing the source does not run this task.</div>
      {!expanded ? <Button colors={colors} flashSuccess={false} onClick={() => setExpanded(true)}>View meeting source</Button> : <>
        {!result ? <div role="status">Checking meeting source…</div> : <>
          {result.note && <div style={{ fontSize: textSize.caption, color: colors.text }}>{result.note.title || 'Meeting note'}</div>}
          {result.noteProblem && <div role="status" style={{ color: colors.textMuted }}>{result.noteProblem}</div>}
          {result.receipt && <div style={{ fontSize: textSize.caption, color: colors.textMuted }}>Orchestrator triage: {result.receipt.status}.</div>}
          {result.triageProblem && <div role="status" style={{ color: colors.textMuted }}>{result.triageProblem}</div>}
          {result.receipt && !result.receipt.sessionId && <div style={{ color: colors.textMuted }}>No triage session is available yet.</div>}
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md }}>
            {result.note && <Button colors={colors} flashSuccess={false} onClick={() => {
              useCommandCenter.getState().focusProjectNote(projectId, result.note!.id);
              onNavigate();
            }}>Open source note</Button>}
            {result.receipt?.sessionId && <Button colors={colors} flashSuccess={false} disabled={opening} onClick={openSession}>{opening ? 'Opening…' : 'Open triage session'}</Button>}
            <Button colors={colors} flashSuccess={false} disabled={opening} onClick={() => setRevision(n => n + 1)}>Refresh source</Button>
          </div>
        </>}
        {openError && <div role="alert" style={{ color: colors.danger }}>{openError}</div>}
      </>}
    </section>
  );
}
