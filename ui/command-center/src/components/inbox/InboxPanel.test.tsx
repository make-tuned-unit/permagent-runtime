/**
 * @vitest-environment jsdom
 *
 * Downloads inbox per-destination result rows (#3, fix G1+G2 forensics item
 * 3). Before this thread, `InboxPanel` kept exactly one result per FILE
 * (`results[fileId]`): routing a file to a project (success) and then to the
 * Brain (failure) overwrote the project's success line with a Brain error
 * that never said which destination it was about — a real second route on
 * the same row made the first one's outcome vanish from the screen.
 *
 * Mounts the REAL `InboxPanel`; only `../../lib/api` is mocked (the
 * ModelsPanel/SpendPanel pattern used across this test suite), so this proves
 * the component's own state keying rather than a mock's behavior.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const allowed = vi.hoisted(() => ({
  getInbox: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`InboxPanel touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { InboxPanel } from './InboxPanel';
import { api, apiFetch, type InboxFile } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getInboxMock = vi.mocked(api.getInbox);
const apiFetchMock = vi.mocked(apiFetch);

const FILE: InboxFile = {
  id: 'file-1',
  filename: 'brief.pdf',
  original_url: 'https://example.com/brief.pdf',
  content_type: 'application/pdf',
  size_bytes: 1024,
  disk_path: 'brief.pdf',
  status: 'received',
  project_id: null,
  created_at: '2026-09-01T00:00:00Z',
};

const PROJECT = { id: 'proj-1', name: 'Acme' };

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getInboxMock.mockReset();
  apiFetchMock.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function findButton(text: string): HTMLButtonElement {
  const btn = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === text);
  expect(btn, `expected a button with text ${JSON.stringify(text)}`).toBeTruthy();
  return btn as HTMLButtonElement;
}

describe('InboxPanel per-destination results', () => {
  it('keeps a project success visible after a later Brain failure on the same row, each line naming its destination', async () => {
    getInboxMock.mockResolvedValue([FILE]);
    apiFetchMock.mockImplementation(async (endpoint: unknown, options?: RequestInit) => {
      const ep = endpoint as string;
      if (ep === '/api/projects') return [PROJECT] as unknown;
      if (ep === `/api/inbox/${FILE.id}/route` && options?.method === 'POST') {
        const body = JSON.parse((options.body as string) ?? '{}') as { destination: string };
        if (body.destination === 'project') {
          return {
            file: { ...FILE, status: 'routed', project_id: PROJECT.id },
            destination: 'project',
            summary: null,
            document_id: 'doc-1',
            card_id: null,
          } as unknown;
        }
        if (body.destination === 'brain') {
          throw new Error('The Reader could not read brief.pdf: garbled text');
        }
      }
      throw new Error(`unexpected fetch ${ep}`);
    });

    await act(async () => {
      root.render(<InboxPanel embedded />);
    });
    await flush();

    expect(container.textContent).toContain('brief.pdf');

    // 1) Route to a project — succeeds.
    await act(async () => {
      findButton('Project…').click();
    });
    await flush();

    const select = container.querySelector('select') as HTMLSelectElement;
    expect(select).toBeTruthy();
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value')!.set!;
      setter.call(select, PROJECT.id);
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await act(async () => {
      findButton('File it').click();
    });
    await flush();

    expect(container.textContent).toContain('Filed as a document in Acme');

    // 2) Route the SAME file to the Brain — fails.
    await act(async () => {
      findButton('Brain').click();
    });
    await flush();

    const text = container.textContent ?? '';
    expect(text).toContain('Filed as a document in Acme'); // project success must still be visible
    expect(text).toContain('Could not send brief.pdf to the Brain'); // failure names its destination
    expect(text).toContain('garbled text'); // and still carries the real reason
  });
});
