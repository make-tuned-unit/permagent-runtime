/**
 * GM4 (dagK) / R5: one state-label vocabulary for a goal, shared by every
 * surface that shows one. Before this, Home's "In flight" list and the goal
 * detail modal computed a goal's label independently — the list read the
 * live `/api/goals/active` hold_note (server text like "held 1/3: verified,
 * waiting on a commit"), the modal read only the board column's name and
 * never learned a goal was held at all. The same goal could read "Ready" in
 * one place and "Held" in the other. Root cause: two label functions, one
 * source of truth. Fixed by giving both surfaces one function.
 *
 * `holdNote` is the ONE server-computed sentence
 * (crate::cost_router::hold_done::hold_user_note, cards.rs's
 * `active_goal_from_row`) that already exists for exactly this purpose — it
 * is never recomputed client-side. Pass it whenever the caller has it (the
 * `/api/goals/active` list has it directly; a single-card view can look its
 * goal up in that same list).
 */

export const GOAL_STATE_LABEL: Record<string, string> = {
  triage: 'Triage',
  ready: 'Ready',
  in_progress: 'In Progress',
  review: 'Review',
  complete: 'Complete',
  cancelled: 'Cancelled',
  failed: 'Failed',
};

export interface GoalStateLabelInputs {
  /** The board column's human name — the fallback when `stateBinding` isn't
   *  one of the known bindings above (a custom column). */
  columnName?: string | null;
  /** GoalState::binding() string ("ready", "in_progress", ...), when known. */
  stateBinding?: string | null;
  /** The server's hold sentence, present only while the goal's "done" claim
   *  is being held back (cost_router::hold_done::hold_user_note). Takes
   *  precedence over the state binding: a held goal is not simply "Ready" or
   *  "In Progress" — the worker's next turn is being gated. */
  holdNote?: string | null;
}

/** The short badge word — what a chip, a list row, or a modal's title-bar
 *  badge shows. */
export function goalStateLabel({ columnName, stateBinding, holdNote }: GoalStateLabelInputs): string {
  if (holdNote) return 'Held';
  if (stateBinding && GOAL_STATE_LABEL[stateBinding]) return GOAL_STATE_LABEL[stateBinding];
  return columnName ?? stateBinding ?? 'Unknown';
}

/** The one-line detail for a held goal — the server's own sentence, verbatim.
 *  Null when the goal isn't held, so callers can render nothing rather than
 *  an empty line. */
export function goalHoldDetail({ holdNote }: GoalStateLabelInputs): string | null {
  return holdNote || null;
}

/**
 * Autonomy label — coordinator ruling (dagK), Microsoft HAX "automation vs
 * augmentation": whether a verified PASS skips manual Review for this goal
 * (`goal_transition::set_goal_auto_approve`) is visible at the interaction
 * level, not only inside the toggle section itself. One function so the goal
 * detail modal's header chip and Home's "In flight" row chip are always the
 * same word for the same boolean — `autoApprove` comes straight off the
 * server (the modal's card metadata, or `/api/goals/active`'s `auto_approve`
 * field), never re-derived.
 */
export function autonomyChip(autoApprove: boolean): { label: string; tone: 'accent' | 'neutral' } {
  return autoApprove
    ? { label: 'Auto-approve', tone: 'accent' }
    : { label: 'Needs review', tone: 'neutral' };
}
