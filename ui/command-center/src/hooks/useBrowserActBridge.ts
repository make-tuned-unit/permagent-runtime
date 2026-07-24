import { useEffect, useRef } from 'react';
import { eventsWsUrl, apiFetch } from '../lib/api';
import { wireEventType } from '../lib/wireEvent';

/**
 * Act-on-page bridge (#649 / #622) — the acting counterpart to
 * useBrowserContentBridge. Subscribes to the daemon's /events bus and, when the
 * agent asks to snapshot or act on the open page, injects the grounding script
 * into the active browser webview (Tauri commands `get_page_snapshot` /
 * `act_on_ref`) and POSTs the result back to the awaiting daemon route.
 *
 * A second /events socket alongside the content bridge is intentional: the bus
 * broadcasts to every subscriber, so each bridge stays a small, independent unit
 * and the proven read path is left untouched. Reconnects on WebSocket drop.
 */
export function useBrowserActBridge(activeWebviewId: string | null | undefined) {
  const webviewIdRef = useRef(activeWebviewId);
  webviewIdRef.current = activeWebviewId;

  useEffect(() => {
    let ws: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let disposed = false;

    async function connect() {
      if (disposed) return;
      // Daemon token rides the WS query (C1/C2 auth); re-check `disposed`
      // after the await so a token load racing unmount never opens a socket.
      const url = await eventsWsUrl();
      if (disposed) return;
      ws = new WebSocket(url);

      ws.onmessage = async (ev) => {
        try {
          const event = JSON.parse(ev.data);
          const eventType = wireEventType(event);

          if (eventType === 'browser_snapshot_requested') {
            await handleSnapshot(event.payload?.request_id, webviewIdRef.current);
            return;
          }
          if (eventType === 'browser_act_requested') {
            await handleAct(event.payload, webviewIdRef.current);
            return;
          }
        } catch {
          // Ignore malformed events.
        }
      };

      ws.onclose = () => {
        if (!disposed) {
          reconnectTimer = setTimeout(connect, 3000);
        }
      };

      ws.onerror = () => {
        ws?.close();
      };
    }

    connect();

    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      ws?.close();
    };
  }, []);
}

interface SnapshotResult {
  url: string;
  webview_id?: string;
  elements: Array<{ ref: number; role: string; name: string; tag: string; value?: string }>;
  truncated: boolean;
  status: string;
}

interface ActResult {
  ok: boolean;
  error?: string;
  snapshot?: SnapshotResult;
}

async function handleSnapshot(requestId: unknown, wvId: string | null | undefined) {
  if (typeof requestId !== 'string' || !requestId) return;

  if (!wvId) {
    await fulfillSnapshot(requestId, {
      url: '',
      elements: [],
      truncated: false,
      status: 'no_tab',
    });
    return;
  }

  try {
    const core = await import('@tauri-apps/api/core');
    const result = (await core.invoke('get_page_snapshot', { webviewId: wvId })) as SnapshotResult;
    await fulfillSnapshot(requestId, { ...result, webview_id: wvId });
  } catch (err) {
    await fulfillSnapshot(requestId, {
      url: '',
      elements: [],
      truncated: false,
      status: 'error',
    });
    void err;
  }
}

async function handleAct(payload: unknown, _activeWebviewId: string | null | undefined) {
  const p = (payload ?? {}) as {
    request_id?: unknown;
    ref?: unknown;
    action?: unknown;
    value?: unknown;
    webview_id?: unknown;
    page_url?: unknown;
  };
  const requestId = p.request_id;
  if (typeof requestId !== 'string' || !requestId) return;

  const binding = resolveActBinding(p, _activeWebviewId);
  if (!binding) {
    await fulfillAct(requestId, {
      ok: false,
      error: 'The browser snapshot identity is missing. Take a fresh snapshot before acting.',
    });
    return;
  }

  try {
    const core = await import('@tauri-apps/api/core');
    const result = (await core.invoke('act_on_ref', {
      webviewId: binding.webviewId,
      expectedUrl: binding.pageUrl,
      refId: p.ref,
      action: p.action,
      value: p.value ?? null,
    })) as ActResult;
    if (result.snapshot) result.snapshot.webview_id = binding.webviewId;
    await fulfillAct(requestId, result);
  } catch (err) {
    await fulfillAct(requestId, { ok: false, error: `Act failed: ${err}` });
  }
}

export function resolveActBinding(
  payload: { webview_id?: unknown; page_url?: unknown },
  _activeWebviewId: string | null | undefined,
): { webviewId: string; pageUrl: string } | null {
  if (
    typeof payload.webview_id !== 'string' ||
    !payload.webview_id ||
    typeof payload.page_url !== 'string' ||
    !payload.page_url
  ) {
    return null;
  }
  return { webviewId: payload.webview_id, pageUrl: payload.page_url };
}

async function fulfillSnapshot(requestId: string, snapshot: SnapshotResult) {
  await apiFetch<unknown>(`/api/browser/snapshot/${requestId}`, {
    method: 'POST',
    body: JSON.stringify(snapshot),
  }).catch(() => {});
}

async function fulfillAct(requestId: string, result: ActResult) {
  await apiFetch<unknown>(`/api/browser/act/${requestId}`, {
    method: 'POST',
    body: JSON.stringify(result),
  }).catch(() => {});
}
