import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
vi.mock('../lib/api', () => ({ api: {}, apiFetch: vi.fn(), extractText: vi.fn(() => ''), extractThinking: vi.fn(() => ''), fileToBase64: vi.fn(), readerIngest: vi.fn(), getApiBaseUrl: vi.fn(() => 'http://localhost:1234') }));
import { dispatchNavigate, dispatchOpenItem, routeGlobalFrame } from './useAppNavigate';
import { useCommandCenter } from '../lib/store';
import { HOME_CARD_TYPES, consumeNavigationRequest, PROJECT_LENSES, WORLD_PLACE_IDS, requestProjectLens, requestWorldPlace, useAppNavigationRequests } from '../lib/appNavigationRequests';
import { CARD_REGISTRY } from '../components/dashboard/cards/registry';
import { HABITAT_PLACES } from '../components/world/forum/habitatLayout';
beforeEach(() => {
  useAppNavigationRequests.setState({ projectLens: null, worldPlace: null, recordOpen: null, homeFocus: null });
  useCommandCenter.setState({ workspaces: ['projects', 'world', 'dashboard'].map(tool => ({ id: tool, layoutJson: { type: 'panel', tool } })) as never, switchWorkspace: vi.fn(), activePanel: 'chat', pendingProjectNavigation: null, pendingSettingsSection: null, pendingCardNavigation: null });
});
describe('allowlisted app navigation', () => {
  it.each(PROJECT_LENSES)('queues the %s lens for the resolved project, including before mount', section => {
    expect(dispatchNavigate({ tool_type: 'projects', section, state: { project_id: 'p1' } })).toBe(true);
    expect(useCommandCenter.getState().pendingProjectNavigation).toBe('p1');
    expect(useAppNavigationRequests.getState().projectLens).toEqual({ projectId: 'p1', lens: section });
    expect(useCommandCenter.getState().switchWorkspace).toHaveBeenCalledWith('projects');
  });
  it('rejects missing project ids and unknown lenses before switching', () => {
    expect(dispatchNavigate({ tool_type: 'projects', section: 'kanban' })).toBe(false);
    expect(dispatchNavigate({ tool_type: 'projects', section: 'delete', state: { project_id: 'p1' } })).toBe(false);
    expect(useCommandCenter.getState().switchWorkspace).not.toHaveBeenCalled();
  });
  it.each(WORLD_PLACE_IDS)('queues compact World destination %s', section => {
    expect(dispatchNavigate({ tool_type: 'world', section })).toBe(true);
    expect(useAppNavigationRequests.getState().worldPlace).toEqual({ placeId: section });
  });
  it('rejects unknown World places and unavailable workspaces without a pending request', () => {
    expect(dispatchNavigate({ tool_type: 'world', section: 'made-up' })).toBe(false);
    useCommandCenter.setState({ workspaces: [] });
    expect(dispatchNavigate({ tool_type: 'world', section: 'gallery' })).toBe(false);
    expect(useAppNavigationRequests.getState().worldPlace).toBeNull();
  });
  it('pins the World allowlist to the actual habitat and backend contract', () => {
    expect([...WORLD_PLACE_IDS].sort()).toEqual(HABITAT_PLACES.map(p => p.id).sort());
    const rust = readFileSync(new URL('../../../../crates/goose/src/agents/platform_extensions/app_conductor.rs', import.meta.url), 'utf8');
    const strings = (name: string) => [...rust.match(new RegExp(`const ${name}: [\\s\\S]*?= &\\[([\\s\\S]*?)\\];`))![1].matchAll(/"([^"]+)"/g)].map(m => m[1]);
    expect(strings('WORLD_SECTIONS')).toEqual([...WORLD_PLACE_IDS]);
    expect(strings('PROJECT_SECTIONS')).toEqual([...PROJECT_LENSES]);
  });
  it('routes Models and History segments through the real panel resolver', () => {
    dispatchNavigate({ tool_type: 'settings', panel_type: 'overlay', section: 'models' });
    expect(useCommandCenter.getState().activePanel).toBe('settings');
    expect(useCommandCenter.getState().pendingSettingsSection).toBe('models');
    for (const section of ['history', 'sessions', 'inbox', 'activity', 'spend']) {
      dispatchNavigate({ tool_type: 'settings', panel_type: 'overlay', section });
      expect(useCommandCenter.getState().activePanel).toBe('history');
      expect(useCommandCenter.getState().pendingSettingsSection).toBe(section);
    }
  });
  it('consumption cannot erase a newer pending request', () => {
    requestProjectLens('p1', 'details'); const old = useAppNavigationRequests.getState().projectLens;
    requestProjectLens('p2', 'kanban'); consumeNavigationRequest('projectLens', old);
    expect(useAppNavigationRequests.getState().projectLens?.projectId).toBe('p2');
    consumeNavigationRequest('projectLens', useAppNavigationRequests.getState().projectLens);
    expect(useAppNavigationRequests.getState().projectLens).toBeNull();
    expect(requestProjectLens(' ', 'details')).toBe(false); expect(requestWorldPlace('start-job')).toBe(false);
  });
  it('opens an actual card on Kanban without creating or executing one', () => {
    dispatchOpenItem({ kind: 'card', project_id: 'p7', card_id: 'c8' });
    expect(useCommandCenter.getState().pendingCardNavigation).toEqual({ projectId: 'p7', cardId: 'c8' });
    expect(useCommandCenter.getState().pendingProjectNavigation).toBe('p7');
  });
  it.each(['overview', ...HOME_CARD_TYPES])('queues Home section %s without editing its layout', section => {
    expect(dispatchNavigate({ tool_type: 'dashboard', section })).toBe(true);
    expect(useAppNavigationRequests.getState().homeFocus).toEqual({ section });
    expect(useCommandCenter.getState().switchWorkspace).toHaveBeenCalledWith('dashboard');
  });
  it('rejects unregistered Home sections before navigating', () => {
    expect(dispatchNavigate({ tool_type: 'dashboard', section: 'create-card' })).toBe(false);
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
    expect(useCommandCenter.getState().switchWorkspace).not.toHaveBeenCalled();
  });
  it('pins Home navigation to the union of real code cards and daemon built-in manifests', () => {
    const source = readFileSync(new URL('../../../../crates/goose-server/src/routes/dashboard_cards.rs', import.meta.url), 'utf8');
    const builtin = source.split('pub fn builtin_card_manifests()')[1].split('async fn get_card_types()')[0];
    const manifestTypes = [...builtin.matchAll(/card_type: "([^"]+)"/g)].map(m => m[1]);
    expect([...HOME_CARD_TYPES].sort()).toEqual([...new Set([...Object.keys(CARD_REGISTRY), ...manifestTypes])].sort());
    const conductor = readFileSync(new URL('../../../../crates/goose/src/agents/platform_extensions/app_conductor.rs', import.meta.url), 'utf8');
    const rustTypes = [...conductor.match(/const HOME_CARD_TYPES: [\s\S]*?= &\[([\s\S]*?)\];/)![1].matchAll(/"([^"]+)"/g)].map(m => m[1]);
    expect(rustTypes).toEqual([...HOME_CARD_TYPES]);
  });
  it('does not act on replayed Home focus requests', () => {
    routeGlobalFrame({ type: 'app_navigate', replayed: true, payload: { tool_type: 'dashboard', section: 'calendar' } }, Date.now(), { navigate: dispatchNavigate, launch: vi.fn(), theme: 'dark' });
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
  });
  it('opens Record only for a fresh open action; ignores replay and start', () => {
    const handlers = { navigate: dispatchNavigate, launch: vi.fn(), theme: 'dark' as const };
    const frame = { type: 'app_action', payload: { surface: 'record', action: 'open' } };
    routeGlobalFrame({ ...frame, replayed: true }, Date.now(), handlers);
    expect(useAppNavigationRequests.getState().recordOpen).toBeNull();
    routeGlobalFrame({ ...frame, payload: { surface: 'record', action: 'start' } }, Date.now(), handlers);
    expect(useAppNavigationRequests.getState().recordOpen).toBeNull();
    routeGlobalFrame(frame, Date.now(), handlers);
    expect(useAppNavigationRequests.getState().recordOpen).not.toBeNull();
  });
});
