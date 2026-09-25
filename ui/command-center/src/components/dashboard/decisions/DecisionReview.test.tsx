/** @vitest-environment jsdom */
import { act, Simulate } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  goalDetail: null as { projectId: string; cardId: string } | null,
  apiFetch: vi.fn(), openGoalDetail: vi.fn(), closeGoalDetail: vi.fn(), toast: vi.fn(),
  answer: vi.fn(), showAll: vi.fn(), refresh: vi.fn(), discard: vi.fn(), history: vi.fn(),
  cancelGoal: vi.fn(), dispatchEvidence: vi.fn(),
}));
vi.mock('../../../lib/api', () => ({ apiFetch: mocks.apiFetch }));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (select: (state: Record<string, unknown>) => unknown) => select(mocks),
}));
vi.mock('../../../lib/notifications', () => ({ toast: mocks.toast }));
vi.mock('../../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ activeCount: 0, loaded: true }) }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('./DiscussionPane', () => ({
  DiscussionPane: ({ decisionId, inputRef }: { decisionId: string; inputRef?: React.RefObject<HTMLTextAreaElement> }) =>
    <textarea aria-label={`Ask about ${decisionId}`} ref={inputRef} />,
}));
vi.mock('./EvidenceDigest', () => ({ EvidenceDigest: () => <div>Dispatch digest</div> }));
vi.mock('./client', () => ({ decisionsClient: { dispatchEvidence: mocks.dispatchEvidence, cancelGoal: mocks.cancelGoal } }));

import { DecisionInbox } from './DecisionInbox';
import { UNDO_MS } from './DecisionReview.undo';
import { MOCK_PENDING_DECISIONS } from './mockDecisions';
import { decisionPlan, effectRows } from './decisionEffects';
import type { Decision, DecisionsResponse } from './types';
import type { useDecisions } from './useDecisions';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
const onClose = vi.fn();
function decision(id: string, patch: Partial<Decision> = {}): Decision {
  return { id, kind: 'risk_gate', tier: 2, headline: `Headline ${id}`, detail: '<b>Plain detail</b>',
    goal_id: null, project_id: null, payload: { action_class: 'cc_shell', description: 'Run a script' }, rank: null, status: 'open', answer: null,
    answer_note: null, answer_choice_id: null, answer_input: null, acted_by: null,
    created_at: '2026-01-01', resolved_at: null, ...patch };
}
const fixture = (id: string) => ({ ...MOCK_PENDING_DECISIONS.find(d => d.id === id)! });
function inbox(rows: Decision[], extra: Partial<DecisionsResponse> = {}): ReturnType<typeof useDecisions> {
  return { data: { decisions: rows, total_pending: rows.length, handled_count: 0, goals_in_flight: 0,
    goals_needing_attention: 0, attention_goals: [], oldest_pending_at: null, ...extra }, loading: false, error: false,
    answer: mocks.answer, refresh: mocks.refresh, showAll: mocks.showAll, discardStaged: mocks.discard, loadHistory: mocks.history };
}
function render(rows: Decision[], placement: 'center' | 'contained' = 'center', extra: Partial<DecisionsResponse> = {}) {
  act(() => root.render(<DecisionInbox inbox={inbox(rows, extra)} onClose={onClose} placement={placement} />));
}
function buttons() { return Array.from(document.body.querySelectorAll('button')); }
function button(label: string) {
  const found = buttons().find(b => b.getAttribute('aria-label') === label || b.textContent === label
    || b.querySelector('.pa-btn__label')?.firstChild?.textContent === label);
  expect(found, label).toBeTruthy();
  return found!;
}
async function click(label: string) { await act(async () => button(label).click()); }
function key(value: string, init: KeyboardEventInit = {}, target: Element = document.body.querySelector('[data-testid="inbox-body"]')!) {
  act(() => { target.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...init })); });
}
function current() { return document.body.querySelector('article[data-testid^="decision-"]')?.getAttribute('data-testid'); }
function live() { return document.body.querySelector('[data-testid="decision-review"] [role="status"]')?.textContent ?? ''; }
const ok = (d: Decision) => ({ ok: true, decision: d, effect: null, effect_error: null });
async function elapse(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
function barLabels() {
  return Array.from(document.body.querySelectorAll('[role="toolbar"] button[data-action]:not([data-action="later"])'))
    .map(b => b.querySelector('.pa-btn__label')?.firstChild?.textContent);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  mocks.goalDetail = null;
  mocks.openGoalDetail.mockImplementation((projectId: string, cardId: string) => { mocks.goalDetail = { projectId, cardId }; });
  mocks.closeGoalDetail.mockImplementation(() => { mocks.goalDetail = null; });
  mocks.showAll.mockResolvedValue(undefined); mocks.refresh.mockResolvedValue(undefined); mocks.history.mockResolvedValue([]);
  mocks.dispatchEvidence.mockResolvedValue(null); mocks.cancelGoal.mockResolvedValue(undefined); mocks.discard.mockResolvedValue(undefined);
  mocks.answer.mockImplementation(async (id: string) => ok(decision(id)));
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); vi.useRealTimers(); });

describe('layout: one question, read top to bottom', () => {
  it('the screenshot case shows the full title and a plain cause — raw detail only under Technical details (U1, U2)', () => {
    const d = fixture('dec-004a-wallclock');
    render([d]);
    const article = document.body.querySelector('article')!;
    expect(article.querySelector('h2')!.textContent).toBe(d.goal_title);
    expect(document.body.querySelector('[data-testid="decision-cause"]')!.textContent)
      .toBe('Ran out of time: each attempt gets 4 min, and this one used all of it. It made no changes worth keeping.');
    expect(article.textContent).toContain('Goal ran out of time');
    const details = document.body.querySelector('[data-testid="technical-details"]') as HTMLDetailsElement;
    expect(details.open).toBe(false);
    expect(details.textContent).toContain('01a0d5ea');
    // Outside the disclosure: no raw detail, no truncated headline, no goal title twice.
    const outside = article.textContent!.replace(details.textContent!, '');
    expect(outside).not.toContain('wallclock cap exhausted');
    expect(outside).not.toContain(d.headline);
    expect(outside.split(d.goal_title!).length - 1).toBe(1);
    expect(document.body.textContent).not.toContain('Parked goals');
    expect(document.body.textContent).not.toMatch(/tier \d|unblock/);
  });

  it('one record drives the If-you-choose rows and the bar buttons (U3)', () => {
    const d = fixture('dec-004a-wallclock');
    render([d]);
    const rows = Array.from(document.body.querySelectorAll('[data-testid="effect-rows"] > div'))
      .map(row => row.firstElementChild!.textContent);
    const plan = decisionPlan(d, 'Henry');
    expect(rows).toEqual(effectRows(plan).map(r => r.label));
    expect(new Set(barLabels())).toEqual(new Set(plan.actions.map(a => a.label)));
  });

  it('a goal unblock has no Reject; Cancel goal… sits alone and is the only danger', () => {
    render([fixture('dec-004a-wallclock')]);
    expect(barLabels()).toEqual(['Cancel goal…', 'Retry with notes', 'Try again']);
    expect(buttons().some(b => /Reject/.test(b.textContent ?? ''))).toBe(false);
    expect(document.body.querySelectorAll('.pa-btn--primary').length).toBe(1);
  });

  it('unknown kinds render plain text only and offer Later, never a dead control (ruling 8)', () => {
    render([decision('future', { kind: 'future_kind' })]);
    expect(document.body.textContent).toContain("can't act on an answer to this yet");
    expect(document.body.querySelector('article b')).toBeNull();
    expect(barLabels()).toEqual([]);
    expect(button('Later')).toBeTruthy();
  });
});

describe('verdicts: auto-advance with undo instead of a confirm (U12)', () => {
  it('1 holds the primary, advances, and sends only after the undo window', async () => {
    render([decision('a'), decision('b')]);
    key('1');
    expect(current()).toBe('decision-b');
    expect(mocks.answer).not.toHaveBeenCalled();
    expect(mocks.toast).toHaveBeenCalledWith('Allowed', 'Headline a', undefined, expect.objectContaining({ force: true, action: expect.objectContaining({ label: 'Undo' }) }));
    await elapse(UNDO_MS);
    expect(mocks.answer).toHaveBeenCalledWith('a', { answer: 'approve' });
  });

  it('Z restores the decision, refocuses it and never sends', async () => {
    render([decision('a'), decision('b')]);
    key('1');
    expect(current()).toBe('decision-b');
    key('z');
    expect(current()).toBe('decision-a');
    await elapse(UNDO_MS * 2);
    expect(mocks.answer).not.toHaveBeenCalled();
  });

  it("the toast's Undo does the same", async () => {
    render([decision('a'), decision('b')]);
    key('3');
    const opts = mocks.toast.mock.calls[0][3] as { action: { onAction: () => void } };
    act(() => opts.action.onAction());
    expect(current()).toBe('decision-a');
    await elapse(UNDO_MS);
    expect(mocks.answer).not.toHaveBeenCalled();
  });

  it('the last verdict shows the calm empty state, and undo still works there', async () => {
    render([decision('a')]);
    key('1');
    expect(document.body.textContent).toContain('Nothing needs you');
    key('z');
    expect(current()).toBe('decision-a');
  });

  it('Return mirrors the reversible primary, but not from a focused button', async () => {
    render([decision('a'), decision('b')]);
    key('Enter', {}, button('Next decision'));
    expect(current()).toBe('decision-a');
    key('Enter');
    expect(current()).toBe('decision-b');
  });

  it('a failed send puts the decision back and says so', async () => {
    mocks.answer.mockRejectedValueOnce(new Error('Offline'));
    render([decision('a'), decision('b')]);
    key('1');
    await elapse(UNDO_MS);
    expect(mocks.toast).toHaveBeenLastCalledWith("Couldn't send your answer", expect.stringContaining('Offline'), undefined, expect.objectContaining({ tone: 'danger' }));
    expect(current()).toBe('decision-a');
  });

  it('a 409 refreshes and brings the decision back rather than dropping it', async () => {
    mocks.answer.mockResolvedValueOnce({ ok: false, conflict: true });
    render([decision('held')]);
    key('1');
    await elapse(UNDO_MS);
    expect(mocks.refresh).toHaveBeenCalled();
    expect(mocks.toast).toHaveBeenLastCalledWith("That answer wasn't accepted", expect.any(String), undefined, expect.anything());
    expect(current()).toBe('decision-held');
  });

  it('closing the inbox sends what is still held (a verdict is never lost)', async () => {
    render([decision('a'), decision('b')]);
    key('1');
    await act(async () => root.unmount());
    expect(mocks.answer).toHaveBeenCalledWith('a', { answer: 'approve' });
    root = createRoot(container);
  });
});

describe('fields and irreversible actions', () => {
  it('Retry with notes opens a field whose Send and retry becomes the one primary', async () => {
    render([fixture('dec-004a-wallclock')]);
    key('2');
    const field = document.body.querySelector('textarea[id^="field-"]') as HTMLTextAreaElement;
    expect(document.activeElement).toBe(field);
    expect(document.querySelector('.pa-btn--primary')!.textContent).toContain('Send and retry');
    expect(button('Try again').className).not.toContain('pa-btn--primary');
    act(() => Simulate.change(field, { target: { value: 'Split it into two goals' } } as never));
    act(() => Simulate.keyDown(field, { key: 'Enter' }));
    await elapse(UNDO_MS);
    expect(mocks.answer).toHaveBeenCalledWith('dec-004a-wallclock', { answer: 'input', input_text: 'Split it into two goals' });
  });

  it('Escape in a field closes the field, not the inbox', () => {
    render([fixture('dec-001')]);
    key('3');
    const field = document.body.querySelector('textarea[id^="field-"]') as HTMLTextAreaElement;
    act(() => { field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })); });
    expect(document.body.querySelector('textarea[id^="field-"]')).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('Send back carries its optional reason as the note', async () => {
    render([fixture('dec-001')]);
    await click('Send back');
    const field = document.body.querySelector('textarea[id^="field-"]') as HTMLTextAreaElement;
    act(() => Simulate.change(field, { target: { value: 'Missing tests' } } as never));
    await click('Send back');
    await elapse(UNDO_MS);
    expect(mocks.answer).toHaveBeenCalledWith('dec-001', { answer: 'reject', note: 'Missing tests' });
  });

  it('Cancel goal… asks through ConfirmDialog, and Escape closes only the confirm', async () => {
    const d = fixture('dec-004a-wallclock');
    render([d]);
    await click('Cancel goal…');
    expect(document.body.textContent).toContain('Cancel this goal?');
    key('Escape', {}, document.body);
    expect(document.body.textContent).not.toContain('Cancel this goal?');
    expect(onClose).not.toHaveBeenCalled();
    await click('Cancel goal…');
    await click('Cancel goal');
    expect(mocks.cancelGoal).toHaveBeenCalledWith('proj-events', 'goal-event-analytics');
    expect(mocks.answer).not.toHaveBeenCalled();
  });

  it('a deletion has no keyboard default and confirms before sending', async () => {
    render([fixture('dec-004d-delete')]);
    key('Enter');
    key('1');
    expect(mocks.answer).not.toHaveBeenCalled();
    await click('Delete goal…');
    await click('Delete goal');
    expect(mocks.answer).toHaveBeenCalledWith('dec-004d-delete', { answer: 'approve' });
  });
});

describe('navigation and help keys (U14)', () => {
  it('J/K move, H defers, keys typed in a field are ignored', () => {
    render([decision('a'), decision('b')]);
    const ask = document.body.querySelector('textarea[aria-label="Ask about a"]')!;
    key('j', {}, ask);
    expect(current()).toBe('decision-a');
    // From a header control (where focus lands when the sheet opens) too.
    key('j', {}, button('Next decision'));
    expect(current()).toBe('decision-b');
    key('k');
    expect(current()).toBe('decision-a');
    key('h');
    expect(current()).toBe('decision-b');
    key('j');
    expect(current()).toBe('decision-a');
    expect(mocks.answer).not.toHaveBeenCalled();
  });

  it('/ focuses Ask Henry; ? toggles the shortcut sheet and Escape closes only the sheet', () => {
    render([decision('a')]);
    key('/');
    expect(document.activeElement?.getAttribute('aria-label')).toBe('Ask about a');
    key('?', { shiftKey: true });
    expect(document.body.querySelector('[role="dialog"][aria-label="Keyboard shortcuts"]')).not.toBeNull();
    key('Escape', {}, document.body);
    expect(document.body.querySelector('[role="dialog"][aria-label="Keyboard shortcuts"]')).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('buttons carry keycaps and aria-keyshortcuts', () => {
    render([fixture('dec-004a-wallclock')]);
    expect(button('Try again').getAttribute('aria-keyshortcuts')).toBe('1 Enter');
    expect(button('Try again').querySelectorAll('kbd').length).toBe(2);
    expect(button('Later').getAttribute('aria-keyshortcuts')).toBe('H');
    expect(button('Cancel goal…').querySelector('kbd')).toBeNull();
  });
});

describe('queue, evidence and reports', () => {
  it('loads all pending decisions and reaches beyond the first ten (ruling 4)', async () => {
    const rows = Array.from({ length: 12 }, (_, i) => decision(String(i).padStart(2, '0')));
    render(rows.slice(0, 10), 'center', { total_pending: 12 });
    expect(mocks.showAll).toHaveBeenCalledTimes(1);
    expect(document.body.querySelector('[data-testid="decision-loading"]')).not.toBeNull();
    expect(current()).toBeUndefined();
    render(rows);
    for (let i = 0; i < 11; i++) await click('Next decision');
    expect(current()).toBe('decision-11');
    expect(button('Next decision').disabled).toBe(true);
    expect(live()).toContain('Decision 12 of 12');
  });

  it('opens the exact Council session endpoint and returns (ruling 10)', async () => {
    mocks.apiFetch.mockResolvedValue({ report: { headline: 'Exact report', markdown: 'Session evidence' } });
    render([decision('c', { kind: 'council_action', payload: { session_id: 'session/older' } })]);
    await click('Open council report');
    expect(mocks.apiFetch).toHaveBeenCalledWith('/api/council/session%2Folder');
    expect(document.body.textContent).toContain('Exact report');
    expect(document.body.textContent).not.toContain('Council session');
    await click('← Back to decision');
    expect(current()).toBe('decision-c');
  });

  it('Open full evidence… opens its exact project and goal, suspending the trap (rulings 10, 11)', async () => {
    const rows = [decision('a'), decision('b', { project_id: 'p', goal_id: 'g' })];
    render(rows);
    await click('Next decision');
    await click('Open full evidence…');
    expect(mocks.openGoalDetail).toHaveBeenCalledWith('p', 'g');
    render(rows);
    const shell = document.body.querySelector('[role="dialog"]')!;
    expect(shell.getAttribute('aria-modal')).toBeNull();
    expect((shell.parentElement as HTMLElement).style.display).toBe('none');
    act(() => mocks.closeGoalDetail());
    render(rows);
    expect(current()).toBe('decision-b');
    expect(document.body.querySelector('[role="dialog"]')!.getAttribute('aria-modal')).toBe('true');
  });

  it('external resolution announces and advances using the previous ordering (ruling 5)', async () => {
    render([decision('a'), decision('b'), decision('c')]);
    await click('Next decision');
    render([decision('a'), decision('c')]);
    expect(current()).toBe('decision-c');
    expect(live()).toContain('Headline b is no longer pending.');
    expect(live()).toContain('Decision 2 of 2');
  });

  it('Later comes back when anything new arrives', () => {
    render([decision('a', { created_at: '2026-01-01' }), decision('b', { created_at: '2026-01-02' })]);
    key('h');
    expect(current()).toBe('decision-b');
    key('k');
    expect(current()).toBe('decision-b');
    render([decision('a', { created_at: '2026-01-01' }), decision('b', { created_at: '2026-01-02' }), decision('c', { created_at: '2026-01-03' })]);
    key('k');
    expect(current()).toBe('decision-a');
  });
});

describe('kind-specific content', () => {
  const check = decision('check', {
    kind: 'choice', project_id: 'proj-1', detail: '',
    payload: {
      options: [{ id: 'approve-once', label: 'Approve once' }, { id: 'approve-and-allowlist', label: 'Approve and allowlist "rm"' }, { id: 'deny', label: 'Deny' }],
      default: null, proposal: 'check_approval',
      check_approval: { command: 'rm -rf node_modules <script>', cwd: '/repo', first_token: 'rm', reason: 'Shell commands need your say-so.', tier: 'user', project_id: 'proj-1' },
    },
  });

  it('a command approval shows the exact command as plain text and its three options', () => {
    render([check]);
    const pre = document.body.querySelector('article pre')!;
    expect(pre.textContent).toBe('rm -rf node_modules <script>');
    expect(pre.querySelector('script')).toBeNull();
    expect(document.body.textContent).toContain('Command approval');
    expect(barLabels()).toEqual(['Approve once', 'Approve and allowlist "rm"', 'Deny']);
  });

  it('an enrichment Discard asks for a find-online hint and sends it as the note', async () => {
    render([decision('e', { kind: 'enrichment_proposal', payload: { person_name: 'Example Person' } })]);
    key('3');
    const hint = document.body.querySelector('textarea[id^="field-"]') as HTMLTextAreaElement;
    expect(hint.placeholder).toMatch(/find this person online/i);
    act(() => Simulate.change(hint, { target: { value: 'Halifax coworking' } } as never));
    act(() => Simulate.keyDown(hint, { key: 'Enter' }));
    await elapse(UNDO_MS);
    expect(mocks.answer).toHaveBeenCalledWith('e', { answer: 'reject', note: 'Halifax coworking' });
  });

  it('a voice-staged verdict says nothing happened yet, commits in one press with its words, and can be discarded', async () => {
    const staged = decision('s', { staged_answer: { answer: 'approve', note: 'go ahead', staged_at: new Date().toISOString(), staged_via: 'voice' } });
    render([staged]);
    const row = document.body.querySelector('[data-testid="staged-s"]')!;
    expect(row.textContent).toContain('Voice staged: Approve');
    expect(row.textContent).toContain('Nothing has happened yet');
    await click('Discard');
    expect(mocks.discard).toHaveBeenCalledWith('s');
    key('1');
    await elapse(UNDO_MS);
    expect(mocks.answer).toHaveBeenCalledWith('s', { answer: 'approve', note: 'go ahead' });
  });
});
