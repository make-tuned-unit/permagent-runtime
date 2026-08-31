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

import { useEffect, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { removeRoadmapGoal, setGoalAutoApprove, setGoalDependencies } from '../../lib/roadmapClient';
import { useCommandCenter } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
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

/** Goal states whose roadmap dependency wiring may still be edited (#251) —
 *  mirrors the daemon's DEP_EDITABLE_STATES. */
const DEP_EDITABLE_STATES = ['triage', 'ready', 'failed'];

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
  // #251 roadmap editing state
  const [projectGoals, setProjectGoals] = useState<CardResponse[]>([]);
  const [editingDeps, setEditingDeps] = useState(false);
  const [draftDeps, setDraftDeps] = useState<string[]>([]);
  const [savingDeps, setSavingDeps] = useState(false);
  const [depsError, setDepsError] = useState<string | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [removed, setRemoved] = useState(false);
  // #252 per-goal auto-approve toggle state
  const [togglingAutoApprove, setTogglingAutoApprove] = useState(false);
  const [autoApproveError, setAutoApproveError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setLoadError(false);
    Promise.all([
      apiFetch<CardResponse>(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}`),
      apiFetch<BoardColumn[]>(`/api/projects/${encodeURIComponent(projectId)}/columns`).catch(() => [] as BoardColumn[]),
      apiFetch<CardResponse[]>(`/api/projects/${encodeURIComponent(projectId)}/cards?card_type=goal`).catch(() => [] as CardResponse[]),
    ])
      .then(([c, cols, goals]) => {
        if (!live) return;
        setCard(c);
        setColumns(cols);
        setProjectGoals(goals);
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
  const isGoal = card?.cardType === 'goal';
  const dependsOn: string[] = Array.isArray(meta.depends_on)
    ? (meta.depends_on as unknown[]).filter((d): d is string => typeof d === 'string')
    : [];
  const depsEditable =
    isGoal && !removed && !cancelledState && !!stateBinding && DEP_EDITABLE_STATES.includes(stateBinding);
  const goalTitle = (id: string) => projectGoals.find(g => g.id === id)?.title ?? id;

  const startEditDeps = () => {
    setDraftDeps(dependsOn);
    setDepsError(null);
    setEditingDeps(true);
  };

  const saveDeps = async () => {
    setSavingDeps(true);
    setDepsError(null);
    try {
      const updated = await setGoalDependencies(projectId, cardId, draftDeps);
      setCard(prev => (prev ? { ...prev, metadataJson: updated.metadataJson, columnId: updated.columnId } : prev));
      setEditingDeps(false);
      return true;
    } catch (e) {
      setDepsError(e instanceof Error ? e.message : 'Saving dependencies failed.');
      // `false` is the Button primitive's "it failed" — the error is on screen,
      // so the button must not also tick success over it.
      return false;
    } finally {
      setSavingDeps(false);
    }
  };

  const autoApprove = meta.auto_approve === true;
  const autoApproveTogglable = isGoal && !removed && !cancelledState;

  const toggleAutoApprove = async () => {
    setTogglingAutoApprove(true);
    setAutoApproveError(null);
    try {
      const updated = await setGoalAutoApprove(projectId, cardId, !autoApprove);
      setCard(prev => (prev ? { ...prev, metadataJson: updated.metadataJson } : prev));
    } catch (e) {
      setAutoApproveError(e instanceof Error ? e.message : 'Updating auto-approve failed.');
    } finally {
      setTogglingAutoApprove(false);
    }
  };

  const doRemove = async () => {
    setRemoving(true);
    setDepsError(null);
    try {
      const res = await removeRoadmapGoal(projectId, cardId);
      setRemoved(true);
      if (res.cancelled) setCancelledState('cancelled');
      setConfirmingRemove(false);
      return true;
    } catch (e) {
      setDepsError(e instanceof Error ? e.message : 'Removing from roadmap failed.');
      return false;
    } finally {
      setRemoving(false);
    }
  };

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
      return true;
    } catch (e) {
      setCancelError(cancelErrorMessage(e));
      return false;
    } finally {
      setCancelling(false);
    }
  };

  const footer = cancellable || (isGoal && !removed && !cancelledState) ? (
    confirming ? (
      <>
        <span style={{ flex: 1, fontSize: textSize.caption, color: colors.textMuted }}>
          Cancel this goal? Its worker is killed immediately.
        </span>
        <Button
          colors={colors}
          type="button"
          onClick={() => setConfirming(false)}
          disabled={cancelling}
          style={ghostVars(colors)}
        >
          Keep running
        </Button>
        <Button
          colors={colors}
          type="button"
          onClick={doCancel}
          disabled={cancelling}
          style={dangerVars(colors)}
        >
          {cancelling ? 'Cancelling…' : 'Confirm cancel'}
        </Button>
      </>
    ) : confirmingRemove ? (
      <>
        <span style={{ flex: 1, fontSize: textSize.caption, color: colors.textMuted }}>
          Remove from roadmap? Dependents are rewired onto this goal's own dependencies
          {cancellable ? ' and the goal is cancelled' : ''}.
        </span>
        <Button
          colors={colors}
          type="button"
          onClick={() => setConfirmingRemove(false)}
          disabled={removing}
          style={ghostVars(colors)}
        >
          Keep it
        </Button>
        <Button
          colors={colors}
          type="button"
          onClick={doRemove}
          disabled={removing}
          style={dangerVars(colors)}
        >
          {removing ? 'Removing…' : 'Confirm remove'}
        </Button>
      </>
    ) : (
      <>
        {isGoal && !removed && !cancelledState && (
          <Button
            colors={colors}
            type="button"
            onClick={() => setConfirmingRemove(true)}
            style={ghostVars(colors)}
          >
            Remove from roadmap
          </Button>
        )}
        {cancellable && (
          <Button
            colors={colors}
            type="button"
            onClick={() => setConfirming(true)}
            style={dangerVars(colors)}
          >
            Cancel goal
          </Button>
        )}
      </>
    )
  ) : null;

  return (
    <DetailModal title={card?.title ?? 'Goal'} badge={badge} onClose={onClose} footer={footer}>
      {loading ? (
        <div style={{ padding: '32px 0', textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
          Loading…
        </div>
      ) : loadError || !card ? (
        <div style={{ padding: '32px 0', textAlign: 'center', fontSize: textSize.caption, color: colors.textMuted }}>
          Couldn't load this goal.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          {card.description && (
            <div style={{
              fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5,
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

          {/* #252: per-goal auto-approve opt-in — a verified PASS skips the
              manual Review answer for THIS goal only. Fail-closed: a FAIL or
              Uncertain verdict (or a Tier-2 approval dial) still holds. */}
          {isGoal && (
            <div>
              <div style={{
                fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
                textTransform: 'uppercase', letterSpacing: '0.04em',
              }}>
                Auto-approve
              </div>
              <label style={{
                marginTop: 6, display: 'flex', alignItems: 'center', gap: 8,
                fontSize: textSize.caption, color: colors.text,
                cursor: autoApproveTogglable ? 'pointer' : 'default',
                opacity: autoApproveTogglable ? 1 : 0.6,
              }}>
                <input
                  type="checkbox"
                  checked={autoApprove}
                  disabled={!autoApproveTogglable || togglingAutoApprove}
                  onChange={toggleAutoApprove}
                />
                <span>
                  Skip manual Review when the verifier passes
                  {togglingAutoApprove ? ' — saving…' : ''}
                </span>
              </label>
              <div style={{ marginTop: 4, fontSize: textSize.micro, color: colors.textDim }}>
                Only a verified pass auto-approves; failures still wait for you.
              </div>
              {autoApproveError && (
                <div style={{
                  marginTop: 6, fontSize: textSize.caption, color: colors.danger,
                  borderRadius: radius.md, border: `1px solid ${colors.danger}`,
                  background: colors.danger + '14', padding: '8px 12px',
                }}>
                  {autoApproveError}
                </div>
              )}
            </div>
          )}

          {/* #251: roadmap dependency wiring — read view + validated editor. */}
          {isGoal && (
            <div>
              <div style={{
                display: 'flex', alignItems: 'center', gap: 8,
                fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
                textTransform: 'uppercase', letterSpacing: '0.04em',
              }}>
                <span>Dependencies</span>
                {depsEditable && !editingDeps && (
                  // A bare text affordance: the horizontal padding it gains is
                  // pulled back out with a matching negative margin, so the
                  // word sits exactly where it did and the hover fill has a box.
                  <Button
                    colors={colors}
                    variant="bare"
                    type="button"
                    onClick={startEditDeps}
                    style={{
                      '--pa-btn-fg': colors.cyan,
                      '--pa-btn-fg-hover': colors.cyan,
                      '--pa-btn-bg-hover': colors.cyanSoft,
                      '--pa-btn-bg-active': colors.cyanGlow,
                      '--pa-btn-pad': '0 4px',
                      '--pa-btn-radius': `${radius.xs}px`,
                      fontFamily: font.mono,
                      fontSize: textSize.micro,
                      marginLeft: -4,
                    } as CSSProperties}
                  >
                    Edit
                  </Button>
                )}
              </div>
              {editingDeps ? (
                <div style={{ marginTop: 6, display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {projectGoals.filter(g => g.id !== cardId).length === 0 ? (
                    <span style={{ fontSize: textSize.caption, color: colors.textDim }}>
                      No other goals in this project to depend on.
                    </span>
                  ) : (
                    projectGoals.filter(g => g.id !== cardId).map(g => (
                      <label key={g.id} style={{
                        display: 'flex', alignItems: 'center', gap: 8,
                        fontSize: textSize.caption, color: colors.text, cursor: 'pointer',
                      }}>
                        <input
                          type="checkbox"
                          checked={draftDeps.includes(g.id)}
                          onChange={e => setDraftDeps(prev =>
                            e.target.checked ? [...prev, g.id] : prev.filter(d => d !== g.id))}
                        />
                        <span style={{ wordBreak: 'break-word' }}>{g.title}</span>
                      </label>
                    ))
                  )}
                  <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
                    <Button
                      colors={colors}
                      type="button"
                      onClick={() => setEditingDeps(false)}
                      disabled={savingDeps}
                      style={ghostVars(colors)}
                    >
                      Discard
                    </Button>
                    <Button
                      colors={colors}
                      variant="ghostOn"
                      type="button"
                      onClick={saveDeps}
                      disabled={savingDeps}
                      style={{
                        ...ghostVars(colors),
                        '--pa-btn-fg': colors.cyan,
                        '--pa-btn-fg-hover': colors.cyan,
                        '--pa-btn-border': colors.cyan,
                        '--pa-btn-border-hover': colors.cyan,
                        '--pa-btn-weight': 500,
                      } as CSSProperties}
                    >
                      {savingDeps ? 'Saving…' : 'Save dependencies'}
                    </Button>
                  </div>
                </div>
              ) : (
                <div style={{ marginTop: 6, fontSize: textSize.caption, color: colors.text }}>
                  {dependsOn.length === 0
                    ? <span style={{ color: colors.textDim }}>None — this is a root goal.</span>
                    : dependsOn.map(d => (
                      <div key={d} style={{ wordBreak: 'break-word' }}>• {goalTitle(d)}</div>
                    ))}
                </div>
              )}
              {depsError && (
                <div style={{
                  marginTop: 6, fontSize: textSize.caption, color: colors.danger,
                  borderRadius: radius.md, border: `1px solid ${colors.danger}`,
                  background: colors.danger + '14', padding: '8px 12px',
                }}>
                  {depsError}
                </div>
              )}
            </div>
          )}

          {removed && (
            <div style={{
              fontSize: textSize.caption, color: colors.text,
              borderRadius: radius.md, border: `1px solid ${colors.border}`,
              background: colors.cyanSoft, padding: '8px 12px',
            }}>
              Removed from the roadmap — dependents were rewired onto this goal's dependencies.
            </div>
          )}

          {/*
            #524: the layered Evidence panel — the same component the Decision
            Inbox uses, keyed on this goal's card. It self-fetches the deterministic
            dispatch evidence plus the L2 verifier digest, rendering "No verification
            evidence…" when none has been recorded yet. Supersedes the lighter,
            dispatch-only proof-of-work block this modal used to show.
          */}
          <div>
            <div style={{
              fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
              textTransform: 'uppercase', letterSpacing: '0.04em',
            }}>
              Evidence
            </div>
            <EvidenceDigest projectId={projectId} goalId={cardId} />
          </div>

          {cancelledState && (
            <div style={{
              fontSize: textSize.caption, color: colors.text,
              borderRadius: radius.md, border: `1px solid ${colors.danger}`,
              background: colors.danger + '14', padding: '8px 12px',
            }}>
              Goal cancelled — the worker was stopped.
            </div>
          )}
          {cancelError && (
            <div style={{
              fontSize: textSize.caption, color: colors.danger,
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
          <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'nowrap' }}>
            {k}
          </span>
          <span style={{
            fontSize: textSize.caption, color: colors.text, wordBreak: 'break-word', userSelect: 'text',
          }}>
            {v}
          </span>
        </div>
      ))}
    </div>
  );
}

/**
 * The footer's two button shapes, moved from inline `CSSProperties` onto the
 * primitive's custom properties. The resting boxes are the ones that were here
 * — same padding, radius, border, fill and type — what is new is that pressing
 * one now shows it: an inline style cannot express `:hover` or `:active`, so
 * "Confirm cancel" looked identical pressed, unpressed and disabled.
 */
function ghostVars(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    fontFamily: font.body,
    fontSize: textSize.caption,
    lineHeight: '18px',
  } as CSSProperties;
}

function dangerVars(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': colors.danger + '14',
    '--pa-btn-fg': colors.danger,
    '--pa-btn-border': colors.danger,
    '--pa-btn-bg-hover': colors.danger + '26',
    '--pa-btn-border-hover': colors.danger,
    '--pa-btn-bg-active': colors.danger + '14',
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    '--pa-btn-weight': 500,
    fontFamily: font.body,
    fontSize: textSize.caption,
    lineHeight: '18px',
  } as CSSProperties;
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
