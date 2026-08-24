/**
 * @vitest-environment jsdom
 *
 * VerificationApprovalPanel — earned privilege must read as none/read-only/
 * full exactly at the cleanRuns boundaries against the two thresholds
 * (default read-only=5, full=20; explicit thresholds override).
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { VerificationApprovalPanel } from './VerificationApprovalPanel';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const project = { id: 'proj-1' } as Project;

function projectWith(cleanRuns: number): Project {
  return {
    ...project,
    metadataJson: {
      verification_approval: {
        allowlist: [],
        cleanRuns,
        readOnlyThreshold: 5,
        fullThreshold: 20,
        onceGrants: [],
        audit: [],
      },
    },
  } as unknown as Project;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(cleanRuns: number) {
  apiFetch.mockResolvedValue(projectWith(cleanRuns));
  await act(async () => root.render(<VerificationApprovalPanel project={project} />));
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

it('reads as None at cleanRuns 0', async () => {
  await mount(0);
  const badge = container.querySelector('[data-testid="privilege-level"]');
  expect(badge?.getAttribute('data-level')).toBe('none');
  expect(container.textContent).toContain('None');
  expect(container.textContent).toContain('0 clean runs');
});

it('reads as Read-only at cleanRuns 5 (the read-only threshold)', async () => {
  await mount(5);
  const badge = container.querySelector('[data-testid="privilege-level"]');
  expect(badge?.getAttribute('data-level')).toBe('read_only');
  expect(container.textContent).toContain('Read-only');
  expect(container.textContent).toContain('5 clean runs');
});

it('reads as Full at cleanRuns 20 (the full threshold)', async () => {
  await mount(20);
  const badge = container.querySelector('[data-testid="privilege-level"]');
  expect(badge?.getAttribute('data-level')).toBe('full');
  expect(container.textContent).toContain('Full');
  expect(container.textContent).toContain('20 clean runs');
});
