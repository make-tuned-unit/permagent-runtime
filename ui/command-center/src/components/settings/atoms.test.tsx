/** @vitest-environment jsdom
 *
 * Covers the atoms added for the dagK settings pass: `Row`'s `dim` prop (a
 * never-seen device fades rather than disappearing, ST5) and `Details` (long
 * technical prose behind a disclosure, ST1 — UX_STANDARD.md R3).
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';
import { Details, ListRow, Row, Section } from './atoms';
import { Toggle } from '../common/Toggle';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
});

describe('Row dim', () => {
  it('fades the row when dim is set, without hiding its content', async () => {
    await act(async () => {
      root.render(
        <Section title="Devices">
          <Row label="Never-seen phone" hint="never seen" dim />
        </Section>,
      );
    });
    const row = container.querySelector('.pa-set-row') as HTMLElement;
    expect(row.style.opacity).toBe('0.6');
    expect(container.textContent).toContain('Never-seen phone');
  });

  it('renders at full opacity when dim is not set', async () => {
    await act(async () => {
      root.render(
        <Section title="Devices">
          <Row label="Active phone" hint="last seen 2m ago" />
        </Section>,
      );
    });
    const row = container.querySelector('.pa-set-row') as HTMLElement;
    expect(row.style.opacity === '' || row.style.opacity === '1').toBe(true);
  });
});

describe('Details', () => {
  it('renders a native, keyboard-operable disclosure with a short summary', async () => {
    await act(async () => {
      root.render(
        <Details summary="How this works">
          <span>Technical prose goes here.</span>
        </Details>,
      );
    });
    const details = container.querySelector('details');
    expect(details).toBeTruthy();
    expect(container.querySelector('summary')?.textContent).toBe('How this works');
    expect(container.textContent).toContain('Technical prose goes here.');
  });
});

describe('ListRow', () => {
  it('renders as a button when onOpen is given, and a plain row otherwise', async () => {
    await act(async () => { root.render(<ListRow title="Alpha" onOpen={() => {}} />); });
    expect(container.querySelector('button')).toBeTruthy();
    await act(async () => { root.render(<ListRow title="Beta" />); });
    expect(container.querySelector('button')).toBeNull();
    expect(container.textContent).toContain('Beta');
  });
});

describe('Row switch labels', () => {
  it('names direct and nested switches from their own visible row without leaking labels', async () => {
    await act(async () => {
      root.render(<>
        <Row label="Task completed"><Toggle on /></Row>
        <Row label="Remote access"><div><Toggle on={false} disabled /></div></Row>
        <Row label="Data sources"><Toggle on label="Enable Public Weather" /></Row>
        <Toggle on label="Standalone switch" />
      </>);
    });
    const switches = Array.from(container.querySelectorAll('[role="switch"]'));
    expect(switches.map(el => el.getAttribute('aria-label'))).toEqual([
      'Task completed', 'Remote access', 'Enable Public Weather', 'Standalone switch',
    ]);
    expect(switches.map(el => el.getAttribute('aria-checked'))).toEqual(['true', 'false', 'true', 'true']);
    expect((switches[1] as HTMLButtonElement).disabled).toBe(true);
  });
});
