// Notifications — the agent reaches out (#618).
//
// The honesty boundary for attention: every notification is a REAL daemon
// event from /events (decision_created, goal review, task failure…) — nothing
// synthetic, nothing polled into existence. Preferences are localStorage-
// persisted per kind (the wired-settings pattern the 2026-07-10 audit
// blessed for Appearance) and consumed HERE, so a toggle genuinely silences
// its kind. OS-level notifications are attempted best-effort through the Web
// Notification API when permitted; the in-app tray and toasts are the
// always-works path.

import { useSyncExternalStore } from 'react';
import { eventsWsUrl } from './api';
import { wireEventType } from './wireEvent';
import { navigateToTool, useCommandCenter } from './store';

export interface AppNotification {
  id: string;
  kind: NotificationKind;
  title: string;
  body: string;
  ts: number;
  read: boolean;
  /** Tab to open when clicked. */
  target?: Parameters<typeof navigateToTool>[0];
  /** Optional deep link — opens in the in-app browser (Build tab). */
  url?: string;
  /** Custom click action — wins over url/target when present (e.g. the
   *  "note saved" toast navigating to the exact note). */
  onActivate?: () => void;
}

export type NotificationKind =
  | 'decision'
  | 'goal_review'
  | 'goal_failure'
  | 'librarian'
  | 'initiative'
  | 'echo'
  | 'system';

export const KIND_LABELS: Record<NotificationKind, string> = {
  decision: 'Decisions need you',
  goal_review: 'Goals ready for review',
  goal_failure: 'Goal failures',
  librarian: 'Librarian runs',
  initiative: 'Initiative proposals',
  echo: 'The Watcher',
  system: 'System messages',
};

const DEFAULT_PREFS: Record<NotificationKind, boolean> = {
  decision: true,
  goal_review: true,
  goal_failure: true,
  librarian: false,
  initiative: true,
  echo: true,
  system: true,
};

const PREFS_KEY = 'permagent-notification-prefs';
const OS_KEY = 'permagent-notification-os';

export function getNotificationPrefs(): Record<NotificationKind, boolean> {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (raw) return { ...DEFAULT_PREFS, ...JSON.parse(raw) };
  } catch { /* corrupted prefs fall back to defaults */ }
  return { ...DEFAULT_PREFS };
}

export function setNotificationPref(kind: NotificationKind, on: boolean): void {
  const prefs = { ...getNotificationPrefs(), [kind]: on };
  localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  publish();
}

export function getOsNotificationsEnabled(): boolean {
  return localStorage.getItem(OS_KEY) === '1';
}

export async function setOsNotificationsEnabled(on: boolean): Promise<boolean> {
  if (!on) {
    localStorage.setItem(OS_KEY, '0');
    publish();
    return false;
  }
  try {
    if (typeof Notification !== 'undefined') {
      const perm = await Notification.requestPermission();
      const granted = perm === 'granted';
      localStorage.setItem(OS_KEY, granted ? '1' : '0');
      publish();
      return granted;
    }
  } catch { /* environment without the API — in-app only */ }
  localStorage.setItem(OS_KEY, '0');
  publish();
  return false;
}

// ── Store ────────────────────────────────────────────────────────────────────

interface NotificationState {
  items: AppNotification[];
  unread: number;
  /** Monotonic bump so prefs changes re-render subscribers. */
  rev: number;
}

const MAX_ITEMS = 50;
let state: NotificationState = { items: [], unread: 0, rev: 0 };
const subscribers = new Set<() => void>();

function publish(): void {
  state = { ...state, rev: state.rev + 1 };
  subscribers.forEach((fn) => fn());
}

function push(n: Omit<AppNotification, 'id' | 'ts' | 'read'>): void {
  const item: AppNotification = {
    ...n,
    id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    ts: Date.now(),
    read: false,
  };
  state = {
    ...state,
    items: [item, ...state.items].slice(0, MAX_ITEMS),
    unread: state.unread + 1,
  };
  publish();

  // Best-effort OS notification — the in-app tray is the guaranteed path.
  if (getOsNotificationsEnabled()) {
    try {
      if (typeof Notification !== 'undefined' && Notification.permission === 'granted') {
        const osNote = new Notification(item.title, { body: item.body });
        // Clicking the OS notification was dead for url/target items (only
        // onActivate was wired) — deep-link every shape, same branch order as
        // the tray's activate().
        if (item.onActivate || item.url || item.target) {
          osNote.onclick = () => {
            window.focus();
            if (item.onActivate) {
              item.onActivate();
            } else if (item.url) {
              useCommandCenter.getState().openInBrowser(item.url);
            } else if (item.target) {
              navigateToTool(item.target);
            }
          };
        }
      }
    } catch { /* webview without the plugin — in-app only */ }
  }
}

export function markAllRead(): void {
  state = {
    ...state,
    items: state.items.map((i) => ({ ...i, read: true })),
    unread: 0,
  };
  publish();
}

export function useNotifications(): NotificationState {
  ensureNotificationStream();
  return useSyncExternalStore(
    (fn) => {
      subscribers.add(fn);
      return () => subscribers.delete(fn);
    },
    () => state,
  );
}

// ── Tray open state ──────────────────────────────────────────────────────────
// The bell lives in the Sidebar brand row (next to the logo) while the tray and
// toasts stay mounted at App root, so the two are in different subtrees and
// cannot share React state. A tiny external store keeps them in sync.

let trayOpen = false;
const traySubscribers = new Set<() => void>();

export function setTrayOpen(next: boolean): void {
  if (trayOpen === next) return;
  trayOpen = next;
  traySubscribers.forEach((fn) => fn());
}

export function toggleTray(): void {
  setTrayOpen(!trayOpen);
}

export function useTrayOpen(): boolean {
  return useSyncExternalStore(
    (fn) => {
      traySubscribers.add(fn);
      return () => traySubscribers.delete(fn);
    },
    () => trayOpen,
  );
}

// ── Event stream ─────────────────────────────────────────────────────────────

function titleFor(evt: { payload?: Record<string, unknown> }, fallback: string): string {
  const p = evt.payload ?? {};
  const t = (p.title ?? p.goal_title ?? p.kind ?? '') as string;
  return t ? `${fallback}: ${t}` : fallback;
}

let started = false;
const mountedAt = Date.now();

export function ensureNotificationStream(): void {
  if (started) return;
  started = true;
  connect();
}

async function connect(): Promise<void> {
  try {
    // Daemon token rides the WS query (C1/C2 auth). This stream is started
    // once and retries forever, so no post-await liveness re-check is needed.
    const ws = new WebSocket(await eventsWsUrl());
    ws.onmessage = (msg) => {
      try {
        const evt = JSON.parse(msg.data as string);
        // Skip the replay buffer — only events after mount deserve a ping.
        const ts = Date.parse(evt.timestamp ?? '') || Date.now();
        if (ts < mountedAt - 2000) return;

        const prefs = getNotificationPrefs();
        switch (wireEventType(evt)) {
          // Wave-1 item 4: decision/goal/task notifications consume the
          // ROUTER's verdict (`notification_routed`, channel=in_app), not the
          // raw facts. Before this, the UI re-derived from raw events, so the
          // per-user thresholds and the digest channel silently did nothing —
          // the router computed a routing and nobody read it. Raw-event kinds
          // the router doesn't classify (librarian, echo) keep their own
          // handling below.
          case 'notification_routed': {
            const p = (evt.payload ?? {}) as {
              channel?: string;
              source_type?: string;
              source_payload?: Record<string, unknown>;
            };
            if (p.channel !== 'in_app') break;
            const src = { payload: p.source_payload ?? {} };
            switch (p.source_type) {
              case 'decision_created': {
                const kind = (src.payload.kind ?? '') as string;
                if (kind === 'automation_proposal' || kind === 'initiative') {
                  if (prefs.initiative) {
                    push({
                      kind: 'initiative',
                      title: `${useCommandCenter.getState().agentName} has a proposal`,
                      body: titleFor(src, 'An initiative proposal is waiting in the Decision Inbox'),
                      target: 'dashboard',
                    });
                  }
                } else if (prefs.decision) {
                  push({
                    kind: 'decision',
                    title: 'Decision needed',
                    body: titleFor(src, 'A decision is waiting in the Decision Inbox'),
                    target: 'dashboard',
                  });
                }
                break;
              }
              case 'goal_state_changed': {
                const to = (src.payload.to ?? '') as string;
                if (to === 'review' && prefs.goal_review) {
                  push({
                    kind: 'goal_review',
                    title: 'Goal ready for review',
                    body: titleFor(src, 'A goal finished and wants your eyes'),
                    target: 'dashboard',
                  });
                }
                break;
              }
              case 'task_failed':
              case 'integration_error': {
                if (prefs.goal_failure) {
                  push({
                    kind: 'goal_failure',
                    title: 'Something failed',
                    body: titleFor(src, 'A background task failed'),
                    target: 'dashboard',
                  });
                }
                break;
              }
              default:
                break;
            }
            break;
          }
          case 'librarian_describe_completed': {
            if (prefs.librarian) {
              push({
                kind: 'librarian',
                title: 'Librarian finished a pass',
                body: 'New memory descriptions are in the Brain',
                target: 'memory',
              });
            }
            break;
          }
          case 'proactive_nudge': {
            const p = (evt.payload ?? {}) as {
              kind?: string; message?: string; subject?: string;
              url?: string; link?: string; source_url?: string;
            };
            // Holding overbought signals are a Finance-tab fact, not a Watcher
            // thread. They still arrive as proactive_nudge so the tray/OS path
            // is one, but clicking opens Finance. Copy is a sell *signal*,
            // never an order.
            if (p.kind === 'rsi_heat' || p.kind === 'sell_signal') {
              if (prefs.echo || prefs.system) {
                push({
                  kind: 'echo',
                  title: p.kind === 'sell_signal' ? 'Sell signal' : 'RSI heat',
                  body: p.message ?? `Overbought signal on ${p.subject ?? 'a holding'}`,
                  target: 'finance',
                });
              }
              break;
            }
            if (p.kind === 'daily_pick') {
              if (prefs.echo) {
                const named = Boolean(p.subject) && p.subject !== 'none';
                push({
                  kind: 'echo',
                  title: named ? 'The Financier · tomorrow' : 'The Financier · no pick tomorrow',
                  body: p.message ?? (named ? `${p.subject}` : 'No pick for tomorrow.'),
                  target: 'finance',
                });
              }
              break;
            }
            // Echo / the Watcher (#672): Henry resurfaces a dormant thread (news
            // + analytics later). The daemon owns the gentle once-a-day budget,
            // so anything that arrives here is meant to be seen.
            if (prefs.echo) {
              // If the nudge carries a source link (project news), clicking the
              // notification opens it in the in-app browser on the Build tab.
              // Fallback target by nudge kind: a dormant thread lives in the
              // Brain; project news is about a project.
              push({
                kind: 'echo',
                title: '✦ The Watcher noticed something',
                body: p.message ?? `A thread worth revisiting${p.subject ? `: ${p.subject}` : ''}.`,
                target: p.kind === 'dormant_thread' ? 'memory' : 'projects',
                url: p.url ?? p.link ?? p.source_url,
              });
            }
            break;
          }
          default:
            break;
        }
      } catch { /* non-JSON frame */ }
    };
    ws.onclose = () => setTimeout(connect, 5000);
  } catch {
    setTimeout(connect, 5000);
  }
}

/** Imperative toast for app code (#45's primitive): surfaces in the tray and
 *  as a transient toast, no daemon event required. Honors the Settings
 *  "System messages" preference — before this guard the toggle persisted
 *  prefs.system while every toast ignored it. */
export function toast(title: string, body = '', onActivate?: () => void): void {
  if (!getNotificationPrefs().system) return;
  push({ kind: 'system', title, body, onActivate });
}
