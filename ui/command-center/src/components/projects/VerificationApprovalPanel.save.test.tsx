/**
 * @vitest-environment jsdom
 *
 * VerificationApprovalPanel writes — the PUT endpoint is a server-side MERGE
 * (like PUT /api/projects/{id}/brand), so every save must ship ONLY the
 * fields that actually changed, and Reset must ship exactly {reset: true}.
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

const baseVa = {
  allowlist: ['cargo'],
  cleanRuns: 3,
  readOnlyThreshold: 5,
  fullThreshold: 20,
  onceGrants: [],
  audit: [
    { at: '2026-08-20T00:00:00Z', command: 'cargo test', tier: 'user', decision: 'approved_once', privilege: 0, level: 'none', reason: 'first run' },
  ],
};

function projectWith(va: Record<string, unknown>): Project {
  return { ...project, metadataJson: { verification_approval: va } } as unknown as Project;
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

async function mount() {
  apiFetch.mockResolvedValue(projectWith(baseVa));
  await act(async () => root.render(<VerificationApprovalPanel project={project} />));
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

it('adding an allowlist entry sends only the updated allowlist', async () => {
  await mount();
  apiFetch.mockResolvedValueOnce(projectWith({ ...baseVa, allowlist: ['cargo', 'npm'] }));

  const input = container.querySelector('input[placeholder^="Command token"]') as HTMLInputElement;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  await act(async () => {
    setter.call(input, 'npm');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const addBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent?.includes('Add'));
  await act(async () => { addBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

  const putCall = apiFetch.mock.calls.find(([url]) => String(url).endsWith('/verification-approval'));
  expect(putCall).toBeTruthy();
  const [url, opts] = putCall!;
  expect(url).toBe('/api/projects/proj-1/verification-approval');
  expect(opts.method).toBe('PUT');
  const body = JSON.parse(opts.body as string);
  expect(Object.keys(body)).toEqual(['allowlist']);
  expect(body.allowlist).toEqual(['cargo', 'npm']);
});

it('saving thresholds sends only the changed threshold, not both and not the allowlist', async () => {
  await mount();
  apiFetch.mockResolvedValueOnce(projectWith({ ...baseVa, fullThreshold: 30 }));

  const inputs = Array.from(container.querySelectorAll('input[type="number"]')) as HTMLInputElement[];
  // [0] = readOnlyThreshold draft (unchanged), [1] = fullThreshold draft (changed)
  const fullInput = inputs[1];
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  await act(async () => {
    setter.call(fullInput, '30');
    fullInput.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const saveBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Save thresholds');
  await act(async () => { saveBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

  const putCall = apiFetch.mock.calls.find(([url]) => String(url).endsWith('/verification-approval'));
  expect(putCall).toBeTruthy();
  const body = JSON.parse(putCall![1].body as string);
  expect(Object.keys(body)).toEqual(['fullThreshold']);
  expect(body.fullThreshold).toBe(30);
});

it('Reset sends exactly {reset: true}', async () => {
  await mount();
  apiFetch.mockResolvedValueOnce(projectWith({
    allowlist: [], cleanRuns: 0, readOnlyThreshold: 5, fullThreshold: 20, onceGrants: [], audit: baseVa.audit,
  }));

  const resetBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Reset');
  await act(async () => { resetBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

  const putCall = apiFetch.mock.calls.find(([url]) => String(url).endsWith('/verification-approval'));
  expect(putCall).toBeTruthy();
  const body = JSON.parse(putCall![1].body as string);
  expect(body).toEqual({ reset: true });
});
