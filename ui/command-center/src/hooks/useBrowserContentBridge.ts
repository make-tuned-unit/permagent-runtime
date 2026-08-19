import { useEffect, useRef } from 'react';
import { useCommandCenter } from '../lib/store';
import { eventsWsUrl, apiFetch } from '../lib/api';
import { wireEventType } from '../lib/wireEvent';
import {
  answerForCapture,
  answerForFailedCapture,
  pageContentAnswer,
  type InboxCapture,
  type PageContentResult,
} from '../lib/pageContentAnswer';

/**
 * Subscribes to the daemon's global event bus (/events WebSocket).
 * When a BrowserContentRequested event arrives, extracts page content
 * from the active browser webview and POSTs it back to the daemon.
 *
 * Reconnects automatically on WebSocket drop (daemon restart, network blip).
 */
export function useBrowserContentBridge(activeWebviewId: string | null | undefined) {
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

          // #567: the agent asked the in-app browser to open a URL.
          if (eventType === 'browser_navigate_requested') {
            const url = event.payload?.url;
            if (typeof url === 'string' && /^https?:\/\//.test(url)) {
              useCommandCenter.getState().openInBrowser(url);
            }
            return;
          }

          if (eventType !== 'browser_content_requested') return;

          const requestId = event.payload?.request_id;
          if (!requestId) return;

          const wvId = webviewIdRef.current;

          // Failure mode 1: no browser tab open
          if (!wvId) {
            await fulfill(requestId, {
              title: '',
              url: '',
              content: 'No browser tab is currently open in Permagent.',
              status: 'no_tab',
              truncated: false,
            });
            return;
          }

          // Attempt content extraction via Tauri command
          try {
            const core = await import('@tauri-apps/api/core');
            const result = (await core.invoke('get_page_content', {
              webviewId: wvId,
            })) as PageContentResult;

            const answer = pageContentAnswer(result);

            // Failure mode 2 used to live here: an empty extraction became "the
            // page appears to be blank or still loading". That is right for an
            // HTML page and wrong for a PDF, where the document is present and
            // simply has no DOM. `pageContentAnswer` tells the two apart.
            if (!answer.capture) {
              await fulfill(requestId, answer.reply);
              return;
            }

            // A native-viewer tab: capture the FILE, and answer with its path.
            // Permagent can read a file; it can do nothing with an empty body.
            try {
              const capture = (await core.invoke('save_tab_to_inbox', {
                webviewId: wvId,
                projectId: null,
                expectDocument: true,
              })) as InboxCapture;
              await fulfill(requestId, answerForCapture(result, capture));
            } catch (captureErr) {
              await fulfill(requestId, answerForFailedCapture(result, String(captureErr)));
            }
            return;
          } catch (err) {
            // Failure mode 3: JS eval or Tauri command failed
            await fulfill(requestId, {
              title: '',
              url: '',
              content: `Failed to extract page content: ${err}`,
              status: 'error',
              truncated: false,
            });
          }
        } catch {
          // Ignore malformed events
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

async function fulfill(
  requestId: string,
  content: {
    title: string;
    url: string;
    content: string;
    status: string;
    truncated: boolean;
  },
) {
  await apiFetch<unknown>(`/api/browser/content/${requestId}`, {
    method: 'POST',
    body: JSON.stringify(content),
  }).catch(() => {});
}
