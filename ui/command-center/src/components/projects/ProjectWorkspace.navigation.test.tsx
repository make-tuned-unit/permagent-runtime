/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('./ProjectsView', () => ({ ProjectKanban: () => <p>Test Kanban</p> }));
vi.mock('./ProjectOverview', () => ({ ProjectOverview: () => <p>Test Overview</p> }));
vi.mock('./ProjectDetails', () => ({ ProjectDetails: () => <p>Test Details</p> }));
vi.mock('../../lib/api', () => ({ api: {}, apiFetch: vi.fn(), getApiBaseUrl: () => 'http://localhost:1234' }));
import { ProjectWorkspace } from './ProjectWorkspace';
import { requestProjectLens, useAppNavigationRequests } from '../../lib/appNavigationRequests';
import { useCommandCenter } from '../../lib/store';
import type { Project } from './types';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
const project = (id: string) => ({ id, name: id, status: 'active' } as Project);
function render(id: string) { act(() => root.render(<ProjectWorkspace project={project(id)} projects={[project(id)]} onSwitchProject={() => {}} onBack={() => {}} />)); }
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); useAppNavigationRequests.setState({ projectLens: null }); useCommandCenter.setState({ pendingCardNavigation: null, pendingNoteNavigation: null }); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
it('holds a request until its project mounts, then consumes it into the requested lens', () => {
  requestProjectLens('p2', 'details'); render('p1'); expect(host.textContent).toContain('Test Overview');
  expect(useAppNavigationRequests.getState().projectLens?.projectId).toBe('p2');
  render('p2'); expect(host.textContent).toContain('Test Details'); expect(useAppNavigationRequests.getState().projectLens).toBeNull();
});
it('an explicit lens request overrides old card/note deep links without modifying either artifact', () => {
  useCommandCenter.setState({ pendingCardNavigation: { projectId: 'p1', cardId: 'c1' }, pendingNoteNavigation: { projectId: 'p1', noteId: 'n1' } });
  requestProjectLens('p1', 'overview'); render('p1'); expect(host.textContent).toContain('Test Overview');
  expect(useCommandCenter.getState().pendingCardNavigation).toBeNull(); expect(useCommandCenter.getState().pendingNoteNavigation).toBeNull();
  act(() => { requestProjectLens('p1', 'kanban'); }); expect(host.textContent).toContain('Test Kanban');
});
