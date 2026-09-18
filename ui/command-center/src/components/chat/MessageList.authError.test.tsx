/** @vitest-environment jsdom
 *
 * The chat pane must say so when the client has no working daemon credential.
 *
 * With no token every request 401s — including the `/events` stream replies
 * ride — so the agent never answers. Before this surface existed the chat just
 * sat there: no bubble, no error, no spinner. A dead agent looked exactly like
 * a quiet one, which is why the underlying token bug went unnoticed for so
 * long. This pins the banner to `daemonAuthError`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { create } from 'zustand';

const hoisted = vi.hoisted(() => ({ store: null as unknown as ReturnType<typeof makeStore> }));

function makeStore() {
  return create<{
    chatMessages: unknown[];
    isStreaming: boolean;
    agentName: string;
    sessionLoadError: string | null;
    daemonAuthError: string | null;
    chatSessionId: string | null;
    chatHistoryLoaded: boolean;
    loadSessionMessages: () => void;
  }>(() => ({
    chatMessages: [],
    isStreaming: false,
    agentName: 'Henry',
    sessionLoadError: null,
    daemonAuthError: null,
    chatSessionId: 's1',
    chatHistoryLoaded: true,
    loadSessionMessages: () => {},
  }));
}

vi.mock('../../lib/store', () => {
  const s = makeStore();
  hoisted.store = s;
  return { useCommandCenter: s };
});
vi.mock('../settings/useSettings', () => ({ usePersona: () => ({ data: undefined }) }));
vi.mock('../../lib/useVoices', () => ({ useVoicePreview: () => ({ preview: vi.fn(), playingId: null }) }));
vi.mock('../../lib/speakReplies', () => ({
  hasSpokenKey: () => true,
  markReplySpoken: vi.fn(),
  replyDedupeKey: () => 'k',
}));
vi.mock('./MarkdownContent', () => ({ MarkdownContent: () => null }));

import { MessageList } from './MessageList';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { cb(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  Element.prototype.scrollIntoView = function () {};
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function banner() {
  return container.querySelector('[data-testid="daemon-auth-error"]');
}

describe('MessageList daemon-auth surface', () => {
  it('shows nothing while the credential is healthy', () => {
    act(() => { root.render(<MessageList />); });
    expect(banner()).toBeNull();
  });

  it('names the auth failure instead of leaving the chat silent', () => {
    act(() => { hoisted.store.setState({ daemonAuthError: 'Could not read the daemon token — requests are unauthenticated (HOME not set)' }); });
    act(() => { root.render(<MessageList />); });

    const el = banner();
    expect(el).not.toBeNull();
    expect(el?.getAttribute('role')).toBe('alert');
    expect(el?.textContent).toContain('Not connected to your agent');
    expect(el?.textContent).toContain('HOME not set');
  });

  it('clears the banner once auth recovers', () => {
    act(() => { hoisted.store.setState({ daemonAuthError: 'boom' }); });
    act(() => { root.render(<MessageList />); });
    expect(banner()).not.toBeNull();

    act(() => { hoisted.store.setState({ daemonAuthError: null }); });
    expect(banner()).toBeNull();
  });
});
