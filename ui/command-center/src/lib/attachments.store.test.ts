/**
 * Durable Chat attachment contract.
 *
 * A sparse screenshot must travel as three complementary forms: a stable
 * server-side attachment reference, any partial local OCR, and the original
 * pixels for provider vision. Losing any one of them recreated the incident
 * where Henry had only an inaccurate one-line OCR summary and could not retry.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  uploadAttachments: vi.fn(),
  sendReply: vi.fn(),
  readerIngest: vi.fn(),
  fileToBase64: vi.fn(),
}));

vi.mock('./api', () => ({
  api: {
    uploadAttachments: mocks.uploadAttachments,
    sendReply: mocks.sendReply,
  },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  hasToolActivity: vi.fn(() => false),
  readerIngest: mocks.readerIngest,
  fileToBase64: mocks.fileToBase64,
}));

import { useCommandCenter } from './store';

beforeEach(() => {
  vi.clearAllMocks();
  mocks.uploadAttachments.mockResolvedValue({
    attachments: [{
      id: 'attachment-42',
      filename: 'story-session.png',
      mime_type: 'image/png',
      size_bytes: 12,
      created_at: '2026-09-04T00:00:00Z',
    }],
  });
  mocks.readerIngest.mockResolvedValue({
    summary: 'partial but useful screenshot text',
    recall_query: '',
    source: 'reader',
    token_count: 6,
    char_count: 35,
    is_visual: true,
    memory_key: 'reader:test',
    already_ingested: false,
  });
  mocks.fileToBase64.mockResolvedValue('base64-pixels');
  mocks.sendReply.mockResolvedValue({ request_id: 'request-7' });
  useCommandCenter.setState({
    chatSessionId: 'session-1',
    chatMessages: [],
    isStreaming: false,
    _activeRequestId: null,
    _streamingMessageId: null,
    workspaces: [],
  });
});

describe('Chat attachment send contract', () => {
  it('sends a durable attachment id, partial OCR, and original pixels together', async () => {
    const screenshot = new File(['fake-pixels'], 'story-session.png', { type: 'image/png' });

    await useCommandCenter.getState().sendMessage('Review this session', [screenshot]);

    expect(mocks.uploadAttachments).toHaveBeenCalledWith('session-1', [screenshot]);
    expect(mocks.sendReply).toHaveBeenCalledOnce();
    const [sessionId, text, images, , attachmentIds] = mocks.sendReply.mock.calls[0];
    expect(sessionId).toBe('session-1');
    expect(text).toContain('[attachment:attachment-42]');
    expect(text).toContain('partial but useful screenshot text');
    expect(images).toEqual([{ data: 'base64-pixels', mime_type: 'image/png' }]);
    expect(attachmentIds).toEqual(['attachment-42']);
  });

  it('keeps an explicit visual fallback when OCR returns no text', async () => {
    mocks.readerIngest.mockResolvedValueOnce({
      summary: '',
      recall_query: '',
      source: 'reader',
      token_count: 0,
      char_count: 0,
      is_visual: true,
      memory_key: 'reader:empty',
      already_ingested: false,
    });
    const screenshot = new File(['fake-pixels'], 'story-session.png', { type: 'image/png' });

    await useCommandCenter.getState().sendMessage('(file upload)', [screenshot]);

    const [, text] = mocks.sendReply.mock.calls[0];
    expect(text).toContain('[attachment:attachment-42]');
    expect(text).toContain('no readable text detected');
  });

  it('fails open but states plainly when durable upload did not happen', async () => {
    mocks.uploadAttachments.mockRejectedValueOnce(new Error('older daemon'));
    const screenshot = new File(['fake-pixels'], 'story-session.png', { type: 'image/png' });

    await useCommandCenter.getState().sendMessage('Review this session', [screenshot]);

    const [, text, images, , attachmentIds] = mocks.sendReply.mock.calls[0];
    expect(text).toContain('Durable attachment upload failed');
    expect(text).not.toContain('[attachment:attachment-42]');
    expect(images).toEqual([{ data: 'base64-pixels', mime_type: 'image/png' }]);
    expect(attachmentIds).toEqual([]);
  });
});
