/**
 * C1 (dagK live walk, 2026-09-24): reopening a session whose agent had called
 * card_create drew an EMPTY "You 03:21 PM" bubble between the two assistant
 * turns. The daemon stores each tool result on a user-role message whose only
 * content is a toolResponse block; history conversion mapped it one-to-one to
 * a ChatMessage with role 'user' and content '', and the render filter let
 * every non-assistant message through.
 *
 * The conversation below is the real wire shape (tool_result_serde wrappers,
 * camelCase blocks, metadata), not a hand-simplified one.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getSession } = vi.hoisted(() => ({ getSession: vi.fn() }));

vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api')>();
  return { ...actual, api: { ...actual.api, getSession } };
});

import { isToolResultCarrier, useCommandCenter } from './store';
import { isRenderableChatMessage } from '../components/chat/MessageList';
import type { DaemonMessage } from './api';

const meta = { userVisible: true, agentVisible: true };

const conversation: DaemonMessage[] = [
  {
    id: 'u1', role: 'user', created: 1_790_000_000, metadata: meta,
    content: [{ type: 'text', text: 'Draft a post about the release and pin it to the board.' }],
  },
  {
    id: 'a1', role: 'assistant', created: 1_790_000_010, metadata: meta,
    content: [
      { type: 'text', text: 'I will put the draft on a card.' },
      {
        type: 'toolRequest', id: 'toolu_01',
        toolCall: { status: 'success', value: { name: 'card_create', arguments: { title: 'Release post' } } },
      },
    ],
  },
  {
    // The carrier: user role, nothing but the tool result.
    id: 'u2', role: 'user', created: 1_790_000_011, metadata: meta,
    content: [{
      type: 'toolResponse', id: 'toolu_01',
      toolResult: { status: 'success', value: { content: [{ type: 'text', text: 'card 42 created' }] } },
    }],
  },
  {
    id: 'a2', role: 'assistant', created: 1_790_000_020, metadata: meta,
    content: [{ type: 'text', text: 'Pinned as card 42.' }],
  },
  {
    // A real user turn that ALSO carries an image must survive.
    id: 'u3', role: 'user', created: 1_790_000_030, metadata: meta,
    content: [{ type: 'image', data: 'iVBORw0KGgo=', mimeType: 'image/png' }],
  },
];

describe('tool-result carrier messages (C1)', () => {
  beforeEach(() => {
    getSession.mockReset();
    useCommandCenter.setState({ chatMessages: [], chatSessionId: 'sess-1', sessionLoadError: null });
  });

  it('identifies a user message holding only tool results', () => {
    expect(isToolResultCarrier(conversation[2])).toBe(true);
    expect(isToolResultCarrier(conversation[0])).toBe(false);
    expect(isToolResultCarrier(conversation[1])).toBe(false); // assistant
    expect(isToolResultCarrier(conversation[4])).toBe(false); // image
    expect(isToolResultCarrier({ ...conversation[2], content: [] })).toBe(false);
    expect(isToolResultCarrier({
      ...conversation[2],
      content: [...conversation[2].content, { type: 'text', text: 'and also this' }],
    })).toBe(false);
  });

  it('loads history without an empty You bubble, keeping the result on the tool card', async () => {
    getSession.mockResolvedValueOnce({ id: 'sess-1', conversation });
    await useCommandCenter.getState().loadSessionMessages('sess-1');
    const msgs = useCommandCenter.getState().chatMessages;

    expect(msgs.map(m => m.id)).toEqual(['u1', 'a1', 'a2', 'u3']);
    expect(msgs.every(isRenderableChatMessage)).toBe(true);
    const card = msgs.find(m => m.id === 'a1')?.tool_calls?.[0];
    expect(card).toMatchObject({ name: 'card_create', result: 'card 42 created', success: true });
    expect(msgs.find(m => m.id === 'u3')?.images).toHaveLength(1);
  });

  it('never renders a contentless bubble for any role', () => {
    const empty = { id: 'x', content: '', timestamp: 't' };
    expect(isRenderableChatMessage({ ...empty, role: 'user' })).toBe(false);
    expect(isRenderableChatMessage({ ...empty, role: 'system' })).toBe(false);
    expect(isRenderableChatMessage({
      ...empty, role: 'user', images: [{ data: 'a', mimeType: 'image/png' }],
    })).toBe(true);
    expect(isRenderableChatMessage({
      ...empty, role: 'user', context_attached: { probed_memories: [], recalled_memories: [] },
    })).toBe(true);
  });
});
