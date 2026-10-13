/** @vitest-environment jsdom */
/**
 * World W2 contract gates that are about the code, not one place:
 * - The places import no store, API client or askFromForum, and call no fetch:
 *   the ONLY way to send is the host's `onRequestSend`, called in one place.
 * - No window/document listeners (Escape is handled on the panel itself).
 * - EvidenceSlot renders ready+stale and truncated honestly.
 * - The Places directory is the non-spatial equivalent.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { EvidenceSlot } from './EvidenceSlot';
import { PlaceDirectory } from './PlaceDirectory';
import { placeColors, placeStyles } from './placeStyles';
import { click, mount, type Mounted } from './testHarness';
import { placeSchema } from '../data/placeSchema';
import type { Section } from '../data/types';
import { FIXTURE_NOW } from '../data/fixtures/worldFixtures';
import { W2_PLACE_IDS } from './integration';

/** Code only: the docs name askFromForum to explain that the HOST calls it. */
function withoutComments(src: string): string {
  return src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^[ \t]*\/\/.*$/gm, '');
}

// jsdom gives import.meta.url an http scheme; vite-node provides __dirname.
const DIR = __dirname;
const appSources = readdirSync(DIR)
  .filter((f) => /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f) && f !== 'testHarness.tsx' && f !== 'placeConformance.tsx')
  .map((f) => ({ file: f, src: withoutComments(readFileSync(join(DIR, f), 'utf8')) }));

describe('World W2 places — code contract', () => {
  it('imports no store, API client, or askFromForum, and never calls fetch', () => {
    expect(appSources.length).toBeGreaterThan(8);
    for (const { file, src } of appSources) {
      expect(src, file).not.toMatch(/from ['"][./]*lib\/store['"]/);
      expect(src, file).not.toMatch(/from ['"][./]*lib\/api['"]/);
      expect(src, file).not.toMatch(/forumQuery|askFromForum\(/);
      expect(src, file).not.toMatch(/\bfetch\(|XMLHttpRequest|EventSource|WebSocket/);
    }
  });

  it('has exactly one call site of onRequestSend, inside the confirmation step', () => {
    const calls = appSources.flatMap(({ file, src }) => (src.match(/onRequestSend\(\{/g) ?? []).map(() => file));
    expect(calls).toEqual(['BriefComposer.tsx']);
    const composer = appSources.find((s) => s.file === 'BriefComposer.tsx')!.src;
    // The send function is reached only from the confirm button.
    expect(composer.match(/send\(\)/g)).toHaveLength(1);
    expect(composer).toMatch(/data-action="confirm_send" onClick=\{\(\) => \{ void send\(\); \}\}/);
  });

  it('writes a note only through saveFieldNoteToProject from the explicit save button', () => {
    const hits = appSources.flatMap(({ file, src }) => (src.match(/saveFieldNoteToProject\(/g) ?? []).map(() => file));
    expect(hits).toEqual(['FieldNoteEditor.tsx']);
    expect(appSources.some(({ src }) => /createProjectNote\(/.test(src))).toBe(false);
  });

  it('installs no global listeners', () => {
    for (const { file, src } of appSources) {
      expect(src, file).not.toMatch(/(window|document)\.addEventListener/);
    }
  });
});

describe('EvidenceSlot — honest values', () => {
  let mounted: Mounted | null = null;
  afterEach(() => { mounted?.unmount(); mounted = null; });
  const colors = placeColors();
  const styles = placeStyles(colors, false);
  const slot = placeSchema('commons').evidence[1];
  const meta = { contract: 'C5' as const, endpoint: '/api/goals/active', staleAfterMs: 60_000 };

  it('ready but old reads stale; truncated says it is not the complete list', async () => {
    const section: Section<string> = { state: 'ready', items: ['a', 'b'], truncated: true, bound: 2, ...meta, fetchedAt: new Date(FIXTURE_NOW - 5 * 60_000).toISOString() };
    mounted = await mount(<EvidenceSlot slot={slot} section={section} loading={false} now={FIXTURE_NOW} styles={styles} colors={colors} moreWhere="Projects" renderItems={(items) => <ul>{items.map((i) => <li key={i}>{i}</li>)}</ul>} />);
    const el = mounted.host.querySelector('[data-slot]')!;
    expect(el.getAttribute('data-slot-state')).toBe('ready');
    expect(el.getAttribute('data-stale')).toBe('true');
    expect(el.textContent).toContain('Stale');
    expect(el.textContent).toContain('Showing 2; more exist — the full list is in Projects. This is not the complete list.');
  });

  it('an error keeps last good items visible, marked stale, with a retry', async () => {
    const onRetry = vi.fn();
    const section: Section<string> = { state: 'error', failure: { cause: 'disconnected', message: 'x', status: null }, lastGood: { items: ['kept'], truncated: false, fetchedAt: new Date(FIXTURE_NOW - 120_000).toISOString() }, ...meta, fetchedAt: new Date(FIXTURE_NOW).toISOString() };
    mounted = await mount(<EvidenceSlot slot={slot} section={section} loading={false} now={FIXTURE_NOW} styles={styles} colors={colors} onRetry={onRetry} renderItems={(items, stale) => <ul data-rendered-stale={String(stale)}>{items.map((i) => <li key={i}>{i}</li>)}</ul>} />);
    const el = mounted.host.querySelector('[data-slot]')!;
    expect(el.textContent).toContain('kept');
    expect(el.querySelector('[data-rendered-stale]')!.getAttribute('data-rendered-stale')).toBe('true');
    expect(el.textContent).toContain('Last good data from 2 min ago');
    click(Array.from(el.querySelectorAll('button')).find((b) => b.textContent === 'Try again')!);
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});

describe('PlaceDirectory — the non-spatial equivalent', () => {
  let mounted: Mounted | null = null;
  afterEach(() => { mounted?.unmount(); mounted = null; });

  it('lists each place with its primary action and opens it without walking', async () => {
    const onOpenPlace = vi.fn();
    mounted = await mount(<PlaceDirectory onOpenPlace={onOpenPlace} />);
    const entries = Array.from(mounted.host.querySelectorAll('[data-place-entry]'));
    expect(entries.map((e) => e.getAttribute('data-place-entry'))).toEqual([...W2_PLACE_IDS]);
    expect(entries[0].textContent).toContain('Arrival table — prepare a brief');
    click(entries[1].querySelector('button'));
    expect(onOpenPlace).toHaveBeenCalledWith('gallery');
  });
});
