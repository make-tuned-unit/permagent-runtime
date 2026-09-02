/** @vitest-environment jsdom
 *
 * Home's grid used to answer "no registry entry for this card type" with
 * `return null`. Two cards in the *default* layout are daemon-served, so that
 * fired whenever the manifest fetch was slow or down — and Reset to default put
 * both of them back into the layout, where they rendered as nothing at all. The
 * button that promises to restore your dashboard appeared to delete cards from
 * it.
 *
 * Three states, and they must not read alike: still coming is not gone, a
 * failed fetch is not an absence, and a card type that really has gone says so.
 */

import { describe, expect, it, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { MissingCard } from './MissingCard';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

function render(status: 'loading' | 'ready' | 'error'): HTMLElement {
  act(() => { root.render(<MissingCard type="calendar" status={status} />); });
  return host.querySelector('[data-testid="missing-card-calendar"]') as HTMLElement;
}

describe('a card the registry has no entry for', () => {
  it('holds its place instead of vanishing', () => {
    expect(render('loading')).not.toBeNull();
    expect(render('error')).not.toBeNull();
    expect(render('ready')).not.toBeNull();
  });

  it('says it is still coming while the manifest is in flight', () => {
    expect(render('loading').textContent).toContain('Loading');
  });

  it('says a failed fetch failed, not that the card is gone', () => {
    const text = render('error').textContent ?? '';
    expect(text).toContain('unavailable');
    expect(text).toContain('come back');
    expect(text).not.toContain('no longer');
  });

  it('says a genuinely absent card type is absent, and how to clear it', () => {
    const text = render('ready').textContent ?? '';
    expect(text).toContain('no longer available');
    expect(text).toContain('Customize');
  });

  it('names the card type, so the placeholder is identifiable', () => {
    expect(render('ready').textContent).toContain('calendar');
  });
});
