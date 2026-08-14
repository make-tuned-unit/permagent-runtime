/**
 * `trackEvent()` — the client half of Permagent first-party analytics.
 *
 * The ingest side already exists (`POST /collect/{site_key}` in the daemon, or
 * the same-origin relay the install brief has each site build). What did not
 * exist is the layer between an app calling `trackEvent('signup')` and a row
 * landing in `analytics_events`: batching, retry, and a flush that survives the
 * tab closing. Every project that hand-rolled that got it subtly wrong in a
 * different way, which is why this is a shared package rather than a file
 * inside one app.
 *
 * Three rules the implementation is built around:
 *
 *   NEVER SURFACE AN ERROR. `trackEvent` returns void, is never awaited, and
 *   swallows everything — a broken analytics endpoint must not break a signup
 *   form. No `console.error` unless `debug: true` is explicitly passed.
 *
 *   NEVER LOSE AN EVENT SILENTLY. Anything discarded — retries exhausted, a
 *   4xx rejection, queue overflow — is COUNTED in `stats()`, reported to
 *   `onDrop`, persisted across reloads, and (by default) reported back into
 *   analytics itself as a `permagent_client_dropped` event. An analytics client
 *   that loses events quietly makes every downstream number wrong in a way
 *   nobody can see.
 *
 *   ONE EVENT PER REQUEST. "Batching" here means grouping the *flush* — the
 *   queue drains on one timer, as one retry unit — not a multi-event body. The
 *   deployed contract (daemon collector and every relay built from the install
 *   brief) is one JSON object per POST with a 2 KiB cap; sending an array would
 *   be rejected by relays that are already live. A consumer whose relay accepts
 *   batches can supply its own `transport`.
 *
 * Works in a browser (sendBeacon on page hide, fetch otherwise) and in Node 18+
 * (fetch, no DOM access, timers unref'd so a process can still exit).
 */

/** Flat scalars only — the collector drops nested values because they cannot
 *  be grouped by. Enforced here so the drop happens where it is visible. */
export type EventProperties = Record<string, string | number | boolean | null | undefined>;

/** The wire body the collector accepts. Deliberately terse: it rides in a
 *  `sendBeacon` payload. */
export interface AnalyticsBeacon {
  /** 'pv' = pageview, 'ev' = named event. */
  k: 'pv' | 'ev';
  /** Path *with* query — the server strips the query for storage and pulls
   *  utm_* from an allowlist. */
  p: string;
  r: string | null;
  n: string | null;
  d: EventProperties | null;
  s: string | null;
}

export type DropReason =
  /** Retries exhausted against a transient failure (network down, 5xx). */
  | 'retries_exhausted'
  /** The endpoint rejected the event outright (4xx) — retrying cannot help. */
  | 'rejected'
  /** The queue hit `maxQueueEvents`; the oldest events were discarded. */
  | 'queue_overflow'
  /** Counted on a previous page load and carried across the reload. */
  | 'previous_page';

export interface TrackerStats {
  /** Events waiting to be sent (including ones awaiting a retry). */
  queued: number;
  /** Events the endpoint accepted. */
  sent: number;
  /** Events given up on. The number that makes every other number honest. */
  dropped: number;
  /** Drops broken out by cause. */
  droppedByReason: Record<DropReason, number>;
  /** Individual send attempts that failed and were retried. */
  retries: number;
  /** Last transport failure, for a debug panel. Never shown to end users. */
  lastError: string | null;
}

/** Result of one delivery attempt. `retryable` is the whole point: a 400 will
 *  still be a 400 in eight seconds, so retrying it only delays the drop and
 *  hides the real problem. */
export interface SendResult {
  ok: boolean;
  retryable?: boolean;
  status?: number;
  error?: string;
}

export type Transport = (
  endpoint: string,
  body: string,
  opts: { unloading: boolean },
) => Promise<SendResult>;

export interface TrackerOptions {
  /** Absolute or same-origin collector URL, e.g. `/api/pa/collect` or
   *  `https://hub.example/collect/<site_key>`. */
  endpoint: string;
  /** Flush once this many events are queued. Default 10. */
  batchSize?: number;
  /** …or this long after the first queued event, whichever comes first.
   *  Default 5000 ms. */
  flushIntervalMs?: number;
  /** Delivery attempts per event before it is dropped and counted. Default 5. */
  maxAttempts?: number;
  /** First retry delay; doubles each attempt, ±25% jitter. Default 1000 ms. */
  retryBaseMs?: number;
  /** Ceiling for the backoff. Default 30000 ms. */
  retryMaxMs?: number;
  /** Hard bound on memory. Past this the OLDEST events are dropped (and
   *  counted) — the alternative is an unbounded queue on a dead endpoint.
   *  Default 500. */
  maxQueueEvents?: number;
  /** Session id sent as `s`. Defaults to a sessionStorage-backed id in the
   *  browser (no cookie, never cross-site) and null in Node. */
  sessionId?: string | null;
  /** Report drops back into analytics as `permagent_client_dropped`. Default
   *  true: a drop counter nobody queries is not observability. */
  reportDrops?: boolean;
  /** Called for every discarded event. Never called with a throw. */
  onDrop?: (events: AnalyticsBeacon[], reason: DropReason) => void;
  /** Swap the network layer (tests, a batch-capable relay, a native shell). */
  transport?: Transport;
  /** Log transport failures to the console. Off by default. */
  debug?: boolean;
}

export interface Tracker {
  /** Fire-and-forget. Returns immediately; never throws. */
  trackEvent(name: string, properties?: EventProperties): void;
  /** Fire-and-forget pageview. Path defaults to the browser's location. */
  trackPageview(path?: string, referrer?: string | null): void;
  /** Force a flush. Resolves when the queue has been attempted once. */
  flush(): Promise<void>;
  stats(): TrackerStats;
  /** Detach listeners and timers, flushing what is queued. */
  shutdown(): Promise<void>;
}

const DROP_EVENT_NAME = 'permagent_client_dropped';
const SESSION_KEY = '_pa_sid';
const DROP_COUNTER_KEY = '_pa_dropped';
/** Properties are capped client-side too, so an oversized payload is truncated
 *  where the developer can see it rather than silently rejected at the door. */
const MAX_PROPERTY_KEYS = 32;
const MAX_PROPERTY_CHARS = 256;
const MAX_NAME_CHARS = 128;

interface QueueEntry {
  beacon: AnalyticsBeacon;
  attempts: number;
  /** Drop reports must never generate drop reports — that is an infinite loop
   *  against a dead endpoint. */
  isDropReport: boolean;
}

function isBrowser(): boolean {
  return typeof window !== 'undefined' && typeof document !== 'undefined';
}

/** Every storage read is wrapped: Safari private mode throws on access, and an
 *  analytics client that throws in a constructor takes the host app with it. */
function readStorage(key: string): string | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string): void {
  try {
    if (typeof localStorage !== 'undefined') localStorage.setItem(key, value);
  } catch {
    /* private mode: the counter degrades to in-memory only */
  }
}

function defaultSessionId(): string | null {
  if (!isBrowser()) return null;
  try {
    let id = sessionStorage.getItem(SESSION_KEY);
    if (!id) {
      id = Math.random().toString(36).slice(2) + Date.now().toString(36);
      sessionStorage.setItem(SESSION_KEY, id);
    }
    return id;
  } catch {
    return null;
  }
}

/** Truncate rather than reject: a rejected beacon is a lost event, and a
 *  too-long value is still worth having. */
export function sanitizeProperties(props?: EventProperties | null): EventProperties | null {
  if (!props || typeof props !== 'object') return null;
  const out: EventProperties = {};
  let keys = 0;
  for (const [key, value] of Object.entries(props)) {
    if (keys >= MAX_PROPERTY_KEYS) break;
    if (value === null || value === undefined) continue;
    const t = typeof value;
    if (t === 'string') {
      out[key] = (value as string).slice(0, MAX_PROPERTY_CHARS);
    } else if (t === 'number') {
      if (!Number.isFinite(value as number)) continue;
      out[key] = value as number;
    } else if (t === 'boolean') {
      out[key] = value as boolean;
    } else {
      // Objects and arrays cannot be grouped by, so the collector drops them.
      continue;
    }
    keys += 1;
  }
  return Object.keys(out).length > 0 ? out : null;
}

/** HTTP status → whether another attempt could plausibly succeed. */
export function isRetryableStatus(status: number): boolean {
  if (status === 408 || status === 425 || status === 429) return true;
  return status >= 500;
}

/** Exponential backoff with ±25% jitter, so a fleet of tabs coming back from
 *  an outage does not retry in lockstep. */
export function backoffDelay(
  attempt: number,
  baseMs: number,
  maxMs: number,
  random: () => number = Math.random,
): number {
  const raw = Math.min(maxMs, baseMs * Math.pow(2, Math.max(0, attempt - 1)));
  const jitter = 1 + (random() - 0.5) * 0.5;
  return Math.max(0, Math.round(raw * jitter));
}

const defaultTransport: Transport = async (endpoint, body, opts) => {
  // On page hide the document is being torn down: sendBeacon is the only
  // mechanism the browser guarantees to complete. It reports queueing success,
  // not delivery — accepted here as the best available, since the alternative
  // is losing the batch outright.
  if (opts.unloading && isBrowser() && typeof navigator !== 'undefined' && navigator.sendBeacon) {
    try {
      const queued = navigator.sendBeacon(endpoint, body);
      if (queued) return { ok: true };
    } catch {
      /* fall through to fetch */
    }
  }
  if (typeof fetch !== 'function') {
    return { ok: false, retryable: true, error: 'no fetch available' };
  }
  try {
    const res = await fetch(endpoint, {
      method: 'POST',
      // text/plain keeps this a CORS "simple request" — no preflight, and it
      // matches what sendBeacon sends, so one collector handles both.
      headers: { 'Content-Type': 'text/plain;charset=UTF-8' },
      body,
      keepalive: opts.unloading,
      credentials: 'omit',
    });
    if (res.ok) return { ok: true, status: res.status };
    return {
      ok: false,
      status: res.status,
      retryable: isRetryableStatus(res.status),
      error: `HTTP ${res.status}`,
    };
  } catch (err) {
    // Network error / offline / CORS — transient by nature.
    return { ok: false, retryable: true, error: String(err) };
  }
};

export function createTracker(options: TrackerOptions): Tracker {
  const endpoint = options.endpoint;
  const batchSize = Math.max(1, options.batchSize ?? 10);
  const flushIntervalMs = Math.max(0, options.flushIntervalMs ?? 5000);
  const maxAttempts = Math.max(1, options.maxAttempts ?? 5);
  const retryBaseMs = Math.max(1, options.retryBaseMs ?? 1000);
  const retryMaxMs = Math.max(retryBaseMs, options.retryMaxMs ?? 30000);
  const maxQueueEvents = Math.max(batchSize, options.maxQueueEvents ?? 500);
  const reportDrops = options.reportDrops !== false;
  const transport = options.transport ?? defaultTransport;
  const sessionId = options.sessionId !== undefined ? options.sessionId : defaultSessionId();

  const queue: QueueEntry[] = [];
  const stats: TrackerStats = {
    queued: 0,
    sent: 0,
    dropped: 0,
    droppedByReason: {
      retries_exhausted: 0,
      rejected: 0,
      queue_overflow: 0,
      previous_page: 0,
    },
    retries: 0,
    lastError: null,
  };

  let timer: ReturnType<typeof setTimeout> | null = null;
  let flushing = false;
  let stopped = false;
  /** Drops seen since the last drop report was enqueued, so a burst becomes one
   *  report rather than one report per lost event. When causes mix, the report
   *  carries the most recent one — the COUNT is the number that must be exact,
   *  and `stats().droppedByReason` keeps the full breakdown locally. */
  let unreportedDrops = 0;
  let unreportedReason: DropReason = 'retries_exhausted';

  function log(message: string): void {
    if (options.debug && typeof console !== 'undefined') {
      // Opt-in only: the default path must be invisible to the end user.
      console.warn(`[permagent-analytics] ${message}`);
    }
  }

  function persistDropCount(delta: number): void {
    const prior = Number(readStorage(DROP_COUNTER_KEY) ?? '0');
    const next = (Number.isFinite(prior) ? prior : 0) + delta;
    writeStorage(DROP_COUNTER_KEY, String(next));
  }

  function recordDrop(entries: QueueEntry[], reason: DropReason): void {
    if (entries.length === 0) return;
    stats.dropped += entries.length;
    stats.droppedByReason[reason] += entries.length;
    // Persisted so a tab closed with a full queue still reports the loss on the
    // next page load. Without this the worst-case loss is also the least
    // visible one.
    persistDropCount(entries.length);
    if (options.onDrop) {
      try {
        options.onDrop(entries.map((e) => e.beacon), reason);
      } catch {
        /* a broken hook must not become an app-visible error */
      }
    }
    log(`dropped ${entries.length} event(s): ${reason}`);
    // Only real events are worth reporting; a lost drop-report is already
    // counted in `stats.dropped`.
    const reportable = entries.filter((e) => !e.isDropReport).length;
    if (reportDrops && reportable > 0) {
      unreportedDrops += reportable;
      unreportedReason = reason;
    }
  }

  function enqueue(beacon: AnalyticsBeacon, isDropReport = false): void {
    if (stopped) return;
    queue.push({ beacon, attempts: 0, isDropReport });
    if (queue.length > maxQueueEvents) {
      // Oldest first: on a dead endpoint the newest events are the ones still
      // worth having when it comes back.
      const overflow = queue.splice(0, queue.length - maxQueueEvents);
      recordDrop(overflow, 'queue_overflow');
    }
    stats.queued = queue.length;
    if (queue.length >= batchSize) {
      void flush();
    } else {
      scheduleFlush(flushIntervalMs);
    }
  }

  function scheduleFlush(delayMs: number): void {
    if (stopped || timer !== null || queue.length === 0) return;
    timer = setTimeout(() => {
      timer = null;
      void flush();
    }, delayMs);
    // Node: a pending analytics timer must not hold the process open.
    (timer as unknown as { unref?: () => void }).unref?.();
  }

  function drainDropReport(): void {
    if (!reportDrops || unreportedDrops === 0) return;
    const count = unreportedDrops;
    const reason = unreportedReason;
    unreportedDrops = 0;
    // Queued directly rather than through `enqueue`, which would flush
    // immediately and re-enter the flush that just produced these drops. It
    // still travels the normal path on the next flush, landing in
    // `analytics_events` as a named event — so the loss shows up in the same UI
    // as the numbers it invalidates.
    queue.push({
      beacon: {
        k: 'ev',
        p: currentPath(),
        r: null,
        n: DROP_EVENT_NAME,
        d: { count, reason },
        s: sessionId,
      },
      attempts: 0,
      isDropReport: true,
    });
    stats.queued = queue.length;
  }

  async function flush(unloading = false): Promise<void> {
    if (flushing || queue.length === 0) return;
    flushing = true;
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    try {
      // One event per request (see the module header), so a 500-event backlog
      // would otherwise mean 500 sequential awaits inside a single flush, with
      // every later flush turned away as re-entrant for the duration. Bounded
      // here and drained by the immediate reschedule below — except on unload,
      // where this is the last chance to send anything at all.
      const take = unloading ? queue.length : Math.min(queue.length, Math.max(batchSize, 20));
      const batch = queue.splice(0, take);
      stats.queued = queue.length;
      const requeue: QueueEntry[] = [];
      const exhausted: QueueEntry[] = [];
      const rejected: QueueEntry[] = [];

      for (const entry of batch) {
        entry.attempts += 1;
        let result: SendResult;
        try {
          result = await transport(endpoint, JSON.stringify(entry.beacon), { unloading });
        } catch (err) {
          // A transport that throws is treated as a transient failure rather
          // than propagating into the caller's promise chain.
          result = { ok: false, retryable: true, error: String(err) };
        }
        if (result.ok) {
          stats.sent += 1;
          continue;
        }
        stats.lastError = result.error ?? `HTTP ${result.status ?? 0}`;
        if (result.retryable === false) {
          rejected.push(entry);
        } else if (entry.attempts >= maxAttempts) {
          exhausted.push(entry);
        } else {
          stats.retries += 1;
          requeue.push(entry);
        }
      }

      recordDrop(rejected, 'rejected');
      recordDrop(exhausted, 'retries_exhausted');

      if (requeue.length > 0) {
        // Retries go to the FRONT: order is preserved, and a steady stream of
        // new events cannot starve an event that has already been waiting.
        queue.unshift(...requeue);
        if (queue.length > maxQueueEvents) {
          recordDrop(queue.splice(0, queue.length - maxQueueEvents), 'queue_overflow');
        }
        stats.queued = queue.length;
      }

      const minAttempts = requeue.reduce(
        (min, e) => Math.min(min, e.attempts),
        Number.MAX_SAFE_INTEGER,
      );
      flushing = false;
      drainDropReport();
      if (requeue.length > 0 && !unloading) {
        scheduleFlush(backoffDelay(minAttempts, retryBaseMs, retryMaxMs));
      } else if (!unloading && queue.length >= batchSize) {
        // Events enqueued WHILE this flush was in flight (and any drop report
        // it just produced) hit `flush()` while `flushing` was true and were
        // turned away. Without this they would sit for a whole interval even
        // though the batch is already full.
        scheduleFlush(0);
      } else {
        scheduleFlush(flushIntervalMs);
      }
    } finally {
      flushing = false;
    }
  }

  function currentPath(): string {
    if (!isBrowser()) return '/';
    try {
      return location.pathname + location.search;
    } catch {
      return '/';
    }
  }

  function onHide(): void {
    // Not awaited: the page is going away and there is nothing to await into.
    void flush(true);
  }

  if (isBrowser()) {
    // Both, deliberately. `visibilitychange` is the only event iOS Safari
    // reliably fires when an app is backgrounded and later killed; `pagehide`
    // covers desktop navigations and the bfcache.
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden') onHide();
    });
    window.addEventListener('pagehide', onHide);
  }

  // Drops counted on a previous page load — including a tab closed mid-flush —
  // surface on the next load rather than vanishing with the tab.
  const carried = Number(readStorage(DROP_COUNTER_KEY) ?? '0');
  if (reportDrops && Number.isFinite(carried) && carried > 0) {
    writeStorage(DROP_COUNTER_KEY, '0');
    // Counted into this tracker's totals as well: the running total has to
    // survive a reload or the worst loss (a tab closed mid-flush) is also the
    // one nobody ever sees.
    stats.dropped += carried;
    stats.droppedByReason.previous_page += carried;
    enqueue(
      {
        k: 'ev',
        p: currentPath(),
        r: null,
        n: DROP_EVENT_NAME,
        d: { count: carried, reason: 'previous_page' },
        s: sessionId,
      },
      true,
    );
  }

  return {
    trackEvent(name: string, properties?: EventProperties): void {
      try {
        const clean = String(name ?? '').trim().slice(0, MAX_NAME_CHARS);
        if (!clean) return;
        enqueue({
          k: 'ev',
          p: currentPath(),
          r: null,
          n: clean,
          d: sanitizeProperties(properties),
          s: sessionId,
        });
      } catch {
        /* fire-and-forget: never surface an error to the end user */
      }
    },
    trackPageview(path?: string, referrer?: string | null): void {
      try {
        enqueue({
          k: 'pv',
          p: path ?? currentPath(),
          r: referrer ?? (isBrowser() ? document.referrer || null : null),
          n: null,
          d: null,
          s: sessionId,
        });
      } catch {
        /* fire-and-forget */
      }
    },
    flush(): Promise<void> {
      return flush().catch(() => undefined);
    },
    stats(): TrackerStats {
      return { ...stats, droppedByReason: { ...stats.droppedByReason }, queued: queue.length };
    },
    async shutdown(): Promise<void> {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
      if (isBrowser()) {
        window.removeEventListener('pagehide', onHide);
      }
      await flush(true).catch(() => undefined);
      stopped = true;
    },
  };
}

/** Module-level convenience for apps that want one tracker: `init()` once at
 *  startup, then `trackEvent()` anywhere. Calling `trackEvent` before `init`
 *  is a no-op rather than a crash — analytics must never be load-order
 *  sensitive in a way that breaks the app. */
let shared: Tracker | null = null;

export function init(options: TrackerOptions): Tracker {
  shared = createTracker(options);
  return shared;
}

export function trackEvent(name: string, properties?: EventProperties): void {
  shared?.trackEvent(name, properties);
}

export function trackPageview(path?: string, referrer?: string | null): void {
  shared?.trackPageview(path, referrer);
}

export function analyticsStats(): TrackerStats | null {
  return shared ? shared.stats() : null;
}

/** Test seam only. */
export function resetSharedTracker(): void {
  shared = null;
}
