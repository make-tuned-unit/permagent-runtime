/**
 * Streaming segment breaks + the empty placeholder bubble — regression tests.
 *
 * The two chat defects reported 2026-08-06:
 *  1. Streamed text glued segments across tool activity ("…works.Let me dig
 *     deeper…") and then SNAPPED into shape when the Finish rehydrate swapped
 *     in the settled transcript. Live rendering must match the settled join.
 *  2. The streaming placeholder rendered as a bare "Henry 10:46 AM" bubble
 *     above the StreamingIndicator — two bubbles before the first token.
 *
 * `./api` keeps its REAL extractText/extractThinking/hasToolActivity (they are
 * part of the behavior under test); only the network-touching exports are
 * stubbed.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';
import type { DaemonMessage, SSEEvent } from './api';

vi.mock('./api', async (importOriginal) => {
  const real = await importOriginal<typeof import('./api')>();
  return {
    ...real,
    api: { cancelReply: vi.fn(), sendReply: vi.fn() },
    apiFetch: vi.fn(),
    fileToBase64: vi.fn(),
    readerIngest: vi.fn(),
  };
});

import { extractText, hasToolActivity } from './api';
import { useCommandCenter, type ChatMessage } from './store';
import { isRenderableChatMessage } from '../components/chat/MessageList';

const STREAM_ID = 'msg-test-stream';

function assistantText(text: string): SSEEvent {
  return {
    type: 'Message',
    message: {
      role: 'assistant',
      created: 0,
      content: [{ type: 'text', text }],
    } as unknown as DaemonMessage,
  } as unknown as SSEEvent;
}

function assistantThinking(thinking: string): SSEEvent {
  return {
    type: 'Message',
    message: {
      role: 'assistant',
      created: 0,
      content: [{ type: 'thinking', thinking }],
    } as unknown as DaemonMessage,
  } as unknown as SSEEvent;
}

function toolFrame(role: 'assistant' | 'user', kind: 'toolRequest' | 'toolResponse'): SSEEvent {
  return {
    type: 'Message',
    message: {
      role,
      created: 0,
      content: [{ type: kind, id: 't1' }],
    } as unknown as DaemonMessage,
  } as unknown as SSEEvent;
}

function armStreaming() {
  useCommandCenter.setState({
    isStreaming: true,
    _streamingMessageId: STREAM_ID,
    _textBreakPending: false,
    _thinkingBreakPending: false,
    chatMessages: [{
      id: STREAM_ID,
      role: 'assistant',
      content: '',
      timestamp: new Date().toISOString(),
    }],
  });
}

function streamed(): ChatMessage {
  return useCommandCenter.getState().chatMessages.find(m => m.id === STREAM_ID)!;
}

beforeEach(() => {
  armStreaming();
});

describe('segment breaks in the live stream', () => {
  it('inserts a paragraph break across tool activity, never gluing segments', () => {
    const handle = useCommandCenter.getState().handleSessionEvent;
    handle(assistantText('compare it to how the harness works.'));
    handle(toolFrame('assistant', 'toolRequest'));
    handle(assistantText('Let me dig deeper'));
    expect(streamed().content).toBe('compare it to how the harness works.\n\nLet me dig deeper');
  });

  it('keeps plain token deltas exactly concatenated within a segment', () => {
    const handle = useCommandCenter.getState().handleSessionEvent;
    handle(assistantText('I need'));
    handle(assistantText(' to see'));
    handle(assistantText(" what you're looking at."));
    expect(streamed().content).toBe("I need to see what you're looking at.");
  });

  it('a user-role tool result also ends the segment', () => {
    const handle = useCommandCenter.getState().handleSessionEvent;
    handle(assistantText('first segment.'));
    handle(toolFrame('user', 'toolResponse'));
    handle(assistantText('Second segment.'));
    expect(streamed().content).toBe('first segment.\n\nSecond segment.');
  });

  it('a thinking-only delta does not consume the break owed to the next text', () => {
    const handle = useCommandCenter.getState().handleSessionEvent;
    handle(assistantText('first segment.'));
    handle(toolFrame('assistant', 'toolRequest'));
    handle(assistantThinking('let me check something'));
    handle(assistantText('Second segment.'));
    expect(streamed().content).toBe('first segment.\n\nSecond segment.');
    expect(streamed().thinking).toBe('let me check something');
  });

  it('no leading break before the first text of the turn', () => {
    const handle = useCommandCenter.getState().handleSessionEvent;
    handle(toolFrame('assistant', 'toolRequest'));
    handle(assistantText('hello'));
    expect(streamed().content).toBe('hello');
  });
});

describe('settled-transcript joins match the live stream', () => {
  it('extractText joins distinct stored text blocks with a paragraph break', () => {
    const msg = {
      role: 'assistant',
      created: 0,
      content: [
        { type: 'text', text: 'harness works.' },
        { type: 'toolRequest', id: 't1' },
        { type: 'text', text: 'Let me dig deeper' },
      ],
    } as unknown as DaemonMessage;
    expect(extractText(msg)).toBe('harness works.\n\nLet me dig deeper');
    expect(hasToolActivity(msg)).toBe(true);
  });
});

describe('the empty placeholder bubble', () => {
  const base = { id: 'm1', role: 'assistant' as const, content: '', timestamp: 't' };

  it('hides a contentless assistant placeholder (the double-bubble)', () => {
    expect(isRenderableChatMessage(base as ChatMessage)).toBe(false);
    expect(isRenderableChatMessage({ ...base, content: '  ' } as ChatMessage)).toBe(false);
  });

  it('shows it the moment there is anything to read', () => {
    expect(isRenderableChatMessage({ ...base, content: 'hi' } as ChatMessage)).toBe(true);
    expect(isRenderableChatMessage({ ...base, thinking: 'hm' } as ChatMessage)).toBe(true);
    expect(isRenderableChatMessage({
      ...base, tool_calls: [{} as never],
    } as ChatMessage)).toBe(true);
  });

  it('never hides user messages', () => {
    expect(isRenderableChatMessage({ ...base, role: 'user', content: '' } as ChatMessage)).toBe(true);
  });
});
