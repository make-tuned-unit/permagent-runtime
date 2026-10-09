// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const reads = vi.hoisted(() => ({
  getConfig: vi.fn(), getProviders: vi.fn(), getLibrarianSchedule: vi.fn(), readSecretConfig: vi.fn(), getOllamaStatus: vi.fn(),
}));
vi.mock('../../lib/api', () => ({ api: reads }));
vi.mock('../../styles/useTheme', async () => {
  const { getThemedColors } = await import('../../styles/tokens');
  return { useTheme: () => ({ colors: getThemedColors(), reduceMotion: true, theme: 'dark' }) };
});
import { StepReview } from './StepReview';
import { SEARCH_PROVIDERS } from '../../lib/searchProviders';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: Root;
const start = vi.fn();
beforeEach(() => {
  Object.values(reads).forEach(read => read.mockReset());
  start.mockReset();
  reads.getConfig.mockResolvedValue({ config: { GOOSE_PROVIDER: 'ollama', GOOSE_MODEL: 'qa:local', dev_roots: [] } });
  reads.getProviders.mockResolvedValue([{ name: 'ollama', is_configured: true, metadata: { display_name: 'Ollama' } }]);
  reads.getLibrarianSchedule.mockResolvedValue({ enabled: false, model: 'qa:local' });
  reads.getOllamaStatus.mockResolvedValue({ reachable: true, installed: [{ name: 'qa:local' }], running: [] });
  reads.readSecretConfig.mockResolvedValue(null);
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
async function renderReview() {
  await act(async () => {
    root.render(<StepReview agentName="QA" toneLabel="Balanced" firstMessage="" onFirstMessage={() => {}}
      onEdit={() => {}} steps={{ model: 1, agent: 2, folders: 3, extras: 4 }} onStart={start} saving={false} />);
  });
}
function row(label: string) {
  const term = [...container.querySelectorAll('dt')].find(node => node.textContent === label)!;
  return term.parentElement!.querySelector('dd')!.textContent;
}

it('announces pending reads without blocking an intentional Start', async () => {
  reads.getProviders.mockReturnValue(new Promise(() => {}));
  await renderReview();
  expect(container.textContent).toContain('Checking saved settings…');
  expect(row('Model')).toBe('Checking…');
  const button = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
  expect(button.disabled).toBe(false);
  await act(async () => { button.click(); });
  expect(start).toHaveBeenCalledOnce();
});

it('reports failed optional reads as unavailable, not off or unconfigured', async () => {
  reads.getLibrarianSchedule.mockRejectedValue(new Error('unreachable'));
  reads.readSecretConfig.mockRejectedValue(new Error('unreachable'));
  await renderReview();
  expect(row('Memory model')).toBe('Unavailable');
  expect(row('Web search')).toBe('Unavailable');
  expect(row('Model')).toBe('On this Mac · qa:local');
  expect(row('Code folders')).toBe('None');
  expect(container.textContent).toContain('Unavailable values have not been confirmed.');
});

it('keeps independently verified fields when config and provider reads fail', async () => {
  reads.getConfig.mockRejectedValue(new Error('unreachable'));
  reads.getProviders.mockRejectedValue(new Error('unreachable'));
  await renderReview();
  expect(row('Model')).toBe('Unavailable');
  expect(row('Code folders')).toBe('Unavailable');
  expect(row('Memory model')).toBe('Off');
  expect(row('Web search')).toBe('Not set up');
  expect(container.textContent).not.toContain("QA can't reply yet");
});

it('retains a verified search provider while identifying other unreadable providers', async () => {
  expect(SEARCH_PROVIDERS.length).toBeGreaterThan(1);
  reads.readSecretConfig.mockImplementation((key: string) => key === SEARCH_PROVIDERS[0].keyName
    ? Promise.resolve({ maskedValue: 'configured' }) : Promise.reject(new Error('unreachable')));
  await renderReview();
  expect(row('Web search')).toBe(`${SEARCH_PROVIDERS[0].displayName} · Other providers unavailable`);
});

it('does not report the product-default enabled schedule as ready without its model', async () => {
  reads.getConfig.mockResolvedValue({ config: {} });
  reads.getLibrarianSchedule.mockResolvedValue({ enabled: true, model: 'qwen2.5:7b' });
  reads.getOllamaStatus.mockResolvedValue({ reachable: false, installed: [], running: [] });
  await renderReview();
  expect(row('Model')).toBe('Not connected');
  expect(row('Memory model')).toBe('On · Ollama unavailable');
  expect(row('Memory model')).not.toContain('On · qwen2.5:7b');
});
