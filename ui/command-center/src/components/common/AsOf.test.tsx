/** @vitest-environment jsdom
 *
 * `<AsOf>` is the one place staleness is drawn. Home's "the figures stopped
 * being refreshed" caption used to compose its own sentence and colour it by
 * hand; the sentences it promised are asserted here, so folding it onto the
 * primitive is a refactor and not a rewording.
 */

import { beforeEach, afterEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { AsOf } from './AsOf';
import { getThemedColors } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = Date.parse('2026-08-31T12:00:00Z');
const minutesAgo = (m: number) => NOW - m * 60_000;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(node: React.ReactElement) {
  act(() => root.render(node));
  return container.firstElementChild as HTMLElement;
}

describe('AsOf', () => {
  it('says how old the figures are and that something is still trying', () => {
    render(
      <AsOf asOf={minutesAgo(2)} now={NOW} prefix="Updated" suffix="reconnecting" staleAfterMs={0} />,
    );
    expect(container.textContent).toBe('Updated 2m ago · reconnecting');
  });

  it('keeps the rest of Home\'s sentences intact', () => {
    const say = (at: number | null) => {
      act(() => root.render(
        <AsOf
          asOf={at}
          now={NOW}
          prefix="Updated"
          suffix="reconnecting"
          staleAfterMs={0}
          unknownLabel="Can't reach the daemon"
        />,
      ));
      return container.textContent;
    };
    expect(say(minutesAgo(30))).toBe('Updated 30m ago · reconnecting');
    expect(say(minutesAgo(150))).toBe('Updated 2h ago · reconnecting');
    expect(say(minutesAgo(60 * 30))).toBe('Updated 1d ago · reconnecting');
    // Nothing ever loaded: no prefix, because there is no "updated" to date.
    expect(say(null)).toBe("Can't reach the daemon · reconnecting");
  });

  it('colours a stale reading with the stale role, not the plain text colour', () => {
    const el = render(<AsOf asOf={minutesAgo(2)} now={NOW} staleAfterMs={0} />);
    expect(el.style.color).toBeTruthy();
    expect(el.style.color).not.toBe(getThemedColors().text);
  });

  it('stays quiet while the reading is still fresh', () => {
    const el = render(<AsOf asOf={minutesAgo(2)} now={NOW} />);
    expect(el.textContent).toBe('2m ago');
    // No dot, no alarm colour — a fresh figure needs no decoration.
    expect(el.querySelector('[data-testid="as-of-dot"]')).toBeNull();
  });

  it('marks a stale reading non-verbally too, for anyone who does not read colour', () => {
    const el = render(<AsOf asOf={minutesAgo(2)} now={NOW} staleAfterMs={0} dot />);
    expect(el.querySelector('[data-testid="as-of-dot"]')).toBeTruthy();
    expect(el.querySelector('[data-testid="as-of-dot"]')!.getAttribute('aria-hidden')).toBe('true');
  });

  it('puts the exact timestamp on hover', () => {
    const el = render(<AsOf asOf={minutesAgo(2)} now={NOW} />);
    expect(el.title).toBe(new Date(minutesAgo(2)).toLocaleString());
  });
});
