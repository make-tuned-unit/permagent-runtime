/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it } from 'vitest';
import { HabitatLearning } from './HabitatLearning';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it('offers all three non-dispatching demonstrations with feedback and revisit', () => {
  const host = document.createElement('div'); document.body.append(host); const root = createRoot(host);
  act(() => root.render(<HabitatLearning />));
  expect(host.textContent).toContain('Plan against constraints');
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent?.includes('Confirm outcome'))!.click());
  expect(host.querySelector('[role=status]')?.textContent).toContain('Supported plan');
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'Garden archive')!.click());
  expect(host.textContent).toContain('2019 brochure');
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent?.includes('Garden room confirmed'))!.click());
  expect(host.querySelector('[role=status]')?.textContent).toContain('Supported by the newer record');
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'Water workshop')!.click());
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'Add shade')!.click());
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent?.includes('surface warms'))!.click());
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'Inspect the illustrative result')!.click());
  expect(host.querySelector('[role=status]')?.textContent).toContain('Prediction matches');
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'Revisit this demonstration')!.click());
  expect(host.querySelector('[role=status]')).toBeNull();
  act(() => root.unmount()); host.remove();
});
