/**
 * GoalDetailModal — goal-specific content filling the reusable
 * {@link EntityDetailModal} shell (#503).
 *
 * Reachable from all three goal surfaces — the Kanban card (ProjectsView), the
 * Home "In flight" list (InFlightCard), and the Decision Inbox — each opens this
 * same modal by `{ projectId, goalId }`. It loads the card via
 * GET /api/projects/{pid}/cards/{cid} (CardResponse, camelCase) and shows:
 *   - title + description (EDITABLE — saved via PATCH, the only fields the user
 *     owns directly; state moves via drag/cancel, not free text)
 *   - state (from metadataJson.goal_state, else the column binding)
 *   - worker / session id (assignedTo + metadataJson.worker_session_id)
 *   - evidence digest summary, when present
 * and a working CANCEL action → POST .../cancel (the #500 backend: kills the
 * worker, moves to terminal Cancelled, leaves the active set). Cancel is gated
 * to non-terminal states.
 *
 * On any successful mutation the daemon emits `goal_state_changed`, so every
 * live surface (useGoalEvents subscribers) refreshes within a frame; we also
 * call onChanged so the opener can refetch its own snapshot immediately.
 */

import { useCallback, useEffect, useState } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { EntityDetailModal, DetailSection } from '../common/EntityDetailModal';

interface CardDetail {
  id: string;
  projectId: string;
  cardType: string;
  title: string;
  description: string;
  columnId: string;
  assignedTo: string | null;
  metadataJson: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

/** State labels that are still cancellable (mirrors ProjectsView CANCELLABLE_STATES). */
const CANCELLABLE_STATES = ['triage', 'ready', 'in_progress', 'review'];

const STATE_LABEL: Record<string, string> = {
  triage: 'Triage',
  ready: 'Ready',
  in_progress: 'In Progress',
  review: 'Review',
  complete: 'Complete',
  cancelled: 'Cancelled',
  parked: 'Parked',
  failed: 'Failed',
};

interface Props {
  projectId: string;
  goalId: string;
  onClose: () => void;
  /** Called after a successful edit/cancel so the opener refetches immediately. */
  onChanged?: () => void;
}

export function GoalDetailModal({ projectId, goalId, onClose, onChanged }: Props) {
  const { colors } = useTheme();
  const [card, setCard] = useState<CardDetail | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState('');
  const [draftDesc, setDraftDesc] = useState('');
  const [busy, setBusy] = useState<'save' | 'cancel' | null>(null);

  const load = useCallback(async () => {
    try {
      const c = await apiFetch<CardDetail>(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(goalId)}`,
      );
      setCard(c);
      setDraftTitle(c.title);
      setDraftDesc(c.description);
    } catch {
      setLoadError(true);
    }
  }, [projectId, goalId]);

  useEffect(() => { load(); }, [load]);

  // Derive state from goal_state metadata (authoritative when present) else
  // best-effort from the metadata; column binding is resolved server-side.
  const meta = card?.metadataJson ?? {};
  const stateRaw = (meta.goal_state as string | undefined) ?? '';
  const stateLabel = STATE_LABEL[stateRaw] ?? (stateRaw || 'Unknown');
  const isGoal = card?.cardType === 'goal';
  const canCancel = isGoal && (!stateRaw || CANCELLABLE_STATES.includes(stateRaw));
  const sessionId = (meta.worker_session_id as string | undefined) ?? null;
  const evidence = ((meta.verification as Record<string, unknown> | undefined)
    ?.evidence_digest) as { verifier_summary?: string } | undefined;

  const save = async () => {
    if (!card) return;
    setBusy('save');
    try {
      await apiFetch(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(goalId)}`,
        {
          method: 'PATCH',
          body: JSON.stringify({ title: draftTitle.trim(), description: draftDesc }),
        },
      );
      setEditing(false);
      await load();
      onChanged?.();
    } catch {
      /* keep the draft so the user can retry */
    } finally {
      setBusy(null);
    }
  };

  const cancelGoal = async () => {
    if (!card) return;
    setBusy('cancel');
    try {
      await apiFetch(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(goalId)}/cancel`,
        { method: 'POST' },
      );
      onChanged?.();
      onClose();
    } catch {
      setBusy(null);
    }
  };

  const subtitle = card
    ? `${card.cardType}${stateRaw ? ` · ${stateLabel}` : ''}`
    : undefined;

  const footer = card && (
    <>
      {editing ? (
        <>
          <Btn primary disabled={busy !== null || !draftTitle.trim()} onClick={save}>
            {busy === 'save' ? 'Saving…' : 'Save'}
          </Btn>
          <Btn disabled={busy !== null} onClick={() => { setEditing(false); setDraftTitle(card.title); setDraftDesc(card.description); }}>
            Cancel edit
          </Btn>
        </>
      ) : (
        <Btn disabled={busy !== null} onClick={() => setEditing(true)}>Edit</Btn>
      )}
      <div style={{ flex: 1 }} />
      {canCancel && !editing && (
        <Btn danger disabled={busy !== null} onClick={cancelGoal}>
          {busy === 'cancel' ? 'Cancelling…' : 'Cancel goal'}
        </Btn>
      )}
    </>
  );

  return (
    <EntityDetailModal
      title={editing ? 'Edit goal' : (card?.title ?? (loadError ? 'Goal' : 'Loading…'))}
      subtitle={subtitle}
      footer={footer}
      onClose={onClose}
    >
      {loadError ? (
        <div style={{ fontSize: 13, color: colors.textMuted }}>Couldn’t load this goal.</div>
      ) : !card ? (
        <div style={{ fontSize: 13, color: colors.textDim }}>Loading…</div>
      ) : editing ? (
        <>
          <DetailSection label="Title">
            <input
              value={draftTitle}
              onChange={e => setDraftTitle(e.target.value)}
              style={inputStyle(colors)}
            />
          </DetailSection>
          <DetailSection label="Description">
            <textarea
              value={draftDesc}
              onChange={e => setDraftDesc(e.target.value)}
              rows={5}
              style={{ ...inputStyle(colors), resize: 'vertical' }}
            />
          </DetailSection>
        </>
      ) : (
        <>
          {card.description && (
            <DetailSection label="Description">
              <span style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>{card.description}</span>
            </DetailSection>
          )}
          {stateRaw && <DetailSection label="State">{stateLabel}</DetailSection>}
          {card.assignedTo && <DetailSection label="Worker">{card.assignedTo}</DetailSection>}
          {sessionId && (
            <DetailSection label="Session">
              <span style={{ fontFamily: font.mono, fontSize: 11, color: colors.textMuted, wordBreak: 'break-all' }}>
                {sessionId}
              </span>
            </DetailSection>
          )}
          {evidence?.verifier_summary && (
            <DetailSection label="Evidence">
              <span style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', color: colors.textMuted }}>
                {evidence.verifier_summary}
              </span>
            </DetailSection>
          )}
        </>
      )}
    </EntityDetailModal>
  );
}

function inputStyle(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    width: '100%', boxSizing: 'border-box',
    borderRadius: radius.md, border: `1px solid ${colors.border}`,
    background: colors.inputBg, color: colors.text,
    fontFamily: font.body, fontSize: 13, padding: '8px 10px', outline: 'none',
  };
}

/** Inline button matching the DecisionItem/post-#273 convention. */
function Btn({ primary = false, danger = false, disabled = false, onClick, children }: {
  primary?: boolean; danger?: boolean; disabled?: boolean;
  onClick: () => void; children: React.ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        borderRadius: radius.md, fontFamily: font.body, fontSize: 12,
        fontWeight: primary ? 600 : 500, padding: '6px 16px',
        cursor: disabled ? 'default' : 'pointer', opacity: disabled ? 0.6 : 1,
        border: primary ? 'none' : `1px solid ${colors.border}`,
        background: primary ? colors.ribbonGradient : colors.surface,
        color: primary ? colors.textOnAccent : danger ? colors.danger : colors.textMuted,
      }}
    >
      {children}
    </button>
  );
}
