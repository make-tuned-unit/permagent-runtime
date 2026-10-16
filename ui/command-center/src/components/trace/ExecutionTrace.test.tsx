/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it } from 'vitest';
import { ExecutionTrace } from './ExecutionTrace';
import { useCommandCenter, type EventRecord } from '../../lib/store';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
it('keeps work outcomes separate from diagnostic churn and filters errors without losing records', async () => {
  const events: EventRecord[] = ['app_navigate', 'task_completed', 'task_failed'].map((event_type, i) => ({ id: String(i), timestamp: '2026-10-15T12:00:00Z', source: 'events', event_type, severity: i === 2 ? 'error' : 'info', run_id: 'run-garden', task_id: null, agent_id: null, correlation_id: null, payload: {} }));
  useCommandCenter.setState({ events });
  const host = document.createElement('div'); document.body.appendChild(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<ExecutionTrace activity />));
    expect(host.textContent).toContain('Latest 2 matching');
    expect(host.textContent).not.toContain('App navigate');
    const change = async (label: string, value: string) => act(async () => {
      const input = host.querySelector<HTMLSelectElement>(`[aria-label="${label}"]`)!;
      input.value = value; input.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await change('Activity view', 'trace');
    expect(host.textContent).toContain('Latest 3 matching');
    await change('Activity status', 'errors');
    expect(host.textContent).toContain('Latest 1 matching');
    expect(useCommandCenter.getState().events).toEqual(events);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
