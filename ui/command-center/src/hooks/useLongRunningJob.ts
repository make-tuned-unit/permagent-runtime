/**
 * `useLongRunningJob` — the app's one phase machine for work that outlives a
 * button press.
 *
 * Why it exists: three surfaces started a multi-second-to-multi-minute job and
 * each invented its own way of saying so. The best of them — the wizard's
 * Ollama pull (`components/wizard/MomentHardware.tsx`) — got it right, and its
 * own code comment states the rule the others were breaking:
 *
 *   > a silent catch here would let the wizard claim the Librarian is
 *   > configured when it isn't
 *
 * That sentence is the contract. This hook is that implementation, extracted,
 * so a fourth variant does not get invented.
 *
 * ── THE CONTRACT (U3 §1.7) ──────────────────────────────────────────────────
 *
 * 1. **Named phases, never a bare boolean.** `idle → starting → running →
 *    succeeded | failed | stopped`. The three terminal phases are distinct
 *    because they are three different sentences: "it worked", "it broke, here
 *    is why", and "you stopped it". A job never falls back to `idle` on its
 *    own — a control that looks untouched after a failure is the lie above.
 * 2. **Honest progress.** `percent` is a number only when the backend actually
 *    reported a `total`; otherwise it is `null` and the view must draw an
 *    indeterminate indicator. Nothing here invents a size, and nothing fakes
 *    forward motion.
 * 3. **The real error text survives.** `error` carries what the backend or the
 *    exception said, never a generic "something went wrong".
 * 4. **Abort is a user outcome, not a failure.** An `AbortError` — whether
 *    raised by our own `AbortSignal` or by a stream's `abort()` — lands in
 *    `stopped` with `error: null`.
 * 5. **Transport-agnostic.** The hook only knows a `run(ctx)` that reports
 *    readings and eventually settles. Two ready-made runners are exported:
 *    `streamingRunner` (SSE / chunked-JSON, the Ollama-pull shape) and
 *    `pollingRunner` (start-then-ask, the scan/sweep shape). Anything else
 *    that can call `ctx.report` works too.
 *
 * ── FOR THE QWEN BRING-UP LANE (harness-dag D2) ─────────────────────────────
 *
 * D2's design names its own phases — `stopped / starting / loading_model /
 * warming / ready / error(reason) / occupied_by_librarian`. Those are *backend*
 * phases and they pass through untouched in `reading.stage`; they are not
 * flattened into this hook's five. Wire D2 as:
 *
 *   const job = useLongRunningJob({
 *     run: streamingRunner(
 *       (onData) => api.startQwen(onData),          // returns { promise, abort }
 *       (f) => ({ stage: f.phase, status: f.detail, completed: f.done, total: f.steps }),
 *     ),
 *     summarize: () => 'Qwen is ready',
 *   });
 *
 * and render `<JobProgress job={job} label="Starting Qwen" />`. The two-tier
 * disclosure D2 wants (parsed phase + raw log lines) is `reading.stage` for the
 * first tier and `reading.status` for the second; if D2 needs a scrolling log
 * rather than a single line it should add `lines?: string[]` to `JobReading`
 * here rather than forking the hook.
 */

import { useCallback, useEffect, useRef, useState } from 'react';

/** The five phases a caller may branch on. Backend phases live in
 *  `JobReading.stage` and are never folded into this. */
export type JobPhase = 'idle' | 'starting' | 'running' | 'succeeded' | 'failed' | 'stopped';

export interface JobReading {
  /** The backend's own phase name, verbatim — "loading_model", "scanning". */
  stage?: string;
  /** One human line: "Downloading layer 3/8", "Fetching 240 tickers". */
  status?: string;
  /** Units done / units total. BOTH are needed for a percentage. */
  completed?: number;
  total?: number;
}

export interface JobContext {
  /** Publish a reading. Fields are merged over the previous reading, so a
   *  status-only frame does not wipe a known total. */
  report: (reading: JobReading) => void;
  /** Aborted when the user presses Stop or the component unmounts. */
  signal: AbortSignal;
}

export type JobRunner<T> = (ctx: JobContext) => Promise<T>;

export interface UseLongRunningJobOptions<T> {
  run: JobRunner<T>;
  /** Names what completed — "Scan complete — 4 findings". Without it the view
   *  falls back to its own label, which is weaker but never wrong. */
  summarize?: (result: T) => string;
  /** Called once on `succeeded`, for the refresh the job's result implies. */
  onSuccess?: (result: T) => void;
}

export interface LongRunningJob<T> {
  phase: JobPhase;
  /** `starting` or `running`. */
  running: boolean;
  reading: JobReading;
  /** 0–100 when the backend reported a size, else null → draw indeterminate. */
  percent: number | null;
  /** The real failure text. Null in every other phase, `stopped` included. */
  error: string | null;
  result: T | null;
  /** `summarize(result)`, or null. */
  summary: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  /** Starts the work. A second call while running is ignored, not queued. */
  start: () => Promise<void>;
  /** User-initiated stop. Lands in `stopped`, never `failed`. */
  abort: () => void;
  /** Back to `idle`, clearing the last outcome. For dismissing a result row. */
  reset: () => void;
}

function isAbort(e: unknown): boolean {
  return (e as { name?: string } | null)?.name === 'AbortError';
}

function messageOf(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === 'string' && e) return e;
  return 'the job failed without saying why';
}

/** Percent, or null when there is no honest one to give. A `total` of 0 means
 *  "size unknown" — reporting it as 0% or 100% would both be inventions. */
export function percentOf(reading: JobReading): number | null {
  const { completed, total } = reading;
  if (typeof total !== 'number' || typeof completed !== 'number') return null;
  if (!Number.isFinite(total) || !Number.isFinite(completed) || total <= 0) return null;
  return Math.max(0, Math.min(100, Math.round((completed / total) * 100)));
}

export function useLongRunningJob<T>(options: UseLongRunningJobOptions<T>): LongRunningJob<T> {
  const { run, summarize, onSuccess } = options;
  const [phase, setPhase] = useState<JobPhase>('idle');
  const [reading, setReading] = useState<JobReading>({});
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<T | null>(null);
  const [summary, setSummary] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [finishedAt, setFinishedAt] = useState<number | null>(null);

  const live = useRef(true);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  // Latest callbacks without making `start` a new function every render.
  const latest = useRef({ run, summarize, onSuccess });
  latest.current = { run, summarize, onSuccess };

  useEffect(() => () => {
    live.current = false;
    controller.current?.abort();
  }, []);

  const start = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    const ctl = new AbortController();
    controller.current = ctl;

    setPhase('starting');
    setReading({});
    setError(null);
    setResult(null);
    setSummary(null);
    setFinishedAt(null);
    setStartedAt(Date.now());

    const report = (next: JobReading) => {
      if (!live.current || ctl.signal.aborted) return;
      // Merge, so a status-only frame keeps a total the previous frame gave.
      setReading(prev => ({ ...prev, ...next }));
      setPhase(p => (p === 'starting' ? 'running' : p));
    };

    try {
      // `running` from the first tick: holding `starting` for the whole job
      // would make the trigger read as a click that never landed.
      setPhase('running');
      const value = await latest.current.run({ report, signal: ctl.signal });
      if (!live.current) return;
      if (ctl.signal.aborted) { setPhase('stopped'); setFinishedAt(Date.now()); return; }
      setResult(value);
      setSummary(latest.current.summarize ? latest.current.summarize(value) : null);
      setPhase('succeeded');
      setFinishedAt(Date.now());
      latest.current.onSuccess?.(value);
    } catch (e) {
      if (!live.current) return;
      setFinishedAt(Date.now());
      if (isAbort(e) || ctl.signal.aborted) {
        // The user's own Stop. Not a defect, and not an error message.
        setPhase('stopped');
        setError(null);
      } else {
        setPhase('failed');
        setError(messageOf(e));
      }
    } finally {
      inFlight.current = false;
    }
  }, []);

  const abort = useCallback(() => {
    controller.current?.abort();
    if (!inFlight.current) return;
    // The runner may never settle after an abort (a poll loop simply stops);
    // the phase must not be left saying "running".
    if (live.current) { setPhase('stopped'); setError(null); setFinishedAt(Date.now()); }
  }, []);

  const reset = useCallback(() => {
    controller.current?.abort();
    controller.current = null;
    setPhase('idle');
    setReading({});
    setError(null);
    setResult(null);
    setSummary(null);
    setStartedAt(null);
    setFinishedAt(null);
  }, []);

  return {
    phase,
    running: phase === 'starting' || phase === 'running',
    reading,
    percent: percentOf(reading),
    error,
    result,
    summary,
    startedAt,
    finishedAt,
    start,
    abort,
    reset,
  };
}

// ── Runners ─────────────────────────────────────────────────────────────────

/**
 * SSE / chunked-JSON transport: `begin` opens the stream and hands back the
 * `{ promise, abort }` pair `api.pullOllamaModel` established. `read` turns one
 * wire frame into a reading — this is the only place a transport's field names
 * are known, which is what keeps the hook transport-agnostic.
 */
export function streamingRunner<T, F>(
  begin: (onData: (frame: F) => void) => { promise: Promise<T>; abort?: () => void },
  read: (frame: F) => JobReading,
): JobRunner<T> {
  return ({ report, signal }) => {
    const { promise, abort } = begin((frame) => report(read(frame)));
    if (abort) {
      if (signal.aborted) abort();
      else signal.addEventListener('abort', abort, { once: true });
    }
    return promise;
  };
}

/** One answer from the job's status endpoint. */
export interface PollTick<T> {
  done: boolean;
  reading?: JobReading;
  /** Present on the terminal tick of a successful run. */
  result?: T;
  /** A failure the backend *reported* rather than threw. Terminal. */
  error?: string;
}

/**
 * Start-then-ask transport, for work with no stream: a scan, a cleanup sweep.
 * `begin` kicks it off, `poll` is asked every `intervalMs` until it says
 * `done`. An indeterminate job simply never sets `total`, and the view draws
 * the honest indicator rather than a fabricated bar.
 */
export function pollingRunner<T>(opts: {
  begin: () => Promise<void>;
  poll: () => Promise<PollTick<T>>;
  /** Default 2s — fast enough to feel live, slow enough not to hammer. */
  intervalMs?: number;
}): JobRunner<T> {
  const { begin, poll, intervalMs = 2000 } = opts;
  return async ({ report, signal }) => {
    const abortError = () => {
      const e = new Error('stopped');
      e.name = 'AbortError';
      return e;
    };
    if (signal.aborted) throw abortError();
    await begin();
    for (;;) {
      if (signal.aborted) throw abortError();
      const tick = await poll();
      if (tick.reading) report(tick.reading);
      if (tick.error) throw new Error(tick.error);
      if (tick.done) return tick.result as T;
      await new Promise<void>((resolve) => { setTimeout(resolve, intervalMs); });
    }
  };
}
