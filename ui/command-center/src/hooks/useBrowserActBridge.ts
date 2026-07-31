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
export function useBrowserActBridge(
  activeWebviewId: string | null | undefined,
  /** Every webview THIS client owns (all its tabs). The act event fans out to
   *  every connected client, so ownership — not focus — decides who performs
   *  it: an act still targets the snapshot's webview after the user switches
   *  tabs, but only the client holding that webview runs it (#939). */
  ownedWebviewIds: ReadonlyArray<string | null> = [],
) {
  const webviewIdRef = useRef(activeWebviewId);
  webviewIdRef.current = activeWebviewId;
  const ownedRef = useRef(ownedWebviewIds);
  ownedRef.current = ownedWebviewIds;

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
            await handleAct(event.payload, ownedRef.current);
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
  /** Generation these refs were stamped in — presented back on act (#939). */
  generation?: string;
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

async function handleAct(payload: unknown, ownedWebviewIds: ReadonlyArray<string | null>) {
  const p = (payload ?? {}) as {
    request_id?: unknown;
    ref?: unknown;
    action?: unknown;
    value?: unknown;
    webview_id?: unknown;
    page_url?: unknown;
    generation?: unknown;
  };
  const requestId = p.request_id;
  if (typeof requestId !== 'string' || !requestId) return;

  const binding = resolveActBinding(p, ownedWebviewIds);

  // Another client owns this webview — stay SILENT. Answering (even with an
  // error) would race the owner's real result through the same one-shot slot.
  if (binding.kind === 'ignore') return;

  if (binding.kind === 'unbound') {
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
      expectedGeneration: binding.generation,
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

export type ActBinding =
  | { kind: 'act'; webviewId: string; pageUrl: string; generation: string | null }
  /** A different client's webview owns this act — do nothing at all. */
  | { kind: 'ignore' }
  /** Malformed/missing identity — answer with an error so the agent is told. */
  | { kind: 'unbound' };

/**
 * Decide this client's role for an act event.
 *
 * The act is broadcast on `/events` to EVERY connected command-center client,
 * and each used to run it independently — so with two windows open, one
 * `act_on_page` executed TWICE in the webview. The second `fulfill` 404s, which
 * made it invisible to the agent while a non-idempotent action (submit payment,
 * confirm delete) had already double-fired (#939).
 *
 * The gate is OWNERSHIP, not focus. An act targets the webview the snapshot was
 * taken in — deliberately, so it still lands after the user switches tabs — so
 * the client that performs it is the one holding that webview among its tabs,
 * whether or not it is the active tab.
 */
export function resolveActBinding(
  payload: { webview_id?: unknown; page_url?: unknown; generation?: unknown },
  ownedWebviewIds: ReadonlyArray<string | null>,
): ActBinding {
  if (
    typeof payload.webview_id !== 'string' ||
    !payload.webview_id ||
    typeof payload.page_url !== 'string' ||
    !payload.page_url
  ) {
    return { kind: 'unbound' };
  }
  // Not my webview, not my act. Silence rather than an error — another client
  // IS the owner and its answer is the real one; answering would race it.
  if (!ownedWebviewIds.includes(payload.webview_id)) {
    return { kind: 'ignore' };
  }
  return {
    kind: 'act',
    webviewId: payload.webview_id,
    pageUrl: payload.page_url,
    generation: typeof payload.generation === 'string' ? payload.generation : null,
  };
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
