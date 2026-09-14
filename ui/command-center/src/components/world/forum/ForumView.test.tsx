/** @vitest-environment jsdom */
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { createRoot, Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
vi.mock('@react-three/fiber', () => ({ Canvas: () => null }));
const identityName = vi.hoisted(() => ({ value: 'Aster' }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => identityName.value }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('./ForumAgents', () => ({ ForumAgents: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
import { ForumView } from './ForumView';
let root: Root, host: HTMLDivElement;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); act(() => root.render(<ForumView />)); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });
function click(label: string) { const b = [...host.querySelectorAll('button')].find(b => b.textContent?.includes(label)); expect(b).toBeTruthy(); act(() => b!.click()); }
it('turns a lesson into an editable draft without claiming dispatch', () => {
  expect(host.textContent).toContain('Job feed unavailable');
  click('How agents work'); click('Use this question');
  expect((host.querySelector('#forum-brief') as HTMLTextAreaElement).value).toContain('turn my request into a plan');
  expect(host.textContent).toContain('does not dispatch work');
  expect(host.textContent).toContain('Export keeps this as a draft');
});
it('blocks an empty brief and exports a nonempty draft with the requested agent', () => {
  click('Give a brief');
  expect([...host.querySelectorAll('button')].find(b => b.textContent?.includes('Export brief'))!.disabled).toBe(true);
  click('Learn'); click('Use this question');
  let blob: Blob | undefined;
  Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn(b => { blob = b; return 'blob:test'; }) });
  Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() });
  const download = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
  click('Export brief');
  expect(blob?.type).toBe('text/markdown'); expect(download).toHaveBeenCalledOnce();
  expect(host.textContent).toContain('No job was dispatched.');
});

it('uses existing product names and routes Brain to the memory tool only when a host supplies navigation', () => {
  const navigate = vi.fn();
  act(() => root.render(<ForumView onNavigate={navigate} />));
  click('Brain');
  expect((host.querySelector('#forum-agent') as HTMLSelectElement).value).toBe('librarian');
  expect(host.textContent).toContain('what your agents remember');
  click('Open Brain');
  expect(navigate).toHaveBeenCalledWith('memory');
});
it('exposes all districts as keyboard-accessible controls without a canvas', () => {
  const nav = host.querySelector('[aria-label="World places"]')!;
  expect([...nav.querySelectorAll('button')].map(b=>b.textContent)).toEqual(['Commons','Gallery','Build','Brain','Automate','Mesh gate','Reading grove','Maker court','Council garden','Arrival gardens','Conservatory','Maker hall','Debating theatre','Harbor']);
});

it('reaches the Mesh Agora without walking, and returns on Escape', () => {
  click('Mesh gate');
  expect(host.textContent).toContain('Walk through the ring to reach the Mesh Agora');
  expect(host.textContent).not.toContain('Return to the Forum');
  // Selecting the gate a second time walks through it.
  click('Mesh gate');
  expect(host.textContent).toContain('MESH AGORA');
  expect(host.textContent).toContain('Return to the Forum');
  act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { code: 'Escape' })); });
  expect(host.textContent).not.toContain('Return to the Forum');
  expect(host.textContent).toContain('MESH GATE');
});

it('opens the Agora from the panel affordance and closes it again', () => {
  click('Mesh gate');
  click('Enter the MESH');
  expect(host.textContent).toContain('MESH AGORA');
  click('Return to the Forum');
  expect(host.textContent).not.toContain('MESH AGORA');
});

it('identifies the user-named sovereign and finds it after selecting a specialist', () => {
  expect(host.querySelector('option[value="henry"]')?.textContent).toBe('Aster · Sovereign orchestrator');
  click('Brain');
  click('Find Aster');
  expect((host.querySelector('#forum-agent') as HTMLSelectElement).value).toBe('henry');
  expect(host.querySelector('.forum-agent-portrait h3')?.textContent).toBe('Aster');
  expect(host.textContent).toContain('Your sovereign orchestrator');
  identityName.value = 'Nova';
  act(() => root.render(<ForumView />));
  expect(host.querySelector('option[value="henry"]')?.textContent).toBe('Nova · Sovereign orchestrator');
  expect(host.querySelector('.forum-agent-portrait h3')?.textContent).toBe('Nova');
  identityName.value = 'Aster';
});
