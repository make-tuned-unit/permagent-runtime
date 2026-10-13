/**
 * TEST SUPPORT — World W2 place tests. Not imported by any app code.
 *
 * Builds a place's host props over W1's TEST DATA fixtures: a real snapshot
 * loader over the fixture client (reads only), an in-memory draft store, and
 * spies for every outward seam (navigation, onRequestSend, the note writer).
 */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import type { ReactElement } from 'react';
import { vi, type Mock } from 'vitest';
import { STALE_AFTER_MS } from '../data/bounds';
import { createDraftStore, type KeyValueStorage } from '../data/drafts';
import type { WorldNoteWriter } from '../data/evidenceClient';
import type { WorldPlaceId } from '../data/identity';
import type { WorldNavigation } from '../data/recordLinks';
import { createWorldSnapshotLoader, type PlaceSnapshot, type WorldSnapshotLoader } from '../data/snapshotLoader';
import type { PlaceSections } from '../data/types';
import { createFixtureClient, FIXTURE_NOW, FIXTURE_PROJECT, WORLD_FIXTURES, type WorldFixture } from '../data/fixtures/worldFixtures';
import type { PlaceHostProps, PlaceSendAck } from './integration';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

export const SIZES = [
  { name: '1280x800', width: 1280, height: 800 },
  { name: '1024x640', width: 1024, height: 640 },
] as const;

const PHASE_STEP_MS = Math.max(...Object.values(STALE_AFTER_MS)) + 1_000;

export function memoryStorage(seed: Record<string, string> = {}): KeyValueStorage & { data: Map<string, string> } {
  const data = new Map(Object.entries(seed));
  return { data, getItem: (k) => data.get(k) ?? null, setItem: (k, v) => { data.set(k, v); } };
}

export function fixtureById(id: string): WorldFixture {
  const f = WORLD_FIXTURES.find((x) => x.id === id);
  if (!f) throw new Error(`no fixture ${id}`);
  return f;
}

/**
 * A loader positioned at the fixture's display phase: earlier phases are
 * loaded for `placeId` first (so a disconnect shows last good data), then
 * the clock moves past every staleness threshold and the cache is invalidated.
 */
export async function fixtureLoader(fixture: WorldFixture, placeId: WorldPlaceId) {
  let phase = 0;
  let clock = FIXTURE_NOW;
  const calls: string[] = [];
  const client = createFixtureClient(() => fixture.phases[phase], calls);
  const loader = createWorldSnapshotLoader(client, { now: () => clock });
  for (; phase < fixture.displayPhase; phase += 1) {
    await loader.loadPlace(placeId, { projectId: fixture.projectId });
    clock += PHASE_STEP_MS;
    loader.invalidate(['/']);
  }
  phase = fixture.displayPhase;
  return { loader, now: () => clock, calls };
}

/** A loader that returns the given sections for any place (for state coverage). */
export function stubLoader(sections: PlaceSections): WorldSnapshotLoader {
  return {
    loadPlace: async (placeId, context): Promise<PlaceSnapshot> => ({ placeId, context, sections, requestCount: 0, current: true }),
    loadAgent: async () => { throw new Error('not used'); },
    invalidate: () => {},
    invalidateForWorldEvent: () => [],
    cancel: () => {},
    totalRequests: 0,
  };
}

export function spyNavigation(): WorldNavigation & Record<keyof WorldNavigation, Mock> {
  const ok = () => vi.fn(() => true);
  return {
    focusBrainMemory: ok(), openGoalDetail: ok(), openDecisionInbox: ok(), openConversation: ok(), openHistory: ok(),
    openGrow: ok(), openTool: ok(), openProject: ok(), focusProjectNote: ok(), openSkills: ok(), openAgentSettings: ok(), openHome: ok(),
  } as never;
}

export interface Harness {
  props: PlaceHostProps;
  onRequestSend: Mock;
  createProjectNote: Mock;
  onClose: Mock;
  onEvent: Mock;
  navigation: ReturnType<typeof spyNavigation>;
  storage: ReturnType<typeof memoryStorage>;
}

export function harness(over: Partial<PlaceHostProps> & { loader: WorldSnapshotLoader; now: () => number }, storage = memoryStorage()): Harness {
  const ack: PlaceSendAck = { acknowledgement: 'Sent to the orchestrator in the shared conversation (TEST DATA).', conversation: { sessionId: 'fixture-session-1' }, goal: null };
  const onRequestSend = vi.fn(async () => ack);
  const createProjectNote = vi.fn(async (projectId: string, note: { title?: string; body: string }) => ({
    id: 'fixture-saved-note-1', project_id: projectId, title: note.title ?? null, body: note.body, memory_key: null,
    created_at: new Date(over.now()).toISOString(), updated_at: new Date(over.now()).toISOString(),
  }));
  const noteWriter: WorldNoteWriter = { createProjectNote };
  const onClose = vi.fn();
  const onEvent = vi.fn();
  const navigation = spyNavigation();
  let seq = 0;
  const props: PlaceHostProps = {
    navigation,
    drafts: createDraftStore(storage, () => new Date(over.now()).toISOString()),
    noteWriter,
    projectId: null,
    projects: [{ id: FIXTURE_PROJECT, name: 'Fixture garden project' }],
    selectedAgentId: 'henry',
    onRequestSend,
    onClose,
    onEvent,
    agentNames: { henry: 'Orchestrator' },
    viewport: { width: 1280, height: 800 },
    newId: () => `fixture-local-note-${(seq += 1)}`,
    ...over,
  };
  return { props, onRequestSend, createProjectNote, onClose, onEvent, navigation, storage };
}

export async function flush() {
  await act(async () => {
    for (let i = 0; i < 12; i += 1) await Promise.resolve();
  });
}

export interface Mounted {
  host: HTMLDivElement;
  root: Root;
  render(el: ReactElement): Promise<void>;
  unmount(): void;
}

/** Mount inside a frame of the given logical size (jsdom does not lay out). */
export async function mount(el: ReactElement, size: { width: number; height: number } = SIZES[0]): Promise<Mounted> {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: size.width });
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: size.height });
  const host = document.createElement('div');
  host.style.width = `${size.width}px`;
  host.style.height = `${size.height}px`;
  document.body.appendChild(host);
  const root = createRoot(host);
  const m: Mounted = {
    host,
    root,
    async render(next) { act(() => root.render(next)); await flush(); },
    unmount() { act(() => root.unmount()); host.remove(); },
  };
  await m.render(el);
  return m;
}

export function click(el: Element | null) {
  if (!el) throw new Error('no element to click');
  act(() => { (el as HTMLElement).click(); });
}

export function key(el: Element | null, k: string) {
  if (!el) throw new Error('no element for key');
  act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); });
}

/** Set a controlled input's value the way React listens for it. */
export function type(el: Element | null, value: string) {
  if (!el) throw new Error('no field');
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
  });
}

/**
 * Stable DOM for snapshots: React's useId values depend on test order, and
 * inline token styles are asserted separately (layout tests), so the snapshot
 * records structure, text and state attributes.
 */
export function stableHtml(el: Element): string {
  return el.outerHTML.replace(/:r[0-9a-z]+:/g, ':id:').replace(/ style="[^"]*"/g, '');
}

/** Every button whose activation is a selection, navigation, local action or layer toggle — never the explicit send / save. */
export function nonSendingButtons(root: Element): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>('button')).filter((b) => {
    const a = b.getAttribute('data-action');
    return !b.disabled && a !== 'confirm_send' && a !== 'save_field_note';
  });
}
