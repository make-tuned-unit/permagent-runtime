/** @vitest-environment jsdom
 *
 * Home's banner slot (C8). Echo and Learn next used to render at the same
 * time in identical shells, which read as one banner split in two. These pin
 * the rule that replaced that: at most one, and always the same one for the
 * same readiness — a slot that changed its mind between renders would flicker.
 */

import { afterEach, beforeEach, expect, it } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

import { slotHolder, useBannerSlot, useBannerSlotStore, BANNER_ORDER } from './bannerSlot';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useBannerSlotStore.setState({ ready: {} });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function Banner({ id, ready }: { id: 'echo' | 'learn-next'; ready: boolean }) {
  const holds = useBannerSlot(id, ready);
  return holds ? <div data-testid={`banner-${id}`}>{id}</div> : null;
}

function Home({ echo, learn }: { echo: boolean; learn: boolean }) {
  return (
    <>
      <Banner id="echo" ready={echo} />
      <Banner id="learn-next" ready={learn} />
    </>
  );
}

function shown(): string[] {
  return Array.from(container.querySelectorAll('[data-testid^="banner-"]'))
    .map(el => el.getAttribute('data-testid')!);
}

it('nothing ready, nothing drawn', () => {
  expect(slotHolder({})).toBeNull();
  expect(slotHolder({ echo: false, 'learn-next': false })).toBeNull();
});

it('only one banner is on screen when both have something to say', async () => {
  await act(async () => { root.render(<Home echo learn />); });
  expect(shown()).toEqual(['banner-learn-next']);
});

it('the one that has something to say gets the slot when it is alone', async () => {
  await act(async () => { root.render(<Home echo learn={false} />); });
  expect(shown()).toEqual(['banner-echo']);
});

it('the slot passes on when the holder goes quiet', async () => {
  await act(async () => { root.render(<Home echo learn />); });
  expect(shown()).toEqual(['banner-learn-next']);

  // Dismissed / nothing left to teach.
  await act(async () => { root.render(<Home echo learn={false} />); });
  expect(shown()).toEqual(['banner-echo']);
});

it('unmounting releases the slot rather than holding it off screen', async () => {
  await act(async () => { root.render(<Home echo learn />); });
  await act(async () => { root.render(<Banner id="echo" ready />); });
  expect(shown()).toEqual(['banner-echo']);
});

it('priority is fixed, so the same readiness always picks the same banner', () => {
  expect(BANNER_ORDER).toEqual(['learn-next', 'echo']);
  const ready = { echo: true, 'learn-next': true };
  expect(slotHolder(ready)).toBe('learn-next');
  expect(slotHolder(ready)).toBe('learn-next');
});
