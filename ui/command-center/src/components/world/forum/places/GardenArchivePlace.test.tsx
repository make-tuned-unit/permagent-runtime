/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { GardenArchivePlace } from './GardenArchivePlace';
import { runPlaceConformance } from './placeConformance';
import { click, fixtureById, fixtureLoader, flush, harness, key, memoryStorage, mount, type, type Mounted } from './testHarness';
import { FIXTURE_PROJECT } from '../data/fixtures/worldFixtures';

runPlaceConformance('gallery', GardenArchivePlace, ['ready', 'empty', 'error', 'setup_required']);

describe('Garden archive', () => {
  let mounted: Mounted | null = null;
  beforeEach(() => { localStorage.clear(); });
  afterEach(() => { mounted?.unmount(); mounted = null; });

  const q = (sel: string) => mounted!.host.querySelector<HTMLElement>(sel);
  const options = () => Array.from(mounted!.host.querySelectorAll<HTMLButtonElement>('[role="option"]'));
  const button = (text: string) => Array.from(mounted!.host.querySelectorAll<HTMLButtonElement>('button')).find((b) => b.textContent === text)!;

  it('browse focuses the real memory folio; arrow keys move between records; selecting is local', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    click(q('[data-place-primary="true"]'));
    const [first, second] = options();
    expect(document.activeElement).toBe(first);
    expect(first.getAttribute('tabindex')).toBe('0');
    expect(second.getAttribute('tabindex')).toBe('-1');
    key(first, 'ArrowDown');
    expect(document.activeElement).toBe(second);
    expect(second.getAttribute('aria-selected')).toBe('true');
    key(second, 'Home');
    expect(document.activeElement).toBe(first);
    expect(first.getAttribute('aria-selected')).toBe('true');
    expect(h.onRequestSend).not.toHaveBeenCalled();
    expect(h.createProjectNote).not.toHaveBeenCalled();
    expect(h.onEvent.mock.calls.filter((c) => c[0].type === 'record_selected')).toHaveLength(2);
  });

  it('shows source provenance for one record and an explicit provenance gap for the other', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    expect(q('[data-inspect]')!.getAttribute('data-inspect')).toBe('none');
    click(options()[0]);
    expect(q('[data-inspect]')!.textContent).toContain('The launch page must load in under two seconds on a phone.');
    expect(q('[data-provenance]')!.getAttribute('data-provenance')).toBe('source_tag');
    expect(q('[data-provenance]')!.textContent).toContain('Source tag: fixture-meeting-notes.md');
    expect(q('[data-provenance]')!.textContent).toContain('not necessarily an openable document');
    click(options()[1]);
    expect(q('[data-provenance]')!.getAttribute('data-provenance')).toBe('none');
    expect(q('[data-provenance]')!.textContent).toContain('No provenance recorded. Treat this as stored context, not sourced evidence.');
  });

  it('opens the record in Brain through navigation only, and reports when Brain is unreachable', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    expect(q('[data-action="open_in_brain"]')!.hasAttribute('disabled')).toBe(true);
    click(options()[0]);
    click(q('[data-action="open_in_brain"]'));
    expect(h.navigation.focusBrainMemory).toHaveBeenCalledWith(expect.objectContaining({ id: 'fixture-mem-1', key: 'note:fixture-1' }));
    h.navigation.focusBrainMemory.mockReturnValue(false);
    click(button('Open in Brain'));
    expect(q('[data-link-failed]')!.textContent).toContain('No workspace currently hosts Brain');
  });

  it('saves a field note only to an explicitly chosen project, through the note writer', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    click(options()[0]);
    const note = q('[data-field-note] textarea') as HTMLTextAreaElement;
    type(note, 'Two seconds is the budget; the hero image is the risk.');
    const save = q('[data-action="save_field_note"]') as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(q('[data-field-note]')!.textContent).toContain('Choose a project to save to.');
    expect(h.createProjectNote).not.toHaveBeenCalled();
    type(q('[data-field-note] select'), FIXTURE_PROJECT);
    expect(save.disabled).toBe(false);
    click(save);
    await flush();
    expect(h.createProjectNote).toHaveBeenCalledTimes(1);
    expect(h.createProjectNote.mock.calls[0][0]).toBe(FIXTURE_PROJECT);
    expect(h.createProjectNote.mock.calls[0][1].body).toContain('Two seconds is the budget');
    expect(h.createProjectNote.mock.calls[0][1].body).toContain('brain:fixture-mem-1/note%3Afixture-1');
    expect(q('[data-field-note] [role="status"]')!.textContent).toBe('Saved to Fixture garden project.');
    expect(q('[data-note][data-saved="true"]')!.textContent).toContain('Saved to Fixture garden project');
    click(button('Open note'));
    expect(h.navigation.focusProjectNote).toHaveBeenCalledWith(FIXTURE_PROJECT, 'fixture-saved-note-1');
    expect(h.onRequestSend).not.toHaveBeenCalled();
  });

  it('keeps an unsaved field note across leaving and returning, and never saves it on its own', async () => {
    const storage = memoryStorage();
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now }, storage);
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    click(options()[1]);
    type(q('[data-field-note] textarea'), 'Probably from the kickoff call — check.');
    mounted.unmount();
    mounted = null;
    const again = harness({ loader, now }, storage);
    mounted = await mount(<GardenArchivePlace {...again.props} />);
    click(options()[1]);
    expect((q('[data-field-note] textarea') as HTMLTextAreaElement).value).toBe('Probably from the kickoff call — check.');
    expect(q('[data-note][data-saved="false"]')!.textContent).toContain('On this device only');
    expect(h.createProjectNote).not.toHaveBeenCalled();
    expect(again.createProjectNote).not.toHaveBeenCalled();
  });

  it('ask about this evidence prepares a question citing the record; Escape returns focus to the trigger', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    click(options()[0]);
    const ask = q('[data-action="ask_about_evidence"]')!;
    click(ask);
    expect(q('[data-layer="composer"]')!.textContent).toContain('brain:fixture-mem-1/note%3Afixture-1');
    expect(document.activeElement?.tagName).toBe('TEXTAREA');
    key(document.activeElement, 'Escape');
    expect(q('[data-layer="composer"]')).toBeNull();
    expect(document.activeElement).toBe(ask);
    expect(h.onClose).not.toHaveBeenCalled();
    expect(h.onRequestSend).not.toHaveBeenCalled();
  });

  it('choosing a project reads that project’s memories and notes; no project means notes need setup', async () => {
    const { loader, now, calls } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    expect(q('[data-slot="notes"]')!.getAttribute('data-slot-state')).toBe('setup_required');
    const chooser = mounted.host.querySelector('[aria-label="Folio controls"] select')!;
    type(chooser, FIXTURE_PROJECT);
    await flush();
    expect(calls).toContain(`projectMemories:${FIXTURE_PROJECT}`);
    expect(calls).toContain(`projectNotes:${FIXTURE_PROJECT}`);
    expect(q('[data-slot="notes"]')!.getAttribute('data-slot-state')).toBe('empty');
    expect(h.onEvent.mock.calls.map((c) => c[0])).toContainEqual({ type: 'project_changed', placeId: 'gallery', projectId: FIXTURE_PROJECT });
  });

  it('the filter narrows only what is loaded, and says so', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('memory_provenance'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    type(q('input[type="search"]'), 'headlines');
    expect(options()).toHaveLength(1);
    expect(mounted.host.textContent).toContain('1 of 2 loaded record(s) match. This filters what is loaded here; Brain searches everything.');
  });

  it('the example is a labelled demonstration and a local event', async () => {
    const { loader, now } = await fixtureLoader(fixtureById('fresh_account'), 'gallery');
    const h = harness({ loader, now });
    mounted = await mount(<GardenArchivePlace {...h.props} />);
    expect(q('[data-action="try_example"]')!.textContent).toBe('Try an example (demonstration)');
    click(q('[data-action="try_example"]'));
    expect(h.onEvent.mock.calls.map((c) => c[0])).toContainEqual({ type: 'local_action', placeId: 'gallery', action: 'try_example' });
    expect(q('[data-place-empty="true"]')!.textContent).toContain('The archive is empty');
  });
});
