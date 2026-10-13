// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const reads = vi.hoisted(() => ({
  getProviders: vi.fn(), getOllamaStatus: vi.fn(), getSystemInfo: vi.fn(),
}));
vi.mock('../../lib/api', () => ({ api: reads }));
vi.mock('../../styles/useTheme', async () => {
  const { getThemedColors } = await import('../../styles/tokens');
  return { useTheme: () => ({ colors: getThemedColors(), reduceMotion: true, theme: 'dark' }) };
});
import { StepModel } from './StepModel';
import { localRuntimeFacts, localRuntimeCard } from './localRuntime';
import { memoryStatus, memoryStatusLabel } from './memoryStatus';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const providers = [{ name: 'ollama', is_configured: false, is_default: false, metadata: { display_name: 'Ollama' } }];

describe('local runtime: one answer for every step', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    Object.values(reads).forEach(r => r.mockReset());
    reads.getProviders.mockResolvedValue(providers);
    reads.getSystemInfo.mockResolvedValue({ total_ram_bytes: 16 * 1024 ** 3 });
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  async function renderModel() {
    await act(async () => { root.render(<StepModel initialChoice={null} onDone={() => {}} />); });
  }
  const localRadio = () => container.querySelector<HTMLInputElement>('input[value="local"]')!;

  it('an unreachable Ollama is unavailable in step 2 and in the review memory check, and the card is disabled', async () => {
    const answer = { reachable: false, installed: [], running: [] };
    reads.getOllamaStatus.mockResolvedValue(answer);
    await renderModel();
    expect(container.textContent).toContain('Ollama unavailable');
    expect(container.textContent).not.toContain('Local runtime available');
    expect(localRadio().disabled).toBe(true);
    const review = memoryStatusLabel(memoryStatus({ schedule: { enabled: true, model: 'm' }, ollama: answer }));
    expect(review).toBe('Ollama unavailable');
  });

  it('a reachable Ollama with models is ready and selectable', async () => {
    reads.getOllamaStatus.mockResolvedValue({ reachable: true, installed: [{ name: 'm', size: 1 }], running: [] });
    await renderModel();
    expect(container.textContent).toContain('Ollama ready');
    expect(localRadio().disabled).toBe(false);
  });

  it('a failed Ollama read is reported as unavailable, not as available', async () => {
    reads.getOllamaStatus.mockRejectedValue(new Error('boom'));
    await renderModel();
    expect(container.textContent).toContain('could not check it');
    expect(localRadio().disabled).toBe(true);
  });

  it('keeps the four facts separate', () => {
    const f = localRuntimeFacts({ providers: [{ name: 'ollama', is_configured: true, is_default: true } as never], ollama: { reachable: true, installed: [] } });
    expect(f).toMatchObject({ offered: true, configured: true, reachable: true, ready: false, inUse: true });
    expect(localRuntimeCard(f, true).enabled).toBe(true);
    expect(localRuntimeCard(localRuntimeFacts({ providers: [], ollama: null }), true).status).toBe('Not offered by this service');
  });
});
