/**
 * TEST SUPPORT — the gates every W2 place must pass, run by each place's test
 * file: one discoverable primary action, every W1 state value rendered, the
 * fixture states, no send on any selection, Escape / focus return, and DOM
 * snapshots at 1280×800 and 1024×640 logical for every fixture state.
 */
import type { ComponentType } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import { placeSchema } from '../data/placeSchema';
import type { WorldPlaceId } from '../data/identity';
import type { PlaceSections, Section } from '../data/types';
import { WORLD_FIXTURES, FIXTURE_NOW } from '../data/fixtures/worldFixtures';
import type { PlaceHostProps } from './integration';
import { click, fixtureLoader, flush, harness, key, mount, nonSendingButtons, SIZES, stableHtml, stubLoader, type Mounted } from './testHarness';

const META = { contract: 'C5' as const, endpoint: '/api/test', fetchedAt: new Date(FIXTURE_NOW).toISOString(), staleAfterMs: 60_000 };

/** One section per W1 state value, for slots whose item shape does not matter when empty. */
export const STATE_SECTIONS: Record<string, Section<never>> = {
  empty: { state: 'empty', copy: 'TEST: nothing in this scope (not proof of inactivity).', ...META },
  error_disconnected: { state: 'error', failure: { cause: 'disconnected', message: 'Failed to fetch', status: null }, lastGood: null, ...META },
  error_last_empty: { state: 'error', failure: { cause: 'server_error', message: 'boom', status: 500 }, lastGood: { items: [], truncated: false, fetchedAt: META.fetchedAt }, ...META },
  unsupported: { state: 'unsupported', reason: 'TEST: this agent has no daemon source.' },
  setup_required: { state: 'setup_required', reason: 'TEST: choose a project first.', action: 'Choose a project' },
};

export function runPlaceConformance(placeId: WorldPlaceId, Place: ComponentType<PlaceHostProps>, expectFixtureStates: readonly string[]) {
  const schema = placeSchema(placeId);
  let mounted: Mounted | null = null;
  let fetchSpy: Mock;
  const realFetch = globalThis.fetch;

  beforeEach(() => {
    localStorage.clear();
    fetchSpy = vi.fn(() => Promise.reject(new Error('network is not allowed in place tests')));
    globalThis.fetch = fetchSpy as never;
  });
  afterEach(() => {
    mounted?.unmount();
    mounted = null;
    globalThis.fetch = realFetch;
  });

  describe(`${schema.name} — conformance`, () => {
    it('has exactly one discoverable primary action, named by the schema, focused on open', async () => {
      const { loader, now } = await fixtureLoader(WORLD_FIXTURES.find((f) => f.id === 'agents_no_work')!, placeId);
      const h = harness({ loader, now });
      mounted = await mount(<Place {...h.props} />);
      const primaries = mounted.host.querySelectorAll('[data-place-primary="true"]');
      expect(primaries).toHaveLength(1);
      const primary = primaries[0] as HTMLButtonElement;
      expect(primary.tagName).toBe('BUTTON');
      expect(primary.textContent).toContain(schema.primaryAction.label);
      expect(primary.getAttribute('data-action')).toBe(schema.primaryAction.id);
      const hint = document.getElementById(primary.getAttribute('aria-describedby')!);
      expect(hint?.textContent).toContain(schema.primaryAction.description);
      // Discoverable: it precedes every other action and every evidence slot in DOM order.
      const all = Array.from(mounted.host.querySelectorAll('button, [data-slot]'));
      const firstOther = all.find((el) => el !== primary && !el.getAttribute('aria-label')?.startsWith('Close '));
      expect(all.indexOf(primary)).toBeLessThan(all.indexOf(firstOther!));
      if (!primary.disabled) expect(document.activeElement).toBe(primary);
      else expect(document.activeElement?.tagName).toBe('H2');
      // The dialog names the place and the host.
      const dialog = mounted.host.querySelector('[role="dialog"]')!;
      expect(dialog.getAttribute('data-world-place')).toBe(placeId);
      expect(dialog.textContent).toContain(schema.name);
      expect(dialog.textContent).toMatch(/Host: /);
    });

    it('renders every W1 section state value for every evidence slot', async () => {
      for (const [name, section] of Object.entries(STATE_SECTIONS)) {
        const sections = Object.fromEntries(schema.evidence.map((s) => [s.id, section])) as PlaceSections;
        const h = harness({ loader: stubLoader(sections), now: () => FIXTURE_NOW });
        mounted = await mount(<Place {...h.props} />);
        for (const s of schema.evidence) {
          const el = mounted.host.querySelector(`[data-slot="${s.id}"]`);
          expect(el, `${name}: slot ${s.id}`).not.toBeNull();
          expect(el!.getAttribute('data-slot-state')).toBe(section.state);
          if (section.state === 'empty') expect(el!.textContent).toContain('TEST: nothing in this scope');
          if (section.state === 'unsupported') expect(el!.textContent).toContain('TEST: this agent has no daemon source.');
          if (section.state === 'setup_required') expect(el!.textContent).toContain('TEST: choose a project first.');
          if (section.state === 'error') {
            expect(el!.getAttribute('data-stale')).toBe('true');
            expect(el!.textContent).toMatch(section.failure.cause === 'disconnected' ? /lost connection, not a failed task/ : /answered with an error/);
            expect(el!.textContent).toMatch(section.lastGood ? /Last known answer .*: nothing/ : /unknown, not empty/);
          }
        }
        mounted.unmount();
        mounted = null;
      }
    });

    it('shows a loading value before evidence arrives — never an empty claim', async () => {
      const h = harness({ loader: { ...stubLoader({}), loadPlace: () => new Promise(() => {}) }, now: () => FIXTURE_NOW });
      mounted = await mount(<Place {...h.props} />);
      const slots = Array.from(mounted.host.querySelectorAll('[data-slot]')).filter((el) => schema.evidence.some((s) => s.id === el.getAttribute('data-slot')));
      expect(slots.length).toBe(schema.evidence.length);
      for (const el of slots) expect(el.getAttribute('data-slot-state')).toBe('loading');
      expect(mounted.host.querySelector('[data-state-tone="loading"]')?.textContent).toContain('Reading evidence');
    });

    it(`renders the fixture states it can produce (${expectFixtureStates.join(', ')})`, async () => {
      const seen = new Set<string>();
      let staleSeen = false;
      for (const fixture of WORLD_FIXTURES) {
        const { loader, now } = await fixtureLoader(fixture, placeId);
        const h = harness({ loader, now, projectId: fixture.projectId });
        mounted = await mount(<Place {...h.props} />);
        for (const el of Array.from(mounted.host.querySelectorAll('[data-slot-state]'))) {
          seen.add(el.getAttribute('data-slot-state')!);
          if (el.getAttribute('data-stale') === 'true') staleSeen = true;
        }
        if (fixture.id === 'disconnect_replay') {
          // Lost connection is shown as such, with the last good data marked stale.
          expect(mounted.host.querySelector('[role="alert"][data-state-tone="error"]')?.textContent).toMatch(/lost connection, not a failed task/);
          expect(mounted.host.querySelector('[data-slot-state="error"][data-stale="true"]')).not.toBeNull();
        }
        mounted.unmount();
        mounted = null;
      }
      for (const s of expectFixtureStates) expect(seen, `state ${s}`).toContain(s);
      expect(staleSeen).toBe(true);
    });

    it('never sends on selection: every non-send control, every record, every key — zero sends, zero writes, zero network', async () => {
      for (const fixture of WORLD_FIXTURES) {
        const { loader, now, calls } = await fixtureLoader(fixture, placeId);
        const h = harness({ loader, now, projectId: fixture.projectId });
        mounted = await mount(<Place {...h.props} />);
        // Keyboard over every record option.
        for (const option of Array.from(mounted.host.querySelectorAll('[role="option"]'))) {
          key(option, 'ArrowDown'); key(option, 'End'); key(option, 'Home'); key(option, 'Enter');
          click(option);
        }
        // Click every non-sending control, repeatedly, as layers open and close.
        for (let pass = 0; pass < 3; pass += 1) {
          for (const button of nonSendingButtons(mounted.host)) {
            if (!button.isConnected || button.getAttribute('aria-label')?.startsWith('Close ')) continue;
            click(button);
            await flush();
          }
        }
        expect(h.onRequestSend, fixture.id).not.toHaveBeenCalled();
        expect(h.createProjectNote, fixture.id).not.toHaveBeenCalled();
        expect(fetchSpy, fixture.id).not.toHaveBeenCalled();
        // Every daemon call was a read through the W1 client (named reads only).
        for (const c of calls) expect(c).toMatch(/^(henryStatus|activeGoals|decisions|brainMemories|projectMemories|projectNotes|agentDetail|agentWork|skills|projectMarket|councilReports|growthResults)/);
        // No navigation answers anything: only openers exist on the nav object.
        const events = h.onEvent.mock.calls.map((c) => (c[0] as { type: string }).type);
        expect(events).not.toContain('send_requested');
        expect(events).not.toContain('note_saved');
        mounted.unmount();
        mounted = null;
      }
    });

    it('Escape closes the panel (no inner layer open) and focus returns to the trigger; Escape does not leak to the window', async () => {
      const { loader, now } = await fixtureLoader(WORLD_FIXTURES.find((f) => f.id === 'active_request')!, placeId);
      const trigger = document.createElement('button');
      trigger.textContent = 'Open place';
      document.body.appendChild(trigger);
      trigger.focus();
      const h = harness({ loader, now });
      const leaked = vi.fn();
      window.addEventListener('keydown', leaked);
      mounted = await mount(<Place {...h.props} />);
      expect(mounted.host.contains(document.activeElement)).toBe(true);
      key(document.activeElement, 'Escape');
      expect(h.onClose).toHaveBeenCalledTimes(1);
      expect(leaked).not.toHaveBeenCalled();
      // The host unmounts on close; focus goes back to what opened the panel.
      mounted.unmount();
      mounted = null;
      expect(document.activeElement).toBe(trigger);
      window.removeEventListener('keydown', leaked);
      trigger.remove();
    });

    it('respects reduced motion: no movement on entrance, opacity only', async () => {
      localStorage.setItem('permagent-reduce-motion', 'true');
      const h = harness({ loader: stubLoader({}), now: () => FIXTURE_NOW });
      mounted = await mount(<Place {...h.props} />);
      const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]')!;
      expect(dialog.getAttribute('data-reduced-motion')).toBe('true');
      expect(dialog.style.transform).toBe('');
      expect(dialog.style.transition).toMatch(/^opacity 160ms/);
      mounted.unmount();
      mounted = null;
      localStorage.setItem('permagent-reduce-motion', 'false');
      mounted = await mount(<Place {...h.props} />);
      const moving = mounted.host.querySelector<HTMLElement>('[role="dialog"]')!;
      expect(moving.getAttribute('data-reduced-motion')).toBe('false');
      expect(moving.style.transition).toMatch(/opacity 320ms .*transform 320ms/);
    });

    it('lays out for 1280×800 (two columns) and 1024×640 (one column, secondary evidence collapsed)', async () => {
      const fixture = WORLD_FIXTURES.find((f) => f.id === 'active_request')!;
      for (const size of SIZES) {
        const { loader, now } = await fixtureLoader(fixture, placeId);
        const h = harness({ loader, now, viewport: { width: size.width, height: size.height } });
        mounted = await mount(<Place {...h.props} />, size);
        const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]')!;
        const grid = Array.from(dialog.querySelectorAll<HTMLElement>('div')).find((d) => d.style.display === 'grid' && d.style.gridTemplateColumns.includes('minmax'))!;
        const collapsed = dialog.querySelectorAll('details[data-slot]').length;
        if (size.height <= 720) {
          expect(grid.style.gridTemplateColumns).toBe('minmax(0, 1fr)');
          expect(collapsed).toBeGreaterThan(0);
        } else {
          expect(grid.style.gridTemplateColumns).toBe('repeat(2, minmax(0, 1fr))');
        }
        // The primary action and the first evidence slot are never collapsed.
        expect(dialog.querySelector('[data-place-primary="true"]')!.closest('details')).toBeNull();
        expect(dialog.querySelector(`[data-slot="${schema.evidence[0].id}"]`)!.tagName).toBe('SECTION');
        mounted.unmount();
        mounted = null;
      }
    });

    describe('DOM snapshots per fixture state', () => {
      for (const size of SIZES) {
        for (const fixture of WORLD_FIXTURES) {
          it(`${size.name} · ${fixture.item}-${fixture.id}`, async () => {
            const { loader, now } = await fixtureLoader(fixture, placeId);
            const h = harness({ loader, now, projectId: fixture.projectId, viewport: { width: size.width, height: size.height } });
            mounted = await mount(<Place {...h.props} />, size);
            const dialog = mounted.host.querySelector('[role="dialog"]')!;
            expect(dialog.getAttribute('data-compact')).toBe(size.height <= 720 ? 'true' : 'false');
            expect(stableHtml(dialog)).toMatchSnapshot();
          });
        }
      }
    });
  });
}
