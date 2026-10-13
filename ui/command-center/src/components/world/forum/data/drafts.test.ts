import { describe, expect, it, vi } from 'vitest';
import {
  DRAFTS_STORAGE_KEY, DRAFT_BOUNDS, FIELD_NOTES_STORAGE_KEY, composeBriefRequest, createDraftStore, draftKey,
  parseDrafts, parseFieldNotes, saveFieldNoteToProject, serializeDrafts, worldStorage, type KeyValueStorage,
} from './drafts';
import { createApiEvidenceClient, type WorldNoteWriter } from './evidenceClient';
import { createWorldSnapshotLoader } from './snapshotLoader';
import { openRecordLink, recordLinkKey, type WorldNavigation } from './recordLinks';
import { PLACE_SCHEMAS } from './placeSchema';
import { WORLD_PLACE_IDS } from './identity';
import { FIXTURE_NOW, WORLD_FIXTURES, createFixtureClient } from './fixtures/worldFixtures';

function memStorage(): KeyValueStorage & { map: Map<string, string> } {
  const map = new Map<string, string>();
  return { map, getItem: (k) => map.get(k) ?? null, setItem: (k, v) => { map.set(k, v); } };
}
let tick = 0;
const clock = () => new Date(FIXTURE_NOW + (tick += 1000)).toISOString();

describe('drafts', () => {
  it('persists per place + project + agent under a versioned permagent.world.* key', () => {
    const storage = memStorage();
    const store = createDraftStore(storage, clock);
    store.update('commons', null, 'henry', { brief: 'Launch page', constraints: ['Under 2s on phone'], acceptance: 'Lighthouse ≥ 90' });
    store.update('commons', 'p1', 'henry', { brief: 'Other project' });
    expect(DRAFTS_STORAGE_KEY.startsWith('permagent.world.')).toBe(true);
    expect(JSON.parse(storage.map.get(DRAFTS_STORAGE_KEY)!).v).toBe(1);
    expect(store.restore('commons', null, 'henry')?.brief).toBe('Launch page');
    expect(store.restore('commons', 'p1', 'henry')?.brief).toBe('Other project');
    expect(store.restore('commons', null, 'steward')).toBeNull();
    // A fresh store over the same storage (World reopened) restores it unsent.
    const reopened = createDraftStore(storage, clock).restore('commons', null, 'henry')!;
    expect(reopened.sent).toBeNull();
    expect(reopened.constraints).toEqual(['Under 2s on phone']);
  });

  it('marks sent only after an acknowledged send, and editing starts a new unsent draft', () => {
    const store = createDraftStore(memStorage(), clock);
    expect(store.markSent('maker', null, 'steward', 'ack')).toBeNull();
    store.update('maker', null, 'steward', { brief: 'Fix footer' });
    expect(store.markSent('maker', null, 'steward', 'Goal fixture-goal-5 created')?.sent?.acknowledgement).toBe('Goal fixture-goal-5 created');
    expect(store.update('maker', null, 'steward', { brief: 'Fix footer again' }).sent).toBeNull();
  });

  it('replay: parsing the same stored payload twice is deep-equal, and serialize∘parse is stable', () => {
    const store = createDraftStore(memStorage(), clock);
    for (const p of WORLD_PLACE_IDS) store.update(p, null, 'henry', { brief: `brief ${p}`, evidenceRefs: [recordLinkKey({ to: 'goal', projectId: 'p', goalId: 'g' })] });
    const raw = serializeDrafts(store.all());
    expect(parseDrafts(raw)).toEqual(parseDrafts(raw));
    expect(serializeDrafts(parseDrafts(raw))).toBe(raw);
  });

  it('corrupt, future-version and empty payloads are reported, never thrown', () => {
    expect(parseDrafts(null).issue).toBe('empty');
    expect(parseDrafts('{not json').issue).toBe('corrupt');
    expect(parseDrafts(JSON.stringify({ drafts: {} })).issue).toBe('corrupt');
    expect(parseDrafts(JSON.stringify({ v: 99, drafts: {} }))).toEqual({ drafts: {}, issue: 'future_version' });
    expect(parseFieldNotes('[]').issue).toBe('corrupt');
    // Invalid entries are dropped, valid ones kept; unknown evidence refs are dropped.
    const raw = JSON.stringify({ v: 1, drafts: {
      a: { placeId: 'nowhere', agentWorldId: 'henry', updatedAt: 'x' },
      b: { placeId: 'gallery', projectId: null, agentWorldId: 'librarian', brief: 'ok', constraints: ['', 'c'], acceptance: 'a', evidenceRefs: ['bogus', 'skills:'], updatedAt: '2026-10-12T00:00:00Z', sent: null },
    } });
    const parsed = parseDrafts(raw);
    expect(Object.keys(parsed.drafts)).toEqual([draftKey('gallery', null, 'librarian')]);
    expect(parsed.drafts[draftKey('gallery', null, 'librarian')]).toMatchObject({ constraints: ['c'], evidenceRefs: ['skills:'] });
  });

  it('bounds user text and the number of stored drafts', () => {
    const storage = memStorage();
    const store = createDraftStore(storage, clock);
    const d = store.update('commons', null, 'henry', { brief: 'x'.repeat(DRAFT_BOUNDS.briefChars + 10), constraints: Array.from({ length: 20 }, (_, i) => `c${i}`) });
    expect(d.brief.length).toBe(DRAFT_BOUNDS.briefChars);
    expect(d.constraints).toHaveLength(DRAFT_BOUNDS.constraints);
    for (let i = 0; i < DRAFT_BOUNDS.drafts + 5; i += 1) store.update('maker', `p${i}`, 'steward', { brief: `b${i}` });
    const all = store.all().drafts;
    expect(Object.keys(all)).toHaveLength(DRAFT_BOUNDS.drafts);
    // The least recently edited were dropped.
    expect(all[draftKey('commons', null, 'henry')]).toBeUndefined();
    expect(all[draftKey('maker', `p${DRAFT_BOUNDS.drafts + 4}`, 'steward')]).toBeDefined();
  });

  it('falls back to memory when localStorage throws', () => {
    const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('SecurityError'); } });
    try {
      const s = worldStorage();
      expect(s.persistent).toBe(false);
      s.setItem('k', 'v');
      expect(s.getItem('k')).toBe('v');
    } finally {
      if (original) Object.defineProperty(globalThis, 'localStorage', original);
      else delete (globalThis as { localStorage?: unknown }).localStorage;
    }
  });

  it('composes request text without sending', () => {
    const r = composeBriefRequest({ brief: ' Launch page ', constraints: ['Fast', ' '], acceptance: ' Lighthouse ≥ 90 ', evidenceRefs: ['goal:p/g'] });
    expect(r.question).toBe('Launch page\n\nConstraints:\n- Fast\n\nEvidence I selected (World record references):\n- goal:p/g');
    expect(r.criteria).toBe('Lighthouse ≥ 90');
  });
});

describe('field notes', () => {
  it('stay local until an explicit save to a chosen project; saving twice writes once', async () => {
    const storage = memStorage();
    const store = createDraftStore(storage, clock);
    const writer: WorldNoteWriter = { createProjectNote: vi.fn(async (projectId, n) => ({ id: 'fixture-note-1', project_id: projectId, title: n.title ?? null, body: n.body, memory_key: null, created_at: '', updated_at: '' })) };
    const note = store.addNote('council_garden', 'Sign-ups lag visits by a week', ['project:p1'], 'local-1');
    expect(FIELD_NOTES_STORAGE_KEY.startsWith('permagent.world.')).toBe(true);
    expect(note.saved).toBeNull();
    expect(writer.createProjectNote).not.toHaveBeenCalled();
    expect(await saveFieldNoteToProject(store, writer, note, '')).toEqual({ ok: false, reason: 'Choose a project to save this note to.' });
    expect(writer.createProjectNote).not.toHaveBeenCalled();
    const saved = await saveFieldNoteToProject(store, writer, note, 'p1');
    expect(saved.ok).toBe(true);
    expect(writer.createProjectNote).toHaveBeenCalledTimes(1);
    const again = store.notes().notes[0];
    expect(again.saved).toMatchObject({ projectId: 'p1', noteId: 'fixture-note-1' });
    await saveFieldNoteToProject(store, writer, again, 'p1');
    expect(writer.createProjectNote).toHaveBeenCalledTimes(1);
  });

  it('a failed save keeps the note and says so', async () => {
    const store = createDraftStore(memStorage(), clock);
    const note = store.addNote('gallery', 'keep me', [], 'local-2');
    const r = await saveFieldNoteToProject(store, { createProjectNote: async () => { throw new TypeError('Failed to fetch'); } }, note, 'p1');
    expect(r).toEqual({ ok: false, reason: 'Not saved: Failed to fetch. Your note is still here.' });
    expect(store.notes().notes[0].saved).toBeNull();
  });
});

describe('no send on selection', () => {
  it('selecting every place in every fixture issues only GET reads and never writes', async () => {
    const methods: string[] = [];
    const writer: WorldNoteWriter = { createProjectNote: vi.fn() };
    for (const f of WORLD_FIXTURES) {
      const fixtureClient = createFixtureClient(() => f.phases[0]);
      // Route the fixture through the REAL api client so request methods are observable.
      const fetcher = async <T,>(endpoint: string, init?: RequestInit): Promise<T> => {
        methods.push(`${(init?.method ?? 'GET').toUpperCase()} ${endpoint}`);
        const path = endpoint.split('?')[0];
        const signal = init?.signal ?? new AbortController().signal;
        if (path === '/api/henry/status') return fixtureClient.henryStatus(signal) as Promise<T>;
        if (path === '/api/goals/active') return fixtureClient.activeGoals(signal) as Promise<T>;
        if (path === '/api/decisions') return fixtureClient.decisions(signal) as Promise<T>;
        if (path === '/api/brain/memories') return fixtureClient.brainMemories(8, signal) as Promise<T>;
        if (path === '/permagent/skills') return fixtureClient.skills(signal) as Promise<T>;
        if (path === '/api/council/reports') return fixtureClient.councilReports(3, signal) as Promise<T>;
        if (path === '/api/growth-results') return fixtureClient.growthResults(new URLSearchParams(endpoint.split('?')[1]).get('projectId'), signal) as Promise<T>;
        const agent = path.match(/^\/api\/agents\/([^/]+)(\/work)?$/);
        if (agent) return (agent[2] ? fixtureClient.agentWork(decodeURIComponent(agent[1]), 10, signal) : fixtureClient.agentDetail(decodeURIComponent(agent[1]), signal)) as Promise<T>;
        const project = path.match(/^\/api\/projects\/([^/]+)\/(memories|notes|market)$/);
        if (project) {
          const id = decodeURIComponent(project[1]);
          return (project[2] === 'memories' ? fixtureClient.projectMemories(id, signal) : project[2] === 'notes' ? fixtureClient.projectNotes(id, signal) : fixtureClient.projectMarket(id, signal)) as Promise<T>;
        }
        throw new Error(`unexpected ${endpoint}`);
      };
      const loader = createWorldSnapshotLoader(createApiEvidenceClient(fetcher), { now: () => FIXTURE_NOW });
      const drafts = createDraftStore(memStorage(), clock);
      const nav: WorldNavigation = {
        focusBrainMemory: () => true, openGoalDetail: () => true, openDecisionInbox: () => true, openConversation: () => true,
        openHistory: () => true, openGrow: () => true, openTool: () => true, openProject: () => true, focusProjectNote: () => true,
        openSkills: () => true, openAgentSettings: () => true, openHome: () => true,
      };
      for (const schema of PLACE_SCHEMAS) {
        // What W2 does on selection: load evidence, restore the draft, open a record link.
        const snap = await loader.loadPlace(schema.id, { projectId: f.projectId });
        drafts.restore(schema.id, f.projectId, schema.host);
        for (const section of Object.values(snap.sections)) {
          if (section?.state === 'ready') {
            const link = (section.items[0] as { link?: Parameters<typeof openRecordLink>[0] | null }).link;
            if (link) openRecordLink(link, nav);
          }
        }
        await loader.loadAgent(schema.host);
      }
    }
    expect(methods.length).toBeGreaterThan(0);
    expect(methods.every((m) => m.startsWith('GET '))).toBe(true);
    expect(writer.createProjectNote).not.toHaveBeenCalled();
  });

  it('only explicit actions are marked as sending; no primary action sends', () => {
    for (const p of PLACE_SCHEMAS) {
      expect(p.primaryAction.sends).toBe(false);
      for (const a of [p.primaryAction, ...p.secondaryActions]) {
        expect(a.sends).toBe(a.kind === 'explicit_send' || a.kind === 'explicit_save');
      }
    }
  });
});
