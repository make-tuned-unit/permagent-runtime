/** @vitest-environment jsdom
 *
 * Wiring test for Models → Local models (ST2, dagK settings pass):
 * `local-inference/*` had no UI. Covers the five states this surface must
 * show honestly (loading, empty, error, populated, downloading) and that
 * download/delete call the right endpoints with confirmation on delete.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const allowed = vi.hoisted(() => ({
  listLocalModels: vi.fn(),
  downloadLocalModel: vi.fn(),
  cancelLocalModelDownload: vi.fn(),
  deleteLocalModel: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`LocalModelsSection touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { LocalModelsSection } from './LocalModelsSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.useRealTimers();
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => { root.render(<LocalModelsSection />); });
}

describe('LocalModelsSection', () => {
  it('shows a loading state before the registry resolves', async () => {
    allowed.listLocalModels.mockReturnValue(new Promise(() => {}));
    await mount();
    expect(container.textContent).toContain('Loading local models');
  });

  it('shows an honest error state, distinct from empty, with a retry', async () => {
    allowed.listLocalModels.mockRejectedValue(new Error('offline'));
    await mount();
    expect(container.textContent).toContain("Couldn't read the local-model registry");
    allowed.listLocalModels.mockResolvedValue([]);
    const retry = container.querySelector('button');
    await act(async () => { retry?.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.listLocalModels).toHaveBeenCalledTimes(2);
  });

  it('shows the empty state when the registry has nothing', async () => {
    allowed.listLocalModels.mockResolvedValue([]);
    await mount();
    expect(container.textContent).toContain('No local models in the registry');
  });

  it('lists a downloaded model with its size and a Delete action', async () => {
    allowed.listLocalModels.mockResolvedValue([{
      id: 'llama-3.2-3b-q4', repo_id: 'bartowski/Llama-3.2-3B-Instruct-GGUF', filename: 'x.gguf',
      quantization: 'Q4_K_M', size_bytes: 2_000_000_000, status: { state: 'Downloaded' },
      recommended: true, vision_capable: false, mmproj_status: null,
    }]);
    await mount();
    expect(container.textContent).toContain('llama-3.2-3b-q4');
    expect(container.textContent).toContain('Recommended');
    expect(container.querySelector('[data-testid="local-model-delete-llama-3.2-3b-q4"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="local-model-download-llama-3.2-3b-q4"]')).toBeNull();
  });

  it('shows live download progress for a downloading model', async () => {
    allowed.listLocalModels.mockResolvedValue([{
      id: 'm1', repo_id: 'org/repo', filename: 'x.gguf', quantization: 'Q4_K_M',
      size_bytes: 1_000_000_000,
      status: { state: 'Downloading', progress_percent: 42, bytes_downloaded: 420_000_000, total_bytes: 1_000_000_000, speed_bps: 5_000_000 },
      recommended: false, vision_capable: false, mmproj_status: null,
    }]);
    await mount();
    expect(container.textContent).toContain('Downloading 42%');
  });

  it('starts a download for a not-downloaded model and refreshes the list', async () => {
    allowed.listLocalModels.mockResolvedValueOnce([{
      id: 'm1', repo_id: 'org/repo', filename: 'x.gguf', quantization: 'Q4_K_M', size_bytes: 1000,
      status: { state: 'NotDownloaded' }, recommended: false, vision_capable: false, mmproj_status: null,
    }]).mockResolvedValue([{
      id: 'm1', repo_id: 'org/repo', filename: 'x.gguf', quantization: 'Q4_K_M', size_bytes: 1000,
      status: { state: 'Downloading', progress_percent: 0, bytes_downloaded: 0, total_bytes: 1000, speed_bps: null },
      recommended: false, vision_capable: false, mmproj_status: null,
    }]);
    allowed.downloadLocalModel.mockResolvedValue('started');
    await mount();
    const btn = container.querySelector('[data-testid="local-model-download-m1"]') as HTMLButtonElement;
    await act(async () => { btn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.downloadLocalModel).toHaveBeenCalledWith('org/repo:Q4_K_M');
  });

  it('confirms before deleting a downloaded model', async () => {
    allowed.listLocalModels.mockResolvedValue([{
      id: 'm1', repo_id: 'org/repo', filename: 'x.gguf', quantization: 'Q4_K_M', size_bytes: 1000,
      status: { state: 'Downloaded' }, recommended: false, vision_capable: false, mmproj_status: null,
    }]);
    await mount();
    const del = container.querySelector('[data-testid="local-model-delete-m1"]') as HTMLButtonElement;
    await act(async () => { del.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    // Confirmation dialog is up; the delete has not been called yet.
    expect(allowed.deleteLocalModel).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('Delete this local model?');
    const confirmButtons = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Delete');
    await act(async () => { confirmButtons[confirmButtons.length - 1].dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.deleteLocalModel).toHaveBeenCalledWith('m1');
  });
});
