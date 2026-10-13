/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import { ArrivalTablePlace } from './ArrivalTablePlace';
import { runPlaceConformance } from './placeConformance';
import { click, fixtureById, fixtureLoader, flush, harness, key, memoryStorage, mount, type, type Mounted } from './testHarness';
import { draftKey } from '../data/drafts';

runPlaceConformance('commons', ArrivalTablePlace, ['ready', 'empty', 'error']);

describe('Arrival table', () => {
  let mounted: Mounted | null = null;
  beforeEach(() => { localStorage.clear(); });
  afterEach(() => { mounted?.unmount(); mounted = null; });

  const q = (sel: string) => mounted!.host.querySelector<HTMLElement>(sel);
  const button = (text: string | RegExp) => Array.from(mounted!.host.querySelectorAll<HTMLButtonElement>('button')).find((b) => (typeof text === 'string' ? b.textContent === text : text.test(b.textContent ?? '')))!;
  const fields = () => Array.from(mounted!.host.querySelectorAll<HTMLTextAreaElement>('[data-layer="composer"] textarea'));

  it('identifies the orchestrator and its four-fact state with evidence', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('active_request'), 'commons');
    const h = harness({ loader, now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    const facts = q('[data-orchestrator-facts]')!;
    expect(facts.getAttribute('data-orchestrator-facts')).toBe('read');
    const value = (id: string) => facts.querySelector(`[data-fact="${id}"]`)!.getAttribute('data-value');
    expect(value('configured')).toBe('unknown');
    expect(value('reachable')).toBe('reachable');
    expect(value('ready')).toBe('unknown');
    expect(value('activelyUsed')).toBe('active_now');
    expect(facts.textContent).toContain('does not report model configuration');
    expect(facts.textContent).toContain('A reachable daemon is not proof a model can answer.');
    expect(q('[role="dialog"]')!.textContent).toContain('Host: Orchestrator');
  });

  it('says the orchestrator was not read when the status route is lost, keeping last good facts stale', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('disconnect_replay'), 'commons');
    const h = harness({ loader, now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    expect(q('[data-orchestrator-facts]')!.getAttribute('data-orchestrator-facts')).toBe('stale');
    expect(q('[data-orchestrator-facts]')!.textContent).toContain('(stale)');
  });

  it('offers "Continue with…" only for a real session, and opening it only navigates', async () => {
    const active = await fixtureLoader(fixtureById('active_request'), 'commons');
    const h = harness({ loader: active.loader, now: active.now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    click(button(/Continue with “Plan the garden launch page”/));
    expect(h.navigation.openConversation).toHaveBeenCalledWith('fixture-session-1');
    expect(h.onRequestSend).not.toHaveBeenCalled();
    mounted.unmount();

    const idle = await fixtureLoader(fixtureById('agents_no_work'), 'commons');
    const h2 = harness({ loader: idle.loader, now: idle.now });
    mounted = await mount(<ArrivalTablePlace {...h2.props} />);
    expect(mounted.host.textContent).not.toMatch(/Continue with “/);
    expect(q('[data-action="resume_conversation"]')!.hasAttribute('disabled')).toBe(true);
    expect(q('[data-disabled-reason="resume_conversation"]')!.textContent).toContain('no conversation is open');
    expect(q('[data-place-empty="true"]')!.textContent).toContain('Nothing is in progress yet');
  });

  it('shows failed and cancelled requests truthfully', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('failed_cancelled'), 'commons');
    const h = harness({ loader, now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    expect(q('[data-outcome="failed"]')!.textContent).toContain('Failed');
    expect(q('[data-outcome="cancelled"]')!.textContent).toContain('Cancelled');
  });

  it('prepares a brief with a constraint and acceptance; Escape peels confirm → composer → panel with focus return', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('agents_no_work'), 'commons');
    const h = harness({ loader, now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    const primary = q('[data-place-primary="true"]')!;
    click(primary);
    expect(primary.getAttribute('aria-expanded')).toBe('true');
    const [brief, constraints, acceptance] = fields();
    expect(document.activeElement).toBe(brief);
    type(brief, 'Outline the launch page');
    type(constraints, 'Keep the current URL\nNo new dependencies');
    type(acceptance, 'An outline with five sections');
    expect(q('[data-draft-stage]')!.getAttribute('data-draft-stage')).toBe('draft');

    const review = q('[data-action="review_send"]')!;
    click(review);
    const preview = q('[data-testid="send-preview"]')!;
    expect(preview.textContent).toContain('Outline the launch page');
    expect(preview.textContent).toContain('- Keep the current URL');
    expect(preview.textContent).toContain('Useful answer criteria: An outline with five sections');
    expect(document.activeElement?.tagName).toBe('H4');

    key(document.activeElement, 'Escape');
    expect(q('[data-layer="confirm-send"]')).toBeNull();
    expect(q('[data-layer="composer"]')).not.toBeNull();
    expect(document.activeElement).toBe(review);

    key(document.activeElement, 'Escape');
    expect(q('[data-layer="composer"]')).toBeNull();
    expect(document.activeElement).toBe(primary);
    expect(h.onClose).not.toHaveBeenCalled();

    key(document.activeElement, 'Escape');
    expect(h.onClose).toHaveBeenCalledTimes(1);
    expect(h.onRequestSend).not.toHaveBeenCalled();
  });

  it('keeps the draft across unmount and remount (leaving World), without sending it', async () => {
    const storage = memoryStorage();
    const { loader, now } = await fixtureLoader(fixtureById('agents_no_work'), 'commons');
    const h = harness({ loader, now, projectId: 'fixture-project-garden' }, storage);
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    click(q('[data-place-primary="true"]'));
    const [brief, constraints, acceptance] = fields();
    type(brief, 'Draft the pricing FAQ');
    type(constraints, 'Two paragraphs at most');
    type(acceptance, 'Answers the three top questions');
    mounted.unmount();
    mounted = null;
    expect(storage.data.get('permagent.world.drafts')).toContain('Draft the pricing FAQ');

    const again = harness({ loader, now, projectId: 'fixture-project-garden' }, storage);
    mounted = await mount(<ArrivalTablePlace {...again.props} />);
    expect(q('[data-layer="composer"]')).toBeNull();
    click(q('[data-place-primary="true"]'));
    const [b2, c2, a2] = fields();
    expect(b2.value).toBe('Draft the pricing FAQ');
    expect(c2.value).toBe('Two paragraphs at most');
    expect(a2.value).toBe('Answers the three top questions');
    expect(q('[data-draft-stage]')!.getAttribute('data-draft-stage')).toBe('draft');
    expect(h.onRequestSend).not.toHaveBeenCalled();
    expect(again.onRequestSend).not.toHaveBeenCalled();
    // A different project is a different draft.
    mounted.unmount();
    const other = harness({ loader, now, projectId: null }, storage);
    mounted = await mount(<ArrivalTablePlace {...other.props} />);
    click(q('[data-place-primary="true"]'));
    expect(fields()[0].value).toBe('');
  });

  it('sends only from the confirmation button, through onRequestSend, and shows the actual acknowledgement', async () => {
    const storage = memoryStorage();
    const { loader, now } = await fixtureLoader(fixtureById('agents_no_work'), 'commons');
    const h = harness({ loader, now, selectedAgentId: 'librarian' }, storage);
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    click(q('[data-place-primary="true"]'));
    const [brief, constraints, acceptance] = fields();
    type(brief, 'Summarise what we know about the launch');
    type(constraints, 'Cite the stored records');
    type(acceptance, 'Three bullet points with sources');
    click(q('[data-action="review_send"]'));
    expect(h.onRequestSend).not.toHaveBeenCalled();
    click(q('[data-action="confirm_send"]'));
    await flush();
    expect(h.onRequestSend).toHaveBeenCalledTimes(1);
    expect(h.onRequestSend.mock.calls[0][0]).toMatchObject({
      placeId: 'commons', projectId: null, agentWorldId: 'librarian', agentName: 'The Librarian', intent: 'job',
      criteria: 'Three bullet points with sources', draftKey: draftKey('commons', null, 'librarian'),
    });
    expect(h.onRequestSend.mock.calls[0][0].question).toContain('Constraints:\n- Cite the stored records');
    const ack = q('[data-send-ack="true"]')!;
    expect(ack.getAttribute('role')).toBe('status');
    expect(ack.textContent).toContain('Sent to the orchestrator in the shared conversation (TEST DATA).');
    expect(ack.textContent).toContain('No tracked goal is linked yet');
    click(button('Open the conversation'));
    expect(h.navigation.openConversation).toHaveBeenCalledWith('fixture-session-1');
    expect(q('[data-draft-stage]')!.getAttribute('data-draft-stage')).toBe('sent');
    expect(h.onEvent.mock.calls.map((c) => c[0].type)).toEqual(expect.arrayContaining(['send_requested', 'send_acknowledged']));

    // Relaunch: the sent state is remembered; nothing is re-sent.
    mounted.unmount();
    const again = harness({ loader, now, selectedAgentId: 'librarian' }, storage);
    mounted = await mount(<ArrivalTablePlace {...again.props} />);
    click(q('[data-place-primary="true"]'));
    expect(q('[data-draft-stage]')!.textContent).toMatch(/^Sent .* Sent to the orchestrator/);
    expect(again.onRequestSend).not.toHaveBeenCalled();
  });

  it('a failed send keeps the draft and says so', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('agents_no_work'), 'commons');
    const h = harness({ loader, now, onRequestSend: vi.fn(async () => { throw new Error('Wait for the current reply, or stop it first.'); }) });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    click(q('[data-place-primary="true"]'));
    type(fields()[0], 'Rename the launch folder');
    click(q('[data-action="review_send"]'));
    click(q('[data-action="confirm_send"]'));
    await flush();
    expect(q('[data-layer="confirm-send"] [role="alert"]')!.textContent).toContain('Not sent: Wait for the current reply, or stop it first. Your draft is still here.');
    expect(q('[data-draft-stage]')!.getAttribute('data-draft-stage')).toBe('draft');
    expect(fields()[0].value).toBe('Rename the launch folder');
  });

  it('sit together, open profile and explore places are local events, not sends', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('agents_no_work'), 'commons');
    const h = harness({ loader, now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    click(q('[data-action="sit_together"]'));
    click(q('[data-action="open_profile"]'));
    click(q('[data-action="explore_places"]'));
    const local = h.onEvent.mock.calls.map((c) => c[0]).filter((e) => e.type === 'local_action').map((e) => e.action);
    expect(local).toEqual(['sit_together', 'open_profile', 'explore_places']);
    expect(h.onRequestSend).not.toHaveBeenCalled();
  });

  it('review a decision opens the inbox and never answers it', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('decision_review'), 'commons');
    const h = harness({ loader, now });
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    expect(q('[data-action="review_decision"]')!.textContent).toBe('Review a decision (1)');
    click(q('[data-action="review_decision"]'));
    expect(h.navigation.openDecisionInbox).toHaveBeenCalledTimes(1);
    const otherNav = Object.entries(h.navigation).filter(([k, fn]) => k !== 'openDecisionInbox' && (fn as Mock).mock.calls.length);
    expect(otherNav).toEqual([]);
  });

  it('an unreachable destination is reported, not silently ignored', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('decision_review'), 'commons');
    const h = harness({ loader, now });
    h.navigation.openDecisionInbox.mockReturnValue(false);
    mounted = await mount(<ArrivalTablePlace {...h.props} />);
    click(q('[data-action="review_decision"]'));
    expect(q('[data-link-failed]')!.textContent).toContain('No workspace currently hosts Home');
  });
});
