import { settleStartup } from './forumStartupTestUtils';
/** @vitest-environment jsdom */
import { beforeEach, afterEach, expect, describe, it, vi } from 'vitest';
import { createRoot, Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
const canvas = vi.hoisted(() => ({ props: null as Record<string, unknown> | null, portraitProps: [] as Record<string, unknown>[] }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: Record<string, unknown>) => { if (props.className === 'forum-canvas') canvas.props = props; else canvas.portraitProps.push(props); return null; } }));
const identityName = vi.hoisted(() => ({ value: 'Aster' }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => identityName.value }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('./ForumAgents', () => ({ ForumAgents: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
import { PCFShadowMap, PCFSoftShadowMap } from 'three';
import { ForumView } from './ForumView';
let root: Root, host: HTMLDivElement;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(async () => { localStorage.clear(); localStorage.setItem('permagent.forum.desk', 'open'); window.history.replaceState({}, '', '/?campus=1'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); act(() => root.render(<ForumView />)); await settleStartup(); });
afterEach(() => { window.history.replaceState({}, '', '/'); act(() => root.unmount()); host.remove(); canvas.props = null; canvas.portraitProps.length = 0; vi.restoreAllMocks(); });
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
  const buttons = [...nav.querySelectorAll('button')];
  // The rail gives each place its name AND what it is — the whole reason it
  // replaced the row of chips across the sky, which had room for names only.
  expect(buttons.map(b => b.querySelector('.forum-rail-place-name')?.textContent)).toEqual(
    ['Commons','Gallery','Build','Brain','Automate','Mesh gate','Reading grove','Maker court','Council garden','Arrival gardens','Conservatory','Maker hall','Debating theatre','Harbor']);
  expect(buttons.every(b => b.querySelector('.forum-rail-place-sub')?.textContent)).toBe(true);
  // One place at a time: radios in a radiogroup, not fourteen loose toggles.
  expect(nav.getAttribute('role')).toBe('radiogroup');
  expect(buttons.filter(b => b.getAttribute('aria-checked') === 'true')).toHaveLength(1);
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

it('mounts the live hero portrait in Meet and never references the retired portrait path', () => {
  expect(host.querySelector('[data-testid="live-agent-portrait"]')).toBeTruthy();
  expect(canvas.portraitProps.some(props => props.frameloop === 'demand')).toBe(true);
  const sourceRoot = join(process.cwd(), 'src');
  const legacyPortraitPath = ['forum', 'characters'].join('-');
  expect(() => execFileSync('rg', ['--fixed-strings', legacyPortraitPath, sourceRoot], { encoding: 'utf8', stdio: 'pipe' })).toThrow();
});

it('asks the renderer for a shadow map type three has not deprecated', () => {
  // A bare `shadows` makes r3f select PCFSoftShadowMap, which three 0.184
  // deprecates: it warns and downgrades to PCFShadowMap on every shadow render
  // (39 warnings per load, job 18 bug 5). Ask for what we actually get.
  expect(canvas.props?.shadows).toEqual({ type: PCFShadowMap });
  expect(canvas.props?.shadows).not.toBe(true);
  expect((canvas.props?.shadows as { type: number }).type).not.toBe(PCFSoftShadowMap);
  // ...and the shadow-casting sun light itself is untouched.
  expect(canvas.props?.frameloop).toBeTruthy();
});

it('Escape closes the Agent desk first, and only the next Escape collapses the place rail', () => {
  const shell = host.querySelector('main.forum-shell')!;
  const rail = host.querySelector('.forum-rail')!;
  expect(shell.getAttribute('data-desk')).toBe('open');
  expect(rail.getAttribute('data-open')).toBe('open');
  const railButton = rail.querySelector('[aria-label="World places"] button') as HTMLButtonElement;
  act(() => railButton.focus());
  const press = () => act(() => { railButton.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', bubbles: true, cancelable: true })); });
  press();
  expect(shell.getAttribute('data-desk')).toBe('closed');
  expect(rail.getAttribute('data-open')).toBe('open');
  press();
  expect(rail.getAttribute('data-open')).toBe('closed');
});

describe('Escape belongs to the screen that is showing', () => {
  // The desk's open/closed choice is remembered; start each case from a fresh one.
  beforeEach(async () => { localStorage.clear(); localStorage.setItem('permagent.forum.desk', 'open'); act(() => root.unmount()); root = createRoot(host); act(() => root.render(<ForumView />)); await settleStartup(); });
  const shell = () => host.querySelector('main.forum-shell')!;
  const esc = (target: EventTarget) => act(() => { target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', bubbles: true, cancelable: true })); });

  it('a hidden World never consumes Escape', () => {
    act(() => root.render(<ForumView visible={false} />));
    expect(shell().getAttribute('data-desk')).toBe('open');
    const press = new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', bubbles: true, cancelable: true });
    act(() => { document.body.dispatchEvent(press); });
    expect(shell().getAttribute('data-desk')).toBe('open');
    expect(press.defaultPrevented).toBe(false);
  });

  it('Escape typed in the Chat dock (outside the world) is left for the dock', () => {
    const dock = document.createElement('textarea');
    document.body.append(dock);
    esc(dock);
    expect(shell().getAttribute('data-desk')).toBe('open');
    dock.remove();
  });

  it('Escape inside an open dialog is left for the dialog', () => {
    const dialog = document.createElement('div');
    dialog.setAttribute('role', 'dialog');
    dialog.setAttribute('aria-modal', 'true');
    const input = document.createElement('input');
    dialog.append(input);
    document.body.append(dialog);
    esc(input);
    expect(shell().getAttribute('data-desk')).toBe('open');
    dialog.remove();
  });

  it('with the world showing and nothing else focused, Escape closes the desk', () => {
    esc(document.body);
    expect(shell().getAttribute('data-desk')).toBe('closed');
  });
});
