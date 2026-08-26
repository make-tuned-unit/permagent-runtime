/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const api = vi.hoisted(() => ({
  getPacks: vi.fn(),
  applyPacks: vi.fn(),
}));

vi.mock('../../lib/api', () => ({ api }));

import { RoleRoutingPrompt } from './RoleRoutingPrompt';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  api.getPacks.mockReset();
  api.applyPacks.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('RoleRoutingPrompt', () => {
  it('shows Apply recommended routing when the daemon says prompt', async () => {
    api.getPacks.mockResolvedValue({
      prompt: true,
      configured: [],
      recommendation: {
        considered: ['anthropic/claude-opus-4-6', 'ollama/qwen3'],
        recommendations: [
          { role: 'orchestrate', provider: 'anthropic', model: 'claude-opus-4-6' },
          { role: 'mechanical', provider: 'ollama', model: 'qwen3' },
        ],
      },
    });

    await act(async () => { root.render(<RoleRoutingPrompt />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });

    expect(container.textContent).toContain('Apply recommended routing');
    expect(container.textContent).toContain('Cheaper per-role routing');
  });

  it('renders nothing when routing is already configured', async () => {
    api.getPacks.mockResolvedValue({
      prompt: false,
      configured: [{ role: 'edit', provider: 'openai', model: 'gpt-5.4' }],
      recommendation: { considered: [], recommendations: [] },
    });

    await act(async () => { root.render(<RoleRoutingPrompt />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });

    expect(container.textContent).not.toContain('Apply recommended routing');
  });
});
