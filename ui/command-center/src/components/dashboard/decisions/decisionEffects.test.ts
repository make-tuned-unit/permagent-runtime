import { describe, expect, it } from 'vitest';

import { MOCK_PENDING_DECISIONS } from './mockDecisions';
import { LATER_EFFECT, UNBLOCK_REJECT_EFFECT, decisionPlan, effectRows, returnTarget, stagedCommit } from './decisionEffects';
import type { Decision } from './types';

function byId(id: string): Decision {
  const found = MOCK_PENDING_DECISIONS.find(d => d.id === id);
  if (!found) throw new Error(`missing fixture ${id}`);
  return found;
}

const goalUnblock = () => byId('dec-004a-wallclock');

describe('decisionPlan — one record drives the effect rows AND the buttons (U3)', () => {
  it('every fixture kind yields rows whose labels are exactly the button labels, plus Later', () => {
    for (const d of MOCK_PENDING_DECISIONS) {
      const plan = decisionPlan(d, 'Nova');
      const rows = effectRows(plan);
      // Same labels as the buttons (rows are ordered primary → danger), Later last.
      expect(rows.map(r => r.label).slice(0, -1).sort()).toEqual(plan.actions.map(a => a.label).sort());
      expect(rows[rows.length - 1].label).toBe('Later');
      for (const row of rows) expect(row.effect.length).toBeGreaterThan(0);
    }
  });

  it('a daemon effects map changes the sentence of the row and of the same action at once', () => {
    const d: Decision = { ...byId('dec-001'), effects: { approve: 'Lands the branch on main.' } };
    const plan = decisionPlan(d, 'Nova');
    const accept = plan.actions.find(a => a.id === 'approve')!;
    expect(accept.effect).toBe('Lands the branch on main.');
    expect(effectRows(plan).find(r => r.label === accept.label)!.effect).toBe('Lands the branch on main.');
    // Absent keys keep the local sentence.
    expect(plan.actions.find(a => a.id === 'reject')!.effect).toMatch(/another attempt/);
  });

  it('never says "no automatic effect" for a goal unblock (the defect this replaces)', () => {
    const text = effectRows(decisionPlan(goalUnblock())).map(r => r.effect).join(' ');
    expect(text).not.toMatch(/no automatic effect|Missing details/i);
    expect(text).toContain('Wakes the goal');
  });

  it('keeps exactly one tinted primary per view (D8)', () => {
    for (const d of MOCK_PENDING_DECISIONS) {
      expect(decisionPlan(d).actions.filter(a => a.role === 'primary').length).toBeLessThanOrEqual(1);
    }
  });
});

describe('goal unblock buttons (delegated ruling: no separate Reject)', () => {
  it('offers Try again (1, Return), Retry with notes (2) and Cancel goal… — no Reject', () => {
    const plan = decisionPlan(goalUnblock(), 'Nova');
    expect(plan.actions.map(a => a.label)).toEqual(['Cancel goal…', 'Try again', 'Retry with notes']);
    expect(plan.actions.some(a => a.body?.answer === 'reject')).toBe(false);
    const tryAgain = plan.actions.find(a => a.label === 'Try again')!;
    expect(tryAgain).toMatchObject({ role: 'primary', key: '1', returnDefault: true, body: { answer: 'approve' } });
    expect(plan.actions.find(a => a.label === 'Retry with notes')).toMatchObject({ role: 'ghost', key: '2', field: 'input', fieldSubmitLabel: 'Send and retry' });
    const cancel = plan.actions.find(a => a.id === 'cancel_goal')!;
    expect(cancel).toMatchObject({ role: 'danger', key: null, returnDefault: false });
    expect(cancel.confirm?.consequence).toBeTruthy();
    expect(plan.later).toEqual({ label: 'Later', effect: LATER_EFFECT, key: 'H' });
  });

  it('has no Cancel goal without a goal to cancel', () => {
    const d: Decision = { ...goalUnblock(), goal_id: null, project_id: null };
    expect(decisionPlan(d).actions.map(a => a.id)).not.toContain('cancel_goal');
  });

  it('a scheduled-job unblock is Resume job / Keep paused', () => {
    const d: Decision = { ...goalUnblock(), goal_id: null, payload: { target: { type: 'scheduled_job', job_id: 'job-7' }, pause_episode_id: 'ep-9', reason: 'consecutive_failure_limit' } };
    expect(decisionPlan(d).actions.map(a => [a.label, a.key])).toEqual([['Resume job', '1'], ['Keep paused', '3']]);
  });
});

describe('irreversible actions', () => {
  it('user_data_deletion has no primary, no Return default and confirms first', () => {
    const plan = decisionPlan(byId('dec-004d-delete'));
    expect(plan.actions.find(a => a.role === 'primary')).toBeUndefined();
    expect(plan.actions.some(a => a.returnDefault)).toBe(false);
    const del = plan.actions.find(a => a.body?.answer === 'approve')!;
    expect(del).toMatchObject({ label: 'Delete goal…', role: 'danger', key: null });
    expect(del.confirm?.consequence).toMatch(/can't be undone/);
  });

  it.each(['person_merge_proposal', 'person_delete_proposal'])('%s approves only through a confirm', kind => {
    const plan = decisionPlan({ ...byId('dec-001'), kind, goal_id: null, payload: {} });
    const approve = plan.actions.find(a => a.body?.answer === 'approve')!;
    expect(approve.role).toBe('danger');
    expect(approve.confirm).toBeTruthy();
  });
});

describe('kinds the UI did not know (U4)', () => {
  it('model_upgrade and config_change_proposal get real buttons', () => {
    const model = decisionPlan({ ...byId('dec-001'), kind: 'model_upgrade', goal_id: null, payload: { model_id: 'qwen3.5:4b', why_better: 'Faster.' } });
    expect(model.actions.map(a => a.label)).toEqual(['Switch model', 'Keep current']);
    expect(model.actions[0].effect).toBe('Makes qwen3.5:4b the active model.');
    const config = decisionPlan({ ...byId('dec-001'), kind: 'config_change_proposal', goal_id: null, payload: { key: 'autonomy_level', value: 'high' } });
    expect(config.actions[0].effect).toBe('Changes autonomy level to high.');
  });

  it('capability_gap and unknown kinds offer only Later, and say why', () => {
    for (const kind of ['capability_gap', 'regression_proposal', 'future_kind']) {
      const plan = decisionPlan({ ...byId('dec-001'), kind });
      expect(plan.actions).toEqual([]);
      expect(plan.answerable).toBe(false);
      expect(plan.later.effect).toMatch(/can't act on an answer to this yet/);
    }
  });
});

describe('choice', () => {
  it('numbers options 1…n, none tinted, with None of these on 0', () => {
    const plan = decisionPlan(byId('dec-003'), 'Nova');
    expect(plan.actions.map(a => [a.label, a.key, a.role])).toEqual([
      ['Keep it in the existing database', '1', 'ghost'],
      ['Keep it in a separate file', '2', 'ghost'],
      ['None of these', '0', 'ghost'],
    ]);
    expect(plan.actions[0].body).toEqual({ answer: 'choice', choice_id: 'opt-existing-db' });
  });

  // Was "Records that none fit; <agent> pauses this goal." — the daemon has no
  // effect arm for a rejected choice, so nothing pauses anything.
  it('None of these promises only what happens: the answer is recorded', () => {
    const goalBound = decisionPlan({ ...byId('dec-003'), goal_id: 'g-1', project_id: 'p-1' }, 'Nova');
    const none = goalBound.actions.find(a => a.label === 'None of these')!;
    expect(none.effect).toBe('Records that none of the options fit. Nothing else changes.');
    expect(none.effect).not.toMatch(/pause/i);
    expect(decisionPlan({ ...byId('dec-003'), goal_id: null }).actions.find(a => a.label === 'None of these')!.effect)
      .toBe(none.effect);
  });
});

describe('voice-staged verdict (D29)', () => {
  const staged = (answer: string) => ({ answer, note: 'said so', staged_at: new Date().toISOString(), staged_via: 'voice' });

  it('makes the staged verdict the one primary', () => {
    const d: Decision = { ...byId('dec-001'), staged_answer: { answer: 'reject', note: null, staged_at: new Date().toISOString(), staged_via: 'voice' } };
    const plan = decisionPlan(d);
    expect(plan.actions.filter(a => a.role === 'primary').map(a => a.label)).toEqual(['Send back']);
  });

  it('a staged approve on a destructive kind is committed by its danger button, never hidden', () => {
    const d: Decision = { ...byId('dec-001'), kind: 'person_delete_proposal', goal_id: null, payload: {}, staged_answer: staged('approve') };
    const commit = stagedCommit(d, decisionPlan(d))!;
    expect(commit.barAction).toMatchObject({ label: 'Delete person…', role: 'danger' });
    expect(commit.barAction!.confirm).toBeTruthy();
    expect(commit.rowAction).toBeNull();
  });

  it('a staged reject on a goal unblock gets its own confirmed commit', () => {
    const d: Decision = { ...goalUnblock(), staged_answer: staged('reject') };
    const commit = stagedCommit(d, decisionPlan(d))!;
    expect(commit.barAction).toBeNull();
    expect(commit.rowAction).toMatchObject({ label: 'Commit reject…', role: 'danger', body: { answer: 'reject' } });
    expect(commit.rowAction!.confirm).toMatchObject({ title: 'Commit the staged Reject?', confirmLabel: 'Reject' });
  });

  // The daemon that runs the reject says what it does. An older arm cancels
  // any live goal, so a local "otherwise nothing changes" would promise safety
  // the daemon then breaks; the narrowed arm sends record-only copy itself.
  it("a staged goal-unblock reject states the daemon's own sentence, row and confirm alike", () => {
    for (const sentence of ['Cancels this goal for good. It will not run again.', 'Records your answer. Nothing else changes.']) {
      const d: Decision = { ...goalUnblock(), staged_answer: staged('reject'), effects: { reject: sentence } };
      const row = stagedCommit(d, decisionPlan(d))!.rowAction!;
      expect(row.effect).toBe(sentence);
      expect(row.confirm!.consequence).toBe(sentence);
    }
  });

  it('without a daemon sentence it says reject can cancel the goal, and never that nothing else changes', () => {
    const d: Decision = { ...goalUnblock(), staged_answer: staged('reject'), effects: null };
    const row = stagedCommit(d, decisionPlan(d))!.rowAction!;
    expect(row.effect).toBe(UNBLOCK_REJECT_EFFECT);
    expect(row.confirm!.consequence).toBe(UNBLOCK_REJECT_EFFECT);
    expect(UNBLOCK_REJECT_EFFECT).toMatch(/can cancel this goal for good.*parked on this decision/);
    expect(UNBLOCK_REJECT_EFFECT).not.toMatch(/nothing else changes|only recorded|Otherwise/i);
  });

  it('a staged verdict no button carries on an answerable kind commits from the row; an unanswerable kind only discards', () => {
    const malformed: Decision = { ...byId('dec-001'), kind: 'malformed', goal_id: null, payload: {}, staged_answer: staged('reject') };
    const row = stagedCommit(malformed, decisionPlan(malformed))!.rowAction!;
    expect(row).toMatchObject({ label: 'Commit reject', role: 'ghost', body: { answer: 'reject' } });
    expect(row.confirm).toBeUndefined();
    const gap: Decision = { ...byId('dec-001'), kind: 'capability_gap', staged_answer: staged('approve') };
    expect(stagedCommit(gap, decisionPlan(gap))).toEqual({ answer: 'approve', barAction: null, rowAction: null });
    expect(stagedCommit(byId('dec-001'), decisionPlan(byId('dec-001')))).toBeNull();
  });
});

describe('Return target (the ↩ keycap)', () => {
  it('is the open field\'s action while one is open, else the reversible primary', () => {
    const plan = decisionPlan(byId('dec-001'));
    const sendBack = plan.actions.find(a => a.id === 'reject')!;
    expect(returnTarget(plan, null)?.id).toBe('approve');
    expect(returnTarget(plan, sendBack)?.id).toBe('reject');
    expect(returnTarget(decisionPlan(byId('dec-004d-delete')), null)).toBeNull();
  });
});

describe('effectRows order', () => {
  it('lists the primary first and the destructive choice last, before Later', () => {
    const rows = effectRows({
      actions: [
        { id: 'cancel', label: 'Cancel goal…', effect: 'x', role: 'danger', key: null },
        { id: 'retry', label: 'Retry with notes', effect: 'y', role: 'ghost', key: '2' },
        { id: 'approve', label: 'Try again', effect: 'z', role: 'primary', key: '1' },
      ],
      later: { label: 'Later', effect: 'w', key: 'H' },
      answerable: true,
    } as unknown as Parameters<typeof effectRows>[0]);
    expect(rows.map(r => r.label)).toEqual(['Try again', 'Retry with notes', 'Cancel goal…', 'Later']);
  });
});
