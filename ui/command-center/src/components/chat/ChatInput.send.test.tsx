/**
 * @vitest-environment jsdom
 *
 * The composer's send button. Reported 2026-09-20: "i dont like how the send
 * message button looks. it needs design work."
 *
 * What was wrong, in the markup: it was filled with `ribbonGradient` — the
 * brand mark's gradient, on a control — and its hover and press states were
 * set to that SAME gradient, so pressing it produced no change at all. It was
 * also 28px beside a 36px field, so the row ran at three different heights.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../voice/VoiceButton', () => ({ VoiceButton: () => null }));

import { ChatInput } from './ChatInput';
import { useCommandCenter } from '../../lib/store';
import { getThemedColors, setTheme } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  setTheme('dark');
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => { useCommandCenter.setState({ isStreaming: false, chatDraft: { text: '', files: [] } }); });
  act(() => root.render(<ChatInput />));
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

const send = () => container.querySelector('[aria-label="Send message"]') as HTMLElement;
const field = () => container.querySelector('textarea') as HTMLTextAreaElement;

/** Type into the composer the way React's controlled input hears it. */
function type(text = 'hello') {
  act(() => {
    const el = field();
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    setter.call(el, text);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('the composer placeholder', () => {
  // Reported twice on 2026-09-20: "Message your agent..." wrapped to a second
  // line in the docked composer and then overflowed the field's box. A
  // textarea's native placeholder wraps and cannot be ellipsized, so it is
  // drawn instead.
  it('is one line that truncates, never the textarea\'s own wrapping one', () => {
    expect(field().getAttribute('placeholder')).toBeNull();
    const drawn = container.querySelector('[aria-hidden="true"]') as HTMLElement;
    expect(drawn?.textContent).toMatch(/^Message /);
    expect(drawn.style.whiteSpace).toBe('nowrap');
    expect(drawn.style.textOverflow).toBe('ellipsis');
    expect(drawn.style.overflow).toBe('hidden');
    // The accessible name survives the swap.
    expect(field().getAttribute('aria-label')).toBe(drawn.textContent);
  });

  it('disappears as soon as there is text, so it cannot sit under the caret', () => {
    type();
    expect(container.querySelector('[aria-hidden="true"]')).toBeNull();
  });
});

describe('send button', () => {
  it('is square with the field, not a smaller box beside it', () => {
    expect(send().style.width).toBe(send().style.height);
    expect(send().style.width).toBe(field().style.minHeight);
  });

  it('never wears the brand gradient, and answers a press once it can send', () => {
    type();
    const style = send().style;
    expect(style.getPropertyValue('--pa-btn-bg')).not.toContain('gradient');
    // A press has to look like something happened; both were the same gradient.
    expect(style.getPropertyValue('--pa-btn-bg-active'))
      .not.toBe(style.getPropertyValue('--pa-btn-bg'));
  });

  it('goes quiet with nothing to send, and lights up once there is', () => {
    const colors = getThemedColors();
    expect((send() as HTMLButtonElement).disabled).toBe(true);
    expect(send().style.getPropertyValue('--pa-btn-bg')).toBe(colors.inputBg);

    type();
    expect((send() as HTMLButtonElement).disabled).toBe(false);
    expect(send().style.getPropertyValue('--pa-btn-bg')).toBe(colors.cyan);
  });
});
