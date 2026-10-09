import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const fetchMock = vi.fn();

function streamResponse(...chunks: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      controller.close();
    },
  }), { status: 200 });
}

function openStreamResponse(chunk: string, onCancel: () => void): Response {
  return new Response(new ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(new TextEncoder().encode(chunk)); },
    cancel: onCancel,
  }), { status: 200 });
}

async function client() {
  vi.resetModules();
  const storage = new Map([['permagent-daemon-token', 'test-token']]);
  vi.stubGlobal('window', { location: { hash: '', pathname: '/', search: '' } });
  vi.stubGlobal('localStorage', { getItem: (key: string) => storage.get(key) ?? null });
  vi.stubGlobal('fetch', fetchMock);
  return (await import('./api')).api;
}

beforeEach(() => { fetchMock.mockReset(); });
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

it('forwards progress and completes a clean Ollama pull stream', async () => {
  fetchMock.mockResolvedValue(streamResponse(
    'data: {"status":"pulling","total":100,"completed":25}\n\n',
    'data: {"status":"success"}\n\n',
  ));
  const api = await client();
  const progress = vi.fn();
  await expect(api.pullOllamaModel('qwen3:8b', progress).promise).resolves.toBeUndefined();
  expect(progress).toHaveBeenCalledTimes(2);
  expect(progress).toHaveBeenLastCalledWith({ status: 'success' });
});

it('rejects the daemon SSE error event with its message', async () => {
  const cancel = vi.fn();
  fetchMock.mockResolvedValue(openStreamResponse(
    'event: error\ndata: {"error":"connection lost while pulling"}\n\n', cancel,
  ));
  const api = await client();
  const progress = vi.fn();
  await expect(api.pullOllamaModel('qwen3:8b', progress).promise).rejects.toThrow('connection lost while pulling');
  expect(progress).not.toHaveBeenCalled();
  expect(cancel).toHaveBeenCalledOnce();
});

it('rejects an Ollama application error carried in an ordinary data frame', async () => {
  fetchMock.mockResolvedValue(streamResponse('data: {"error":"model not found"}\n\n'));
  const api = await client();
  await expect(api.pullOllamaModel('missing', vi.fn()).promise).rejects.toThrow('model not found');
});

it('does not swallow a progress callback failure as malformed JSON', async () => {
  const cancel = vi.fn();
  fetchMock.mockResolvedValue(openStreamResponse('data: {"status":"pulling"}\n\n', cancel));
  const api = await client();
  await expect(api.pullOllamaModel('qwen3:8b', () => { throw new Error('progress handler failed'); }).promise)
    .rejects.toThrow('progress handler failed');
  expect(cancel).toHaveBeenCalledOnce();
});
