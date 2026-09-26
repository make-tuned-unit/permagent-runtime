/**
 * Listens for the daemon's `dashboard_layout_preview` event — emitted by the
 * agent's `preview_dashboard_layout` tool (platform_extensions/dashboard.rs)
 * — and hands the proposed layout to whichever component wants to show the
 * current-vs-proposed overlay (`LayoutPreviewOverlay`). Mirrors
 * `notifications.ts`'s own dedicated `/events` connection: this app already
 * opens one such socket per concern rather than sharing a single dispatcher,
 * so a second small one here matches the existing pattern rather than adding
 * a third shape.
 *
 * The daemon NEVER persists on this event (P9) — it is purely "here's what I
 * would show you"; only `apply_dashboard_layout` (a separate tool call) or the
 * gallery's own Apply button writes `dashboard.json`.
 */
import { useSyncExternalStore } from 'react';
import { eventsWsUrl } from '../../lib/api';
import { wireEventType } from '../../lib/wireEvent';
import type { DashboardLayoutData } from './useLayout';

export interface DashboardLayoutPreviewPayload {
  templateId: string | null;
  layout: DashboardLayoutData;
  reason: string;
  /** Bumped on every new event so an identical re-preview still re-opens the
   *  overlay after the user dismissed it. */
  seq: number;
}

let current: DashboardLayoutPreviewPayload | null = null;
let seq = 0;
const subscribers = new Set<() => void>();
let started = false;
const mountedAt = Date.now();

function publish(next: DashboardLayoutPreviewPayload | null): void {
  current = next;
  subscribers.forEach(fn => fn());
}

async function connect(): Promise<void> {
  try {
    const ws = new WebSocket(await eventsWsUrl());
    ws.onmessage = (msg) => {
      try {
        const evt = JSON.parse(msg.data as string);
        const ts = Date.parse(evt.timestamp ?? '') || Date.now();
        if (ts < mountedAt - 2000) return; // skip the replay buffer
        if (wireEventType(evt) !== 'dashboard_layout_preview') return;
        const p = (evt.payload ?? {}) as { template_id?: string | null; layout?: DashboardLayoutData; reason?: string };
        if (!p.layout) return;
        publish({ templateId: p.template_id ?? null, layout: p.layout, reason: p.reason ?? '', seq: ++seq });
      } catch { /* malformed frame — ignore */ }
    };
    ws.onclose = () => { setTimeout(connect, 3000); };
    ws.onerror = () => { ws.close(); };
  } catch {
    setTimeout(connect, 3000);
  }
}

function ensureStarted(): void {
  if (started) return;
  started = true;
  connect();
}

export function useDashboardLayoutPreviewEvent(): DashboardLayoutPreviewPayload | null {
  ensureStarted();
  return useSyncExternalStore(
    (fn) => { subscribers.add(fn); return () => subscribers.delete(fn); },
    () => current,
  );
}

export function clearDashboardLayoutPreview(): void {
  publish(null);
}
