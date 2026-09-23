import { useCallback, useSyncExternalStore } from 'react';
import { api, extractText, extractSystemNotice, hasToolActivity } from './api';
import type { AppContextPayload, DaemonMessage, SSEEvent } from './api';
import { useCommandCenter } from './store';
import type { LayoutNode, WorkspaceState } from './store';

export type DiscussionStatus = 'idle' | 'creating' | 'ready' | 'sending' | 'error';
export type DiscussionMessage = { id: string; role: 'user' | 'assistant'; text: string; streaming?: boolean };

type Snapshot = {
  status: DiscussionStatus;
  error: string | null;
  messages: DiscussionMessage[];
  draft: string;
  sessionId: string | null;
};
type Discussion = {
  snapshot: Snapshot;
  listeners: Set<() => void>;
  flight: Promise<void> | null;
  source: EventSource | null;
  cursor: string | null;
  connection: Promise<void> | null;
  cancelConnection: (() => void) | null;
  seeded: boolean;
  pending: boolean;
  streamId: string | null;
  paragraphBreak: boolean;
};

// Keep discussion state for the app lifetime; only mounted views own a stream.
const discussions = new Map<string, Discussion>();
let messageSequence = 0;
function discussionFor(id: string): Discussion {
  let discussion = discussions.get(id);
  if (!discussion) {
    discussion = {
      snapshot: { status: 'idle', error: null, messages: [], draft: '', sessionId: null },
      listeners: new Set(), flight: null, source: null, cursor: null,
      connection: null, cancelConnection: null,
      seeded: false, pending: false, streamId: null, paragraphBreak: false,
    };
    discussions.set(id, discussion);
  }
  return discussion;
}
function update(d: Discussion, patch: Partial<Snapshot>) {
  d.snapshot = { ...d.snapshot, ...patch };
  d.listeners.forEach(listener => listener());
}
function fail(d: Discussion, error: unknown) {
  update(d, { status: 'error', error: error instanceof Error ? error.message : String(error) });
}
function finish(d: Discussion) {
  d.pending = false;
  d.streamId = null;
  update(d, {
    status: 'ready', error: null,
    messages: d.snapshot.messages.map(message => ({ ...message, streaming: false })),
  });
}
function receive(d: Discussion, sessionId: string, event: SSEEvent) {
  if (d.snapshot.sessionId !== sessionId) return;
  if (event.type === 'Message' && d.streamId) {
    const message = event.message as DaemonMessage;
    if (message.role === 'assistant') {
      const notice = extractSystemNotice(message);
      const text = extractText(message) || notice?.text || '';
      if (text) {
        update(d, { messages: d.snapshot.messages.map(m => m.id === d.streamId
          ? { ...m, text: m.text + (d.paragraphBreak && m.text ? '\n\n' : '') + text }
          : m) });
        d.paragraphBreak = false;
      }
    }
    if (hasToolActivity(message)) d.paragraphBreak = true;
  } else if (event.type === 'Finish' && (d.pending || d.streamId)) {
    finish(d);
  } else if (event.type === 'Error') {
    d.pending = false;
    d.streamId = null;
    update(d, { messages: d.snapshot.messages.map(m => ({ ...m, streaming: false })) });
    fail(d, event.error);
  }
}

const RECONNECT_DELAYS_MS = [500, 1000, 2000] as const;
class DiscussionDetachedError extends Error {}
class MissingDiscussionSessionError extends Error {
  constructor() {
    super('Discussion session creation returned no session id');
    this.name = 'MissingDiscussionSessionError';
  }
}

/** Reported to the daemon as the overlay the user has open while discussing. */
const DISCUSSION_ACTIVE_PANEL = 'decision-review';

export function discussionAppContext(
  state: { workspaces: Pick<WorkspaceState, 'id' | 'layoutJson'>[]; activeWorkspaceId: string | null },
  decisionId: string,
  seeded: boolean,
): AppContextPayload {
  const primaryToolType = (node: LayoutNode): string | null => {
    if (node.type === 'panel') return node.tool;
    return node.children.length ? primaryToolType(node.children[0]) : null;
  };
  const workspace = state.workspaces.find(w => w.id === state.activeWorkspaceId);
  const tool = workspace ? primaryToolType(workspace.layoutJson) : null;
  // The daemon reads `discuss_decision_id` from this context on the first turn
  // (session_events.rs, "Decision Inbox deep-link"); without it the session
  // never learns which decision this is. So the seed is never dropped, even
  // when no workspace tool can be named — the daemon prints an unknown tab
  // name verbatim, and "dashboard" is where the review is opened from.
  return {
    current_tab: tool ?? 'dashboard',
    active_panel: DISCUSSION_ACTIVE_PANEL,
    ...(seeded ? {} : { view_state: { discuss_decision_id: decisionId } }),
  };
}

function connect(d: Discussion, sessionId: string): Promise<void> {
  if (d.connection) return d.connection;
  if (d.source?.readyState === EventSource.OPEN) return Promise.resolve();
  d.cancelConnection?.();
  let cancelled = false;
  let failures = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const connection = new Promise<void>((resolve, reject) => {
    d.cancelConnection = () => {
      cancelled = true;
      clearTimeout(timer);
      d.source?.close();
      d.source = null;
      d.connection = null;
      reject(new DiscussionDetachedError());
    };
    const attempt = async () => {
      if (cancelled) return;
      let source: EventSource | null = null;
      const connectionFailed = (reason: string) => {
        if (cancelled || (source && d.source !== source)) return;
        clearTimeout(timer);
        source?.close();
        d.source = null;
        if (failures < RECONNECT_DELAYS_MS.length) {
          timer = setTimeout(() => { void attempt(); }, RECONNECT_DELAYS_MS[failures++]);
        } else {
          fail(d, reason);
          reject(new Error(reason));
        }
      };
      try {
        const url = await api.sessionEventsUrl(sessionId, d.cursor);
        if (cancelled) return;
        source = new EventSource(url);
        d.source = source;
        timer = setTimeout(() => connectionFailed('Discussion connection timed out'), 15000);
        source.onerror = (event) => {
          const message = 'message' in event && typeof event.message === 'string' ? event.message : null;
          connectionFailed(message || 'Could not connect to this decision discussion');
        };
        source.onmessage = (frame) => {
          if (cancelled || d.source !== source || d.snapshot.sessionId !== sessionId) return;
          if (frame.lastEventId) d.cursor = frame.lastEventId;
          let event: SSEEvent;
          try { event = JSON.parse(frame.data) as SSEEvent; } catch { return; }
          if (event.type === 'ActiveRequests') {
            clearTimeout(timer);
            failures = 0;
            if (Array.isArray(event.request_ids) && event.request_ids.length === 0 && d.pending) {
              // Keep the placeholder for missed text replayed after this frame.
              d.pending = false;
              update(d, { messages: d.snapshot.messages.map(m => ({ ...m, streaming: false })) });
            }
            update(d, { status: d.pending ? 'sending' : 'ready', error: null });
            resolve();
          }
          receive(d, sessionId, event);
          if (event.type === 'Error') {
            clearTimeout(timer);
            source?.close();
            d.source = null;
            reject(new Error(String(event.error)));
          }
        };
      } catch (error) {
        connectionFailed(error instanceof Error ? error.message : String(error));
      }
    };
    void attempt();
  }).finally(() => { if (d.connection === connection) d.connection = null; });
  d.connection = connection;
  return connection;
}

function send(d: Discussion, decisionId: string, headline: string, retry = false): Promise<void> {
  if (d.flight) return d.flight;
  if (d.snapshot.status === 'sending' || (!retry && !d.snapshot.draft.trim())) return Promise.resolve();
  // Defer work until flight is assigned, so two calls in one event are one send.
  const flight = Promise.resolve().then(async () => {
    const text = d.snapshot.draft;
    update(d, { status: d.snapshot.sessionId ? 'sending' : 'creating', error: null });
    try {
      if (!d.snapshot.sessionId) {
        const session = await api.createSession();
        update(d, { sessionId: session.id });
      }
      const sessionId = d.snapshot.sessionId;
      if (!sessionId) throw new MissingDiscussionSessionError();
      if (!d.listeners.size) throw new DiscussionDetachedError();
      await connect(d, sessionId);
      // A broken stream is reattached, never a reason to submit the same
      // accepted request again. Replay resumes from this session's cursor.
      if (d.pending) return;
      if (!text.trim()) { update(d, { status: 'ready' }); return; }
      const userId = `discussion-${++messageSequence}`;
      const streamId = `discussion-${++messageSequence}`;
      d.streamId = streamId;
      d.pending = true;
      d.paragraphBreak = false;
      update(d, { status: 'sending', messages: [...d.snapshot.messages,
        { id: userId, role: 'user', text },
        { id: streamId, role: 'assistant', text: '', streaming: true },
      ] });
      try {
        // The daemon consumes view_state independently of user_message, so the
        // legacy seed sentence + user's question can share the first turn.
        await api.sendReply(sessionId,
          d.seeded ? text : `I'd like to talk through this decision: "${headline}".\n\n${text}`,
          undefined,
          discussionAppContext(useCommandCenter.getState(), decisionId, d.seeded),
        );
        d.seeded = true;
        update(d, { draft: d.snapshot.draft === text ? '' : d.snapshot.draft });
      } catch (error) {
        d.pending = false;
        d.streamId = null;
        update(d, { messages: d.snapshot.messages.filter(m => m.id !== userId && m.id !== streamId) });
        throw error;
      }
    } catch (error) { if (!(error instanceof DiscussionDetachedError)) fail(d, error); }
  }).finally(() => { if (d.flight === flight) d.flight = null; });
  d.flight = flight;
  return flight;
}

export function useDecisionDiscussion(decisionId: string, headline: string): {
  status: DiscussionStatus;
  error: string | null;
  messages: DiscussionMessage[];
  draft: string;
  setDraft(text: string): void;
  send(): Promise<void>;
  retry(): Promise<void>;
  sessionId: string | null;
} {
  const d = discussionFor(decisionId);
  const subscribe = useCallback((listener: () => void) => {
    d.listeners.add(listener);
    if (d.snapshot.sessionId && !d.source) {
      void connect(d, d.snapshot.sessionId).catch(error => {
        if (!(error instanceof DiscussionDetachedError)) fail(d, error);
      });
    }
    return () => {
      d.listeners.delete(listener);
      if (d.listeners.size === 0) {
        d.cancelConnection?.();
        d.cancelConnection = null;
        d.source?.close();
        d.source = null;
      }
    };
  }, [d]);
  const snapshot = useSyncExternalStore(
    subscribe,
    () => d.snapshot,
    () => d.snapshot,
  );
  return {
    ...snapshot,
    setDraft: text => update(d, { draft: text }),
    send: () => send(d, decisionId, headline),
    retry: () => send(d, decisionId, headline, true),
  };
}
