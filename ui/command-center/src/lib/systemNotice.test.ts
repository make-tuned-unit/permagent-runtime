import { describe, expect, it } from 'vitest';
import { extractSystemNotice, extractText, type DaemonMessage } from './api';
import { isRenderableChatMessage } from '../components/chat/MessageList';
import type { ChatMessage } from './store';

/**
 * The silence the user reported: "there is no response coming from my agent."
 *
 * The daemon was answering every turn — with a `systemNotification` saying both
 * cloud providers were out of credit and the local split was offline. Nothing
 * on the UI side read that block, so the frame carried no text, rendered as
 * nothing, and the agent looked dead instead of unpaid.
 */

const frame = (content: unknown[]): DaemonMessage => ({
  role: 'assistant',
  created: 1758200000,
  content: content as DaemonMessage['content'],
  metadata: { userVisible: true, agentVisible: true },
});

describe('the daemon explaining itself', () => {
  it('reads a credits-exhausted notice that carries no text block', () => {
    const msg = frame([
      { type: 'systemNotification', notificationType: 'creditsExhausted', msg: 'Anthropic credit exhausted.' },
    ]);
    // The old path: no text at all, which is why the turn looked empty.
    expect(extractText(msg)).toBe('');
    expect(extractSystemNotice(msg)).toEqual({
      kind: 'creditsExhausted',
      text: 'Anthropic credit exhausted.',
    });
  });

  it('reads an inline notice', () => {
    const msg = frame([{ type: 'systemNotification', notificationType: 'inlineMessage', msg: 'Local split offline.' }]);
    expect(extractSystemNotice(msg)).toEqual({ kind: 'inline', text: 'Local split offline.' });
  });

  it('ignores transient thinking status, which the indicator already shows', () => {
    const msg = frame([{ type: 'systemNotification', notificationType: 'thinkingMessage', msg: 'Thinking…' }]);
    expect(extractSystemNotice(msg)).toBeNull();
  });

  it('ignores an empty or whitespace notice rather than drawing a blank box', () => {
    expect(extractSystemNotice(frame([{ type: 'systemNotification', notificationType: 'inlineMessage', msg: '   ' }]))).toBeNull();
    expect(extractSystemNotice(frame([{ type: 'systemNotification', notificationType: 'inlineMessage' }]))).toBeNull();
  });

  it('ignores a notification type it does not know instead of guessing', () => {
    const msg = frame([{ type: 'systemNotification', notificationType: 'somethingNew', msg: 'x' }]);
    expect(extractSystemNotice(msg)).toBeNull();
  });

  it('leaves an ordinary reply alone', () => {
    const msg = frame([{ type: 'text', text: 'Here is the answer.' }]);
    expect(extractSystemNotice(msg)).toBeNull();
    expect(extractText(msg)).toBe('Here is the answer.');
  });

  it('carries a notice alongside text when the turn had both', () => {
    const msg = frame([
      { type: 'text', text: 'Partial answer.' },
      { type: 'systemNotification', notificationType: 'creditsExhausted', msg: 'Ran out mid-turn.' },
    ]);
    expect(extractText(msg)).toBe('Partial answer.');
    expect(extractSystemNotice(msg)?.text).toBe('Ran out mid-turn.');
  });
});

describe('a notice survives the render filter', () => {
  const base: ChatMessage = {
    id: 'm1',
    role: 'assistant',
    content: '',
    timestamp: new Date().toISOString(),
  };

  it('draws an assistant turn that produced only a notice', () => {
    // This filter is the second place the explanation was thrown away: a
    // contentless assistant bubble is normally the streaming placeholder.
    expect(isRenderableChatMessage(base)).toBe(false);
    expect(isRenderableChatMessage({ ...base, notice: { kind: 'creditsExhausted', text: 'out of credit' } })).toBe(true);
  });
});
