/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const reads = vi.hoisted(() => ({ getProviders: vi.fn(), getConfig: vi.fn(), getLibrarianSchedule: vi.fn(), getOllamaStatus: vi.fn(), readSecretConfig: vi.fn() }));
vi.mock('../../lib/api', () => ({ api: reads }));
import { StepModel } from './StepModel';
import { StepReview } from './StepReview';
import { setTheme } from '../../styles/tokens';
const providers = [{ name: 'ollama', provider_type: 'Builtin', is_configured: false, is_default: false,
  metadata: { name: 'ollama', display_name: 'Ollama', description: '', default_model: 'local', known_models: [], config_keys: [] } }];
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement; let root: Root;
let finish: (value: typeof providers) => void; let fail: (reason: Error) => void;
const done = vi.fn();
beforeEach(() => {
  localStorage.clear(); localStorage.setItem('permagent-reduce-motion', 'true'); done.mockClear();
  reads.getProviders.mockReturnValue(new Promise((resolve, reject) => { finish = resolve; fail = reject; }));
  reads.getConfig.mockResolvedValue({ config: {} });
  reads.getLibrarianSchedule.mockResolvedValue({ enabled: false, model: '' });
  reads.getOllamaStatus.mockResolvedValue({ reachable: false, installed: [], running: [] });
  reads.readSecretConfig.mockResolvedValue(null);
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); act(() => setTheme('dark')); });
async function render(screen: 'model' | 'review') { await act(async () => root.render(screen === 'model'
  ? <StepModel initialChoice={null} onDone={done} />
  : <StepReview agentName="Aster" toneLabel="Balanced" firstMessage="" onFirstMessage={() => {}} onEdit={() => {}}
    steps={{ model: 1, agent: 2, folders: 3, extras: 4 }} onStart={done} saving={false} />)); }
function radio(value: string) { return host.querySelector<HTMLInputElement>(`input[value="${value}"]`)!; }
function primary() { return host.querySelector<HTMLButtonElement>('button[type="submit"]')!; }
it('reserves all three model choices before capabilities resolve without replacing or widening cards', async () => {
  await render('model');
  const cards = [...host.querySelectorAll('[data-choice-selected]')];
  expect(cards).toHaveLength(3); expect(radio('local').disabled).toBe(true);
  const grid = cards[0].parentElement!; const geometry = grid.getAttribute('style');
  const footer = host.querySelector('[data-testid="wizard-action-footer"]')!;
  await act(async () => finish(providers));
  expect([...host.querySelectorAll('[data-choice-selected]')]).toEqual(cards);
  expect(grid.getAttribute('style')).toBe(geometry); expect(radio('local').disabled).toBe(false);
  expect(host.querySelector('[data-testid="wizard-action-footer"]')).toBe(footer);
  expect(host.textContent).not.toContain('Best quality'); expect(host.textContent).toContain("provider's servers");
});
it('keeps the local slot and the honest no-model path when local is unavailable or checking fails', async () => {
  await render('model'); await act(async () => finish([]));
  expect(host.querySelectorAll('input[type="radio"]')).toHaveLength(3);
  expect(radio('local').disabled).toBe(true); expect(host.textContent).toContain('Not offered by this service');
  act(() => radio('later').click()); expect(primary().textContent).toBe('Continue without a model');
  act(() => primary().click()); expect(done).toHaveBeenCalledWith('later');
});
it('uses a neutral offset keyboard ring independently of accent selection and makes keyboard feedback immediate', async () => {
  await render('model'); const hosted = radio('hosted'); const card = hosted.closest('label')!;
  expect(document.activeElement).toBe(hosted); expect(hosted.checked).toBe(false);
  expect(card.dataset.choiceFocus).toBe('true'); expect(card.dataset.choiceSelected).toBe('false');
  const neutralRing = card.style.boxShadow;
  expect(neutralRing).not.toBe('none'); expect(card.style.transition).toBe('none');
  act(() => hosted.click());
  expect(hosted.checked).toBe(true); expect(card.dataset.choiceSelected).toBe('true');
  expect(card.style.boxShadow).toBe(neutralRing);
});
it.each(['ready', 'failed'] as const)('keeps the review action and reserved status region stationary when checks are %s', async outcome => {
  await render('review');
  const action = primary(); const footer = action.closest('[data-testid="wizard-action-footer"]')!;
  const footerStyle = footer.getAttribute('style'); const status = host.querySelector('[data-testid="review-check-status"]')!;
  const statusStyle = status.getAttribute('style');
  expect(footer.parentElement).toBe(host.querySelector('form'));
  expect(action.disabled).toBe(false);
  await act(async () => { if (outcome === 'ready') finish(providers); else fail(new Error('Unavailable')); });
  expect(primary()).toBe(action); expect(footer.getAttribute('style')).toBe(footerStyle);
  expect(status.getAttribute('style')).toBe(statusStyle); expect(statusStyle).toContain('min-height: 72px');
  expect(action.style.minWidth).toBe('220px'); act(() => action.click()); expect(done).toHaveBeenCalledOnce();
});
for (const [width, height] of [[1280, 800], [1024, 640]]) for (const theme of ['dark', 'silver'] as const) for (const screen of ['model', 'review'] as const) for (const state of ['loading', 'ready', 'failed'] as const) {
  it(`DOM Wizard ${screen} ${state} ${width}x${height} ${theme}`, async () => {
    Object.defineProperties(window, { innerWidth: { configurable: true, value: width }, innerHeight: { configurable: true, value: height } });
    host.style.width = `${width}px`; host.style.height = `${height}px`; act(() => setTheme(theme));
    await render(screen);
    if (state !== 'loading') await act(async () => { if (state === 'ready') finish(providers); else fail(new Error('Provider list unavailable')); });
    expect(host).toMatchSnapshot();
  });
}
