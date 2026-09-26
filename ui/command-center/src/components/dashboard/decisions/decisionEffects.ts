/**
 * One record per decision drives BOTH the "If you choose" rows (⑤) and the
 * action bar's buttons (⑨), so the label a row names and the button it names
 * cannot drift apart (U3). Before this, `effectSummaryFor` and `effectTextFor`
 * described the same button two ways on one screen — a goal unblock's panel
 * said "no automatic effect" while its confirm strip said "will wake this goal
 * up".
 *
 * Source of the sentences, in order: the daemon's per-answer `effects` map
 * (daemon lane D4) when it sends one, else the local per-kind table from the
 * inbox design report. Sentences marked † there are not fully traced and say
 * only what is verified.
 *
 * Roles: exactly one `primary` at most (D8), destructive/irreversible actions
 * are `danger` (leading edge, ellipsis, ConfirmDialog, no key, never default).
 */
import type { AnswerBody, Decision } from './types';
import { checkApprovalOf, choiceOptions, draftText, recommendedChoiceId } from './types';
import { scheduledJobId } from './decisionEvidence';

export type ActionRole = 'primary' | 'ghost' | 'danger';

/** A field the action opens inline before it can be sent. */
export type ActionField =
  /** Required free text sent as `input_text` (answer 'input'). */
  | 'input'
  /** Required revised draft sent as `input_text` (answer 'edit'). */
  | 'edit'
  /** Optional one-line reason sent as the answer note. */
  | 'reason'
  /** Optional find-online hint sent as the note (enrichment discard). */
  | 'hint';

export interface VerdictAction {
  /** Stable id; also the key into the daemon's `effects` map. */
  id: string;
  /** The exact button title. The ⑤ row names this same string. */
  label: string;
  /** One consequence sentence. */
  effect: string;
  role: ActionRole;
  /** Single-key shortcut ('1'…'9', '0'), or null. */
  key: string | null;
  /** Return mirrors this action (only a reversible primary). */
  returnDefault: boolean;
  /** What gets sent. Null for `cancel_goal`, which is its own route. */
  body: AnswerBody | null;
  field?: ActionField;
  /** Label of the send button while `field` is open ("Send and retry"). */
  fieldSubmitLabel?: string;
  fieldPlaceholder?: string;
  /** Irreversible: goes through ConfirmDialog, never the undo hold. */
  confirm?: { title: string; consequence: string; confirmLabel: string };
  /** Toast title after the verdict ("Sent back"). */
  doneLabel: string;
}

export interface DecisionPlan {
  actions: VerdictAction[];
  /** Always offered: bare, `H`. */
  later: { label: string; effect: string; key: 'H' };
  /** False for kinds the daemon cannot take an answer for yet (D5). */
  answerable: boolean;
}

export const LATER_EFFECT = 'Keeps it here; it comes back after anything new.';

function asObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function asText(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

/** `autonomy_level` / `budget.ceiling_usd` → "autonomy level" / "budget ceiling usd". */
function humanKey(key: string): string {
  return key.replace(/[._-]+/g, ' ').trim();
}

function displayValue(value: unknown): string | null {
  if (typeof value === 'string') return value.trim() || null;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return null;
}

type Draft = Omit<VerdictAction, 'returnDefault' | 'doneLabel'> & { returnDefault?: boolean; doneLabel?: string };

function primary(id: string, label: string, effect: string, body: AnswerBody | null, extra: Partial<Draft> = {}): Draft {
  return { id, label, effect, role: 'primary', key: '1', returnDefault: true, body, ...extra };
}
function ghost(id: string, label: string, effect: string, key: string | null, body: AnswerBody | null, extra: Partial<Draft> = {}): Draft {
  return { id, label, effect, role: 'ghost', key, body, ...extra };
}
function danger(id: string, label: string, effect: string, body: AnswerBody | null, confirm: VerdictAction['confirm'], doneLabel?: string): Draft {
  return { id, label, effect, role: 'danger', key: null, body, confirm, doneLabel };
}

const APPROVE: AnswerBody = { answer: 'approve' };
const REJECT: AnswerBody = { answer: 'reject' };

function localActions(d: Decision, agentName: string, canCancelGoal: boolean): Draft[] | null {
  const payload = asObject(d.payload);
  switch (d.kind) {
    case 'approve_review':
      return [
        primary('approve', 'Accept work', 'Marks the goal done and lands or hands off the work.', APPROVE, { doneLabel: 'Accepted' }),
        ghost('reject', 'Send back', 'Sends the work back for another attempt with your reason.', '3', REJECT, {
          field: 'reason', fieldSubmitLabel: 'Send back', fieldPlaceholder: 'Why? (optional — the worker sees this)', doneLabel: 'Sent back',
        }),
      ];
    case 'unblock': {
      if (scheduledJobId(d)) {
        return [
          primary('approve', 'Resume job', 'Turns the scheduled job back on from its next run.', APPROVE, { doneLabel: 'Resumed' }),
          ghost('reject', 'Keep paused', 'The job stays paused; you can resume it later.', '3', REJECT, { doneLabel: 'Kept paused' }),
        ];
      }
      // No Reject (D8, the user's delegated ruling): the daemon has no reject arm
      // for a goal unblock, so it left a parked goal with no open question.
      const actions: Draft[] = [
        primary('approve', 'Try again', 'Wakes the goal and gives it more attempts.', APPROVE, { doneLabel: 'Trying again' }),
        ghost('input', 'Retry with notes', 'Sends your notes to the worker as written, then retries.', '2', { answer: 'input' }, {
          field: 'input', fieldSubmitLabel: 'Send and retry', fieldPlaceholder: 'Notes for the worker — sent exactly as written', doneLabel: 'Notes sent',
        }),
      ];
      if (canCancelGoal) {
        actions.unshift(danger('cancel_goal', 'Cancel goal…', 'Stops this goal for good. Its branch and history stay viewable.', null, {
          title: 'Cancel this goal?',
          consequence: `${agentName} stops working on it for good and the worker is shut down. Its branch and history stay viewable, but the goal can't be resumed.`,
          confirmLabel: 'Cancel goal',
        }, 'Goal cancelled'));
      }
      return actions;
    }
    case 'choice': {
      const options = choiceOptions(d);
      const check = checkApprovalOf(d);
      const optionActions: Draft[] = options.slice(0, 9).map((opt, index) => {
        const effect = check
          ? opt.id === 'approve-once' ? 'The command runs this one time.'
            : opt.id === 'approve-and-allowlist' ? `The command runs, and commands starting with “${check.first_token ?? 'it'}” won't ask again.`
            : opt.id === 'deny' ? "The check doesn't run this command."
            : `${agentName} continues with this option.`
          : `${agentName} continues with ${opt.label}.`;
        return ghost(`choice:${opt.id}`, opt.label, effect, String(index + 1), { answer: 'choice', choice_id: opt.id }, { doneLabel: opt.label });
      });
      if (!check) {
        // The daemon has no effect arm for a rejected choice (only a command
        // approval's): the answer is the whole outcome, goal-bound or not.
        optionActions.push(ghost('reject', 'None of these', 'Records that none of the options fit. Nothing else changes.',
          '0', REJECT, { doneLabel: 'None of these' }));
      }
      return optionActions;
    }
    case 'risk_gate': {
      if (payload?.action_class === 'user_data_deletion') {
        const object = d.goal_id ? 'goal' : 'data';
        return [
          danger('approve', `Delete ${object}…`, `Permanently deletes this ${object}. This can't be undone.`, APPROVE, {
            title: `Delete this ${object}?`,
            consequence: `This permanently deletes the ${object}${d.goal_title ? ` “${d.goal_title}”` : ''}. It can't be undone.`,
            confirmLabel: `Delete ${object}`,
          }, 'Deleted'),
          ghost('reject', "Don't delete", 'Nothing is deleted.', '3', REJECT, { doneLabel: 'Kept' }),
        ];
      }
      const action = asText(payload?.description);
      const allow = payload?.action_class === 'repo_worktree_reap'
        ? 'Removes this worktree folder; anything with uncommitted or unpushed work is skipped.'
        : payload?.action_class === 'repo_branch_delete'
          ? 'Deletes this local branch, only if it is still fully merged.'
          : `${agentName} may go ahead with ${action ? action.replace(/[.]$/, '').replace(/^./, c => c.toLowerCase()) : 'this action'}.`;
      return [
        primary('approve', 'Allow', allow, APPROVE, { doneLabel: 'Allowed' }),
        ghost('reject', "Don't allow", `${agentName} won't do it and will look for another way.`, '3', REJECT, { doneLabel: 'Not allowed' }),
      ];
    }
    case 'automation_proposal': {
      const actions: Draft[] = [primary('approve', 'Approve', 'Records your approval.', APPROVE, { doneLabel: 'Approved' })];
      if (draftText(d)) {
        actions.push(ghost('edit', 'Edit and approve', 'Opens the draft for you to change before approving.', '2', { answer: 'edit' }, {
          field: 'edit', fieldSubmitLabel: 'Approve edited', fieldPlaceholder: 'Your version of the draft', doneLabel: 'Approved with edits',
        }));
      }
      actions.push(ghost('reject', "Don't suggest again", `${agentName} won't pitch this automation again.`, '3', REJECT, { doneLabel: 'Dismissed' }));
      return actions;
    }
    case 'enrichment_proposal':
      return [
        primary('approve', 'Save to profile', 'Writes these fields to the profile.', APPROVE, { doneLabel: 'Saved' }),
        ghost('reject', 'Discard', 'Nothing is written; your hint helps next time.', '3', REJECT, {
          field: 'hint', fieldSubmitLabel: 'Discard', fieldPlaceholder: 'How can the agent find this person online? Company, LinkedIn, city…', doneLabel: 'Discarded',
        }),
      ];
    case 'project_intel_proposal':
      return [
        primary('approve', 'Apply', "Adds this to the project's notes.", APPROVE, { doneLabel: 'Applied' }),
        ghost('reject', 'Discard', 'Nothing is written.', '3', REJECT, { doneLabel: 'Discarded' }),
      ];
    case 'file_to_project': {
      const project = asText(payload?.project_name);
      return [
        primary('approve', 'File it', `Files this item under ${project ?? 'the project'}.`, APPROVE, { doneLabel: 'Filed' }),
        ghost('reject', "Don't file", 'Nothing is filed.', '3', REJECT, { doneLabel: 'Not filed' }),
      ];
    }
    case 'council_action':
      return [
        primary('approve', 'File card', 'Files a board card for this action.', APPROVE, { doneLabel: 'Card filed' }),
        ghost('reject', 'Decline', "Recorded; the Council won't pitch it again.", '3', REJECT, { doneLabel: 'Declined' }),
      ];
    case 'tool_approval':
      return [
        primary('approve', 'Allow once', 'The tool runs this one time.', APPROVE, { doneLabel: 'Allowed once' }),
        ghost('reject', 'Deny', `The tool call is refused; ${agentName} is told why.`, '3', REJECT, { doneLabel: 'Denied' }),
      ];
    case 'session_gate':
      // Deviation from the report's table, kept honest: the session advances
      // when the gate is answered in its own tab (decisionEffects history).
      return [
        primary('approve', 'Allow', 'Records your ruling; the session continues once its tab takes it.', APPROVE, { doneLabel: 'Allowed' }),
        ghost('reject', 'Deny', 'Records your ruling; the session stops at this step.', '3', REJECT, { doneLabel: 'Denied' }),
      ];
    case 'malformed':
      return [primary('approve', 'Dismiss', 'Removes it from your queue. Nothing else changes.', APPROVE, { doneLabel: 'Dismissed' })];
    case 'model_upgrade': {
      const model = asText(payload?.model_id);
      return [
        primary('approve', 'Switch model', `Makes ${model ?? 'the proposed model'} the active model.`, APPROVE, { doneLabel: 'Switched' }),
        ghost('reject', 'Keep current', 'The active model is unchanged.', '3', REJECT, { doneLabel: 'Kept current' }),
      ];
    }
    case 'config_change_proposal': {
      const key = asText(payload?.key);
      const value = displayValue(payload?.value);
      return [
        primary('approve', 'Apply change', key && value
          ? `Changes ${humanKey(key)} to ${value}.`
          : 'Applies the proposed setting change.', APPROVE, { doneLabel: 'Applied' }),
        ghost('reject', 'Keep setting', 'No setting is changed.', '3', REJECT, { doneLabel: 'Kept' }),
      ];
    }
    case 'person_merge_proposal':
      return [
        danger('approve', 'Merge people…', 'Combines these two into one person.', APPROVE, {
          title: 'Merge these people?',
          consequence: `${asText(payload?.duplicate_name) ?? 'The duplicate'} is folded into ${asText(payload?.survivor_name) ?? 'the other record'} and then deleted. This can't be undone.`,
          confirmLabel: 'Merge people',
        }, 'Merged'),
        ghost('reject', 'Keep separate', 'Nothing changes.', '3', REJECT, { doneLabel: 'Kept separate' }),
      ];
    case 'person_delete_proposal':
      return [
        danger('approve', 'Delete person…', "Permanently deletes this person's record.", APPROVE, {
          title: 'Delete this person?',
          consequence: `${asText(payload?.display_name) ?? "This person"}'s record is permanently deleted. This can't be undone.`,
          confirmLabel: 'Delete person',
        }, 'Deleted'),
        ghost('reject', 'Keep', 'Nothing changes.', '3', REJECT, { doneLabel: 'Kept' }),
      ];
    case 'capability_gap':
    case 'regression_proposal':
      return null;
    default:
      return null;
  }
}

/**
 * The single source for a decision's verdicts. `canCancelGoal` is false where
 * the surface has no goal to cancel (no project/goal id).
 */
export function decisionPlan(d: Decision, agentName = 'Your agent', canCancelGoal = !!(d.project_id && d.goal_id)): DecisionPlan {
  const drafts = localActions(d, agentName, canCancelGoal);
  const daemonEffects = d.effects && typeof d.effects === 'object' ? d.effects : null;
  const actions: VerdictAction[] = (drafts ?? []).map(a => {
    const fromDaemon = daemonEffects ? asText(daemonEffects[a.id] ?? (a.body?.choice_id ? daemonEffects[a.body.choice_id] : undefined)) : null;
    return {
      ...a,
      effect: fromDaemon ?? a.effect,
      returnDefault: a.role === 'primary' && !a.confirm && (a.returnDefault ?? false),
      doneLabel: a.doneLabel ?? a.label.replace(/…$/, ''),
    };
  });
  // A voice-staged verdict (D29) makes its own action the one tinted button:
  // committing it is the next step, whichever verdict it is.
  const staged = d.staged_answer?.answer;
  if (staged === 'approve' || staged === 'reject') {
    const target = actions.find(a => a.body?.answer === staged && a.role !== 'danger');
    if (target && target.role !== 'primary') {
      for (const a of actions) if (a.role === 'primary') { a.role = 'ghost'; a.returnDefault = false; }
      target.role = 'primary';
      target.returnDefault = true;
    }
  }
  const laterEffect = asText(daemonEffects?.later) ?? LATER_EFFECT;
  return {
    actions,
    later: { label: 'Later', effect: drafts === null ? `${agentName} can't act on an answer to this yet. ${laterEffect}` : laterEffect, key: 'H' },
    answerable: drafts !== null,
  };
}

/**
 * The control that carries the ↩ keycap, so Return does exactly what that
 * control does: an open field's send button while one is open (it becomes the
 * one primary), else the plan's reversible primary. Null = Return is inert.
 */
export function returnTarget(plan: DecisionPlan, fieldAction: VerdictAction | null): VerdictAction | null {
  return fieldAction ?? plan.actions.find(a => a.returnDefault) ?? null;
}

/**
 * What committing a goal unblock's Reject does when the daemon's `effects`
 * map does not say. The daemon's own sentence wins whenever it sends one: it
 * knows both its reject arm and the goal's state. This fallback promises
 * nothing it can't back on every daemon: the narrowed arm (D8) cancels only
 * a goal parked on this decision and otherwise just records the answer, but
 * the earlier arm cancels any goal that isn't already finished, parked or not.
 */
export const UNBLOCK_REJECT_EFFECT =
  "Rejecting can cancel this goal for good — it does when the goal is parked on this decision — and a cancelled goal can't be resumed.";

/**
 * How a staged (spoken, uncommitted) verdict gets committed (D29). Every
 * staged approve or reject is shown with its Discard, whatever commits it:
 *  - `barAction`: the bar button carrying the same answer — a danger one
 *    included, which still asks through ConfirmDialog;
 *  - `rowAction`: no button carries it (a goal unblock has no Reject, a
 *    command approval has no plain reject), so the staged row offers its own;
 *  - neither: this surface can't answer the kind yet; it can only be discarded.
 */
export interface StagedCommit {
  answer: 'approve' | 'reject';
  barAction: VerdictAction | null;
  rowAction: VerdictAction | null;
}

export function stagedCommit(d: Decision, plan: DecisionPlan): StagedCommit | null {
  const answer = d.staged_answer?.answer;
  if (answer !== 'approve' && answer !== 'reject') return null;
  const barAction = plan.actions.find(a => a.body?.answer === answer && a.role !== 'danger')
    ?? plan.actions.find(a => a.body?.answer === answer)
    ?? null;
  if (barAction || !plan.answerable) return { answer, barAction, rowAction: null };
  const doneLabel = answer === 'approve' ? 'Approved' : 'Rejected';
  const fromDaemon = d.effects && typeof d.effects === 'object' ? asText(d.effects[answer]) : null;
  if (d.kind === 'unblock' && d.goal_id && answer === 'reject') {
    // It can cancel the goal, so it confirms first and is never held for
    // undo. The sentence is the daemon's when it sends one (it describes the
    // arm that will actually run), never a local promise it may not keep.
    const effect = fromDaemon ?? UNBLOCK_REJECT_EFFECT;
    return {
      answer, barAction: null,
      rowAction: {
        id: 'staged', label: 'Commit reject…', effect,
        role: 'danger', key: null, returnDefault: false, body: { answer }, doneLabel,
        confirm: { title: 'Commit the staged Reject?', consequence: effect, confirmLabel: 'Reject' },
      },
    };
  }
  return {
    answer, barAction: null,
    rowAction: {
      id: 'staged', label: `Commit ${answer}`, effect: fromDaemon ?? 'Records your answer.',
      role: 'ghost', key: null, returnDefault: false, body: { answer }, doneLabel,
    },
  };
}

const ROW_ORDER: Record<ActionRole, number> = { primary: 0, ghost: 1, danger: 2 };

/** The ⑤ rows, read from the same record as the buttons: [label, sentence].
 *  Read top-down as a recommendation: the primary first, alternatives next,
 *  the destructive choice last (the bar puts it apart at the leading edge),
 *  then Later. */
export function effectRows(plan: DecisionPlan): Array<{ label: string; effect: string }> {
  const ordered = plan.actions
    .map((a, i) => ({ a, i }))
    .sort((x, y) => ROW_ORDER[x.a.role] - ROW_ORDER[y.a.role] || x.i - y.i)
    .map(({ a }) => a);
  return [
    ...ordered.map(a => ({ label: a.label, effect: a.effect })),
    { label: plan.later.label, effect: plan.later.effect },
  ];
}

/** Henry's recommended choice option label, for the eyebrow chip. */
export function recommendedLabel(d: Decision): string | null {
  const id = recommendedChoiceId(d);
  if (!id) return null;
  return choiceOptions(d).find(o => o.id === id)?.label ?? null;
}

// Full tool-call arguments carried by a tool_approval payload / session_gate
// input, pretty-printed for display.
export function toolArgumentsText(d: Decision): string | null {
  if (!d.payload) return null;
  const raw =
    d.kind === 'tool_approval'
      ? (d.payload as { arguments?: unknown }).arguments
      : d.kind === 'session_gate'
        ? (d.payload as { input?: unknown }).input
        : undefined;
  if (raw === undefined || raw === null) return null;
  try {
    return JSON.stringify(raw, null, 2) ?? null;
  } catch {
    return null;
  }
}

/**
 * Informed-reject warning (#458). When an approve_review goal's work was
 * already pushed, sending it back does NOT un-ship it. Null = no warning.
 */
export function pushedRejectWarning(
  kind: string,
  pushTarget: string | null | undefined,
): string | null {
  if (kind !== 'approve_review' || !pushTarget) return null;
  return (
    `This work is already on ${pushTarget} — sending it back won't un-ship it. ` +
    'Rework lands as new commits; undoing the pushed commit needs a revert.'
  );
}
