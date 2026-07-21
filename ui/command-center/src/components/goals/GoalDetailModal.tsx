/**
 * GoalDetailModal (#503) — the single goal-detail view, reachable from every
 * goal surface (Kanban card, Decision Inbox row, dashboard "in flight" item).
 *
 * It is self-contained: given a {projectId, cardId} it fetches the full card
 * (GET /api/projects/{pid}/cards/{cid}) plus the project columns (to resolve the
 * card's column into a human goal state), renders title / description / state /
 * proof-of-work / worker session / attempts, and offers an immediate Cancel.
 *
 * Cancel calls the existing POST /api/projects/{pid}/cards/{cid}/cancel (#500):
 * the worker is SIGKILLed via its process group, the goal moves to the terminal
 * Cancelled state, and its open decisions are superseded. The cancelling
 * transition emits `goal_state_changed`, so the live surfaces (useGoalEvents /
 * useLiveGoals) refresh themselves — the modal only reflects the new state and
 * lets the user close.
 *
 * GoalDetailModalHost is mounted once at the app root and renders the modal
 * whenever the store's `goalDetail` target is set (mirrors the discussDecision
 * deep-link seam). Built on the generic DetailModal shell so the CRM People
 * modal (epic slice 2) can reuse the same chrome.
 */

import { useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DetailModal } from '../common/DetailModal';
import { EvidenceDigest } from '../dashboard/decisions/EvidenceDigest';

interface CardResponse {
  id: string;
  projectId: string;
  cardType: string;
  title: string;
  description: string;
  columnId: string;
  assignedTo: string | null;
  metadataJson: Record<string, unknown> | null;
  createdAt: string;
  updatedAt: string;
}

interface BoardColumn {
  id: string;
  name: string;
  stateBinding?: string | null;
}

/** Goal lifecycle states a goal can still be cancelled from (mirrors #490);
 *  Failed goals (#250) can be abandoned too. */
const CANCELLABLE_STATES = ['triage', 'ready', 'in_progress', 'review', 'failed'];

function fmtTime(iso: string): string {
  const t = Date.parse(iso);
  return Number.isFinite(t) ? new Date(t).toLocaleString() : iso;
}

/**
 * Map a cancel failure to a clear, actionable message. The cancel endpoint
 * returns 409 Conflict for two distinct cases — the policy classes the action
 * as decision-required (Tier 2), or the goal is already in a terminal state —
 * and both carry an explanatory server message, which we surface verbatim when
 * present. We only replace the bare `Unknown error` / `HTTP <n>` fallbacks.
 */
function cancelErrorMessage(e: unknown): string {
  const status = (e as { status?: number } | null)?.status;
  const raw = e instanceof Error ? e.message : '';
  const hasServerText = raw && !/^Unknown error$|^HTTP \d+$/.test(raw);
  if (hasServerText) return raw;
  if (status === 409) {
    return 'Cancel was rejected — this goal needs approval or is already in a terminal state.';
  }
  if (status === undefined || (status >= 500 && status < 600)) {
    return "Couldn't reach the server to cancel. Please try again.";
  }
  return raw || 'Cancel failed.';
}

export function GoalDetailModal({
  projectId,
  cardId,
  onClose,
}: {
  projectId: string;
  cardId: string;
  onClose: () => void;
}) {
  const { colors } = useTheme();
  const [card, setCard] = useState<CardResponse | null>(null);
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const [cancelledState, setCancelledState] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setLoadError(false);
    Promise.all([
      apiFetch<CardResponse>(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}`),
      apiFetch<BoardColumn[]>(`/api/projects/${encodeURIComponent(projectId)}/columns`).catch(() => [] as BoardColumn[]),
    ])
      .then(([c, cols]) => {
        if (!live) return;
        setCard(c);
        setColumns(cols);
      })
      .catch(() => { if (live) setLoadError(true); })
      .finally(() => { if (live) setLoading(false); });
    return () => { live = false; };
  }, [projectId, cardId]);

  const column = columns.find(c => c.id === card?.columnId) ?? null;
  const stateBinding = cancelledState ?? column?.stateBinding ?? null;
  const stateLabel = cancelledState
    ? 'Cancelled'
    : column?.name ?? (stateBinding ?? 'Unknown');
  const cancellable =
    !cancelledState && !!stateBinding && CANCELLABLE_STATES.includes(stateBinding);

  const meta = card?.metadataJson ?? {};
  const workerSessionId = typeof meta.worker_session_id === 'string' ? meta.worker_session_id : null;
  const attemptCount = typeof meta.attempt_count === 'number' ? meta.attempt_count : null;

  const badge = cancelledState
    ? { label: 'Cancelled', color: colors.danger, bg: colors.danger + '24' }
    : column
      ? { label: stateLabel, color: colors.cyan, bg: colors.cyanSoft }
      : null;

  const doCancel = async () => {
    setCancelling(true);
    setCancelError(null);
    try {
      const res = await apiFetch<{ cancelled: boolean; state: string }>(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}/cancel`,
        { method: 'POST' },
      );
      setCancelledState(res.state || 'cancelled');
      setConfirming(false);
    } catch (e) {
      setCancelError(cancelErrorMessage(e));
    } finally {
      setCancelling(false);
    }
  };

  const footer = cancellable ? (
    confirming ? (
      <>
        <span style={{ flex: 1, fontSize: 12, color: colors.textMuted }}>
          Cancel this goal? Its worker is killed immediately.
        </span>
        <button onClick={() => setConfirming(false)} disabled={cancelling} style={ghostBtn(colors)}>
          Keep running
        </button>
        <button onClick={doCancel} disabled={cancelling} style={dangerBtn(colors)}>
          {cancelling ? 'Cancelling…' : 'Confirm cancel'}
        </button>
      </>
    ) : (
      <button onClick={() => setConfirming(true)} style={dangerBtn(colors)}>
        Cancel goal
      </button>
    )
  ) : null;

  return (
    <DetailModal title={card?.title ?? 'Goal'} badge={badge} onClose={onClose} footer={footer}>
      {loading ? (
        <div style={{ padding: '32px 0', textAlign: 'center', fontSize: 12, color: colors.textDim }}>
          Loading…
        </div>
      ) : loadError || !card ? (
        <div style={{ padding: '32px 0', textAlign: 'center', fontSize: 12, color: colors.textMuted }}>
          Couldn't load this goal.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          {card.description && (
            <div style={{
              fontSize: 13, color: colors.textMuted, lineHeight: 1.5,
              whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
            }}>
              {card.description}
            </div>
          )}

          <MetaGrid colors={colors} rows={[
            ['State', stateLabel],
            ['Assigned to', card.assignedTo || '—'],
            ['Worker session', workerSessionId || '—'],
            ['Attempts', attemptCount != null ? String(attemptCount) : '—'],
            ['Created', fmtTime(card.createdAt)],
            ['Updated', fmtTime(card.updatedAt)],
          ]} />

          {/*
            #524: the layered Evidence panel — the same component the Decision
            Inbox uses, keyed on this goal's card. It self-fetches the deterministic
            dispatch evidence plus the L2 verifier digest, rendering "No verification
            evidence…" when none has been recorded yet. Supersedes the lighter,
            dispatch-only proof-of-work block this modal used to show.
          */}
          <div>
            <div style={{
              fontSize: 11, color: colors.textDim, fontFamily: font.mono,
              textTransform: 'uppercase', letterSpacing: '0.04em',
            }}>
              Evidence
            </div>
            <EvidenceDigest projectId={projectId} goalId={cardId} />
          </div>

          {cancelledState && (
            <div style={{
              fontSize: 12, color: colors.text,
              borderRadius: radius.md, border: `1px solid ${colors.danger}`,
              background: colors.danger + '14', padding: '8px 12px',
            }}>
              Goal cancelled — the worker was stopped.
            </div>
          )}
          {cancelError && (
            <div style={{
              fontSize: 12, color: colors.danger,
              borderRadius: radius.md, border: `1px solid ${colors.danger}`,
              background: colors.danger + '14', padding: '8px 12px',
            }}>
              {cancelError}
            </div>
          )}
        </div>
      )}
    </DetailModal>
  );
}

function MetaGrid({ colors, rows }: {
  colors: ReturnType<typeof useTheme>['colors'];
  rows: [string, string][];
}) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 14px' }}>
      {rows.map(([k, v]) => (
        <div key={k} style={{ display: 'contents' }}>
          <span style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'nowrap' }}>
            {k}
          </span>
          <span style={{
            fontSize: 12, color: colors.text, wordBreak: 'break-word', userSelect: 'text',
          }}>
            {v}
          </span>
        </div>
      ))}
    </div>
  );
}

function ghostBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '6px 14px', borderRadius: radius.md,
    border: `1px solid ${colors.border}`, background: 'none',
    fontFamily: font.body, fontSize: 12, color: colors.textMuted, cursor: 'pointer',
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
export function GoalDetailModalHost() {
  const goalDetail = useCommandCenter(s => s.goalDetail);
  const closeGoalDetail = useCommandCenter(s => s.closeGoalDetail);
  if (!goalDetail) return null;
  return (
    <GoalDetailModal
      projectId={goalDetail.projectId}
      cardId={goalDetail.cardId}
      onClose={closeGoalDetail}
    />
  );
}
