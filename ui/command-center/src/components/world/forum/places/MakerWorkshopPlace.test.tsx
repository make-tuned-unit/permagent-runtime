/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MakerWorkshopPlace } from './MakerWorkshopPlace';
import { runPlaceConformance } from './placeConformance';
import { click, fixtureById, fixtureLoader, harness, key, memoryStorage, mount, type, type Mounted } from './testHarness';
import { createDraftStore } from '../data/drafts';
import { FIXTURE_NOW, FIXTURE_PROJECT } from '../data/fixtures/worldFixtures';

runPlaceConformance('maker', MakerWorkshopPlace, ['ready', 'empty', 'error']);

describe('Maker workshop', () => {
  let mounted: Mounted | null = null;
  beforeEach(() => { localStorage.clear(); });
  afterEach(() => { mounted?.unmount(); mounted = null; });

  const q = (sel: string) => mounted!.host.querySelector<HTMLElement>(sel);
  const qa = (sel: string) => Array.from(mounted!.host.querySelectorAll<HTMLElement>(sel));

  it('tracked work: the primary action opens the real goal behind the selected work', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('active_request'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    const primary = q('[data-place-primary="true"]') as HTMLButtonElement;
    expect(primary.disabled).toBe(false);
    expect(q('[role="option"][aria-selected="true"] [data-stage]')!.getAttribute('data-stage')).toBe('tracked');
    click(primary);
    expect(h.navigation.openGoalDetail).toHaveBeenCalledWith(FIXTURE_PROJECT, 'fixture-goal-1');
    expect(q('[data-slot="result_shelf"]')!.getAttribute('data-results')).toBe('0');
    expect(q('[data-slot="result_shelf"]')!.textContent).toContain('A request reaching the server does not put anything on this shelf');
  });

  it('review needed: says so, and opening review never approves', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('decision_review'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    expect(q('[role="option"] [data-stage]')!.getAttribute('data-stage')).toBe('needs_review');
    expect(q('[data-action="review_required"]')!.textContent).toBe('Review required decision (1)');
    click(q('[data-action="review_required"]'));
    expect(h.navigation.openDecisionInbox).toHaveBeenCalledTimes(1);
    expect(h.navigation.openGoalDetail).not.toHaveBeenCalled();
  });

  it('review-record result: complete goal with a review record is not called Verified without an explicit verdict; spend names its scope', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('completed_goal'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    const shelf = q('[data-slot="result_shelf"]')!;
    expect(shelf.getAttribute('data-results')).toBe('1');
    expect(shelf.querySelector('[data-stage]')!.getAttribute('data-stage')).toBe('complete_review_record');
    expect(shelf.textContent).toContain('Complete; review record present');
    expect(shelf.textContent).not.toContain('Verified');
    click(Array.from(shelf.querySelectorAll('button')).find((b) => b.textContent === 'Open the result and its evidence')!);
    expect(h.navigation.openGoalDetail).toHaveBeenCalledWith(FIXTURE_PROJECT, 'fixture-goal-5');
    const spend = q('[data-slot="steward_spend"]')!;
    expect(spend.textContent).toContain('$0.42');
    expect(spend.textContent).toContain('Session cost is a separate scope');
    expect(q('[data-slot="skills"]')!.textContent).toContain('Check links');
    expect(q('[data-slot="skills"]')!.textContent).toContain('Saved, proposed and successfully exercised are different claims.');
  });

  it('failed and cancelled work is shown as such', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('failed_cancelled'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    const stages = qa('[role="option"] [data-stage]').map((e) => e.getAttribute('data-stage'));
    expect(stages).toEqual(['failed', 'cancelled']);
    expect(mounted.host.textContent).toContain('Failed');
    expect(mounted.host.textContent).toContain('Cancelled');
    expect(q('[data-slot="result_shelf"]')!.getAttribute('data-results')).toBe('0');
  });

  it('nothing attributed: the primary action says why it is unavailable', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('agents_no_work'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    expect((q('[data-place-primary="true"]') as HTMLButtonElement).disabled).toBe(true);
    expect(q('[data-disabled-reason="inspect_work"]')!.textContent).toContain('no goal is attributed to the Steward yet');
    expect(q('[data-slot="request_trail"]')!.textContent).toContain('Nothing prepared or sent from the World yet');
  });

  it('distinguishes a draft from a sent request in the request trail (from any place)', async () => {
    const storage = memoryStorage();
    const clock = () => new Date(FIXTURE_NOW).toISOString();
    const store = createDraftStore(storage, clock);
    store.update('commons', null, 'henry', { brief: 'Outline the launch page', acceptance: 'Five sections' });
    store.markSent('commons', null, 'henry', 'Sent to the orchestrator in the shared conversation (TEST DATA).');
    store.update('gallery', null, 'henry', { brief: 'Where did the two-second budget come from?' });
    const { loader, now } = await fixtureLoader(fixtureById('active_request'), 'maker');
    const h = harness({ loader, now }, storage);
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    const rows = qa('[data-slot="request_trail"] [data-stage]');
    const byStage = Object.fromEntries(rows.map((r) => [r.getAttribute('data-stage'), r.textContent]));
    expect(byStage.sent).toContain('Sent — acknowledged');
    expect(byStage.sent).toContain('Arrival table');
    expect(byStage.sent).toContain('Sent to the orchestrator in the shared conversation (TEST DATA).');
    expect(byStage.draft).toContain('Draft — not sent');
    expect(byStage.draft).toContain('Garden archive');
    expect(h.onRequestSend).not.toHaveBeenCalled();
  });

  it('keyboard: arrow keys choose another goal, and the primary action opens that one', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('failed_cancelled'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    const [first, second] = qa('[role="option"]');
    first.focus();
    key(first, 'ArrowDown');
    expect(document.activeElement).toBe(second);
    click(q('[data-place-primary="true"]'));
    expect(h.navigation.openGoalDetail).toHaveBeenCalledWith(FIXTURE_PROJECT, 'fixture-goal-6b');
  });

  it('a repair brief is a draft that survives leaving the World, and cites the selected goal', async () => {
    const storage = memoryStorage();
    const { loader, now } = await fixtureLoader(fixtureById('failed_cancelled'), 'maker');
    const h = harness({ loader, now }, storage);
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    click(q('[data-action="prepare_repair"]'));
    expect(q('[data-layer="composer"]')!.textContent).toContain(`goal:${FIXTURE_PROJECT}/fixture-goal-6a`);
    type(q('[data-layer="composer"] textarea'), 'Retry the publish with the fixed credentials');
    mounted.unmount();
    mounted = null;
    const again = harness({ loader, now }, storage);
    mounted = await mount(<MakerWorkshopPlace {...again.props} />);
    expect(q('[data-slot="request_trail"] [data-stage="draft"]')!.textContent).toContain('Retry the publish with the fixed credentials');
    click(q('[data-action="prepare_repair"]'));
    expect((q('[data-layer="composer"] textarea') as HTMLTextAreaElement).value).toBe('Retry the publish with the fixed credentials');
    expect(h.onRequestSend).not.toHaveBeenCalled();
    expect(again.onRequestSend).not.toHaveBeenCalled();
  });

  it('inspect a skill and open Build only navigate', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('completed_goal'), 'maker');
    const h = harness({ loader, now });
    mounted = await mount(<MakerWorkshopPlace {...h.props} />);
    click(q('[data-action="inspect_skill"]'));
    click(q('[data-action="open_build"]'));
    expect(h.navigation.openSkills).toHaveBeenCalledTimes(1);
    expect(h.navigation.openTool).toHaveBeenCalledWith('build');
    expect(vi.isMockFunction(h.onRequestSend) && h.onRequestSend.mock.calls.length).toBe(0);
  });
});
