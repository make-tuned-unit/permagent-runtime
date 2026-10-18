/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';

const apiFetch = vi.hoisted(() => vi.fn());
vi.mock('../../lib/api', () => ({ apiFetch }));
vi.mock('./PeopleGraphCanvas', () => ({ PeopleGraph: () => <div>Your network</div> }));

import { PeopleView } from './PeopleView';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url === '/api/people/calendar/import') return Promise.reject(new Error('calendar permission service unavailable'));
    if (url === '/api/people') return Promise.resolve([]);
    return Promise.resolve([]);
  });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

it('surfaces calendar import failure and retries the same API path on intent', async () => {
  await act(async () => { root.render(<PeopleView />); });
  expect(host.textContent).toContain("Couldn't check your calendar");
  const retry = [...host.querySelectorAll('button')].find(button => button.textContent === 'Retry')!;
  apiFetch.mockImplementation((url: string) => {
    if (url === '/api/people/calendar/import') return Promise.resolve({ imported: 2 });
    if (url === '/api/people') return Promise.resolve([]);
    return Promise.resolve([]);
  });
  const peopleRev = useCommandCenter.getState().peopleRev;

  await act(async () => { retry.click(); });

  const imports = apiFetch.mock.calls.filter(([url, init]) => url === '/api/people/calendar/import' && init?.method === 'POST');
  expect(imports).toHaveLength(2);
  expect(host.textContent).toContain('Calendar synced · 2 new meetings');
  expect(useCommandCenter.getState().peopleRev).toBe(peopleRev + 1);
});
