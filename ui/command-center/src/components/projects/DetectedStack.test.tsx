/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));
import { DetectedStack } from './DetectedStack';
import type { Project } from './types';
import { useCommandCenter } from '../../lib/store';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
const project = {id: 'p/a', rootPath: '/sample'} as Project;
beforeEach(() => { apiFetch.mockReset(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
const result = (name = 'React') => ({ entries: [{service_name: name, category: 'other', evidence: ['web/package.json']}], scanned_files: 2, incomplete: false });
it('discovers automatically with source evidence and no write request', async () => {
  apiFetch.mockResolvedValue(result());
  await act(async () => root.render(<DetectedStack project={project} />));
  expect(apiFetch).toHaveBeenCalledOnce();
  expect(apiFetch).toHaveBeenCalledWith('/api/projects/p%2Fa/stack/detected');
  expect(host.textContent).toContain('React');
  expect(host.querySelector('details')?.textContent).toContain('web/package.json');
});
it('does not scan without a connected folder', async () => {
  await act(async () => root.render(<DetectedStack project={{...project, rootPath: null}} />));
  expect(apiFetch).not.toHaveBeenCalled();
  expect(host.textContent).toContain('Connect a code folder');
});
it('prepares a connection request without sending or replacing an existing draft', async () => {
  const openChatDock = vi.fn();
  const originalOpen = useCommandCenter.getState().openChatDock;
  useCommandCenter.setState({ chatDraft: { text: 'My existing draft', files: [] }, openChatDock });
  try {
    await act(async () => root.render(<DetectedStack project={{...project, name: 'Garden', rootPath: null}} />));
    await act(async () => host.querySelector('button')!.click());
    expect(useCommandCenter.getState().chatDraft.text).toContain('My existing draft\n\nHelp me connect the project "Garden"');
    expect(openChatDock).toHaveBeenCalledOnce();
    expect(apiFetch).not.toHaveBeenCalled();
  } finally { useCommandCenter.setState({ openChatDock: originalOpen, chatDraft: {text: '', files: []} }); }
});
it('distinguishes failed and partial discovery from an empty stack', async () => {
  apiFetch.mockRejectedValueOnce(new Error('offline'));
  await act(async () => root.render(<DetectedStack project={project} />));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('Couldn’t read');
  expect(host.textContent).not.toContain('No supported');
  apiFetch.mockResolvedValue({...result(), incomplete: true});
  await act(async () => host.querySelector('button')!.click());
  expect(host.textContent).toContain('Partial scan');
  expect(host.textContent).toContain('React');
});
it('ignores an old project response after switching projects', async () => {
  let resolveOld!: (value: unknown) => void;
  apiFetch.mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; })).mockResolvedValue(result('Rust'));
  await act(async () => root.render(<DetectedStack project={project} />));
  await act(async () => root.render(<DetectedStack project={{...project, id: 'b'}} />));
  await act(async () => resolveOld(result('React')));
  expect(host.textContent).toContain('Rust');
  expect(host.textContent).not.toContain('React');
});
