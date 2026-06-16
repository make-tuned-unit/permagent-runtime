// ============================================================================
// DEV-ONLY TIME-LAPSE DRIVER — NOT shipped, NOT real activity.
// ============================================================================
// THE_CAVE_vision_bible.md §7 (the time-lapse asset) + §4 ("Demos use the time-lapse,
// not accelerated live rates"). This driver compresses the slow, real banked-events loop
// into seconds so the tending + construction system can be DEMONSTRATED and captured for
// evidence without waiting days for real describe/ingest events.
//
// HONESTY GUARDS (bible §8 / sim-state clamp):
//   • Gated on import.meta.env.DEV — a no-op in production builds. installTimeLapse()
//     returns immediately when DEV is false.
//   • Every deposit goes through bankEvent(..., 'dev') — the bank's provenance bit marks
//     these units 'dev', NEVER 'daemon'. Evidence overlays/logs show the tag, so a
//     time-lapse capture can never be mistaken for a real run.
//   • It does NOT touch agent HUD state (no fake working/error). It only fills the bank;
//     the genuine tending/construction logic reacts exactly as it does to real events.
//   • Exposed only on window.__worldTimeLapse for the CDP evidence harness.
//
// Usage (dev console / CDP):  window.__worldTimeLapse.run({ units: 8, intervalMs: 1200 })

import { bankEvent, resetBank } from '../shared/tendingBank';
import { resetConstruction } from './construction';

export interface TimeLapseOptions {
  /** How many banked units to deposit over the run. */
  units?: number;
  /** Milliseconds between deposits (compressed time — real loop is days). */
  intervalMs?: number;
  /** Fraction of deposits that are 'ingest' (memory_added) vs 'describe'. */
  ingestRatio?: number;
}

interface TimeLapseHandle {
  run: (opts?: TimeLapseOptions) => void;
  stop: () => void;
  reset: () => void;
  running: () => boolean;
}

let timer: ReturnType<typeof setInterval> | null = null;
let installed = false;

function stop(): void {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
}

function run(opts: TimeLapseOptions = {}): void {
  stop();
  const units = opts.units ?? 8;
  const intervalMs = opts.intervalMs ?? 1200;
  const ingestRatio = opts.ingestRatio ?? 0.4;
  let i = 0;
  // First deposit immediately so the demo starts without a beat of dead air.
  const deposit = () => {
    const isIngest = Math.random() < ingestRatio;
    bankEvent(
      isIngest ? 'ingest' : 'describe',
      `dev-timelapse-${Date.now()}-${i}`,
      'dev', // ← provenance: never 'daemon'
    );
    i++;
    if (i >= units) stop();
  };
  deposit();
  timer = setInterval(deposit, intervalMs);
}

/**
 * Install the dev-only handle on window. No-op outside DEV builds (the whole driver is
 * absent from production behaviour). Returns a teardown for the caller's useEffect.
 */
export function installTimeLapse(): () => void {
  if (!import.meta.env.DEV || installed) return () => {};
  installed = true;
  const handle: TimeLapseHandle = {
    run,
    stop,
    reset: () => {
      stop();
      resetBank();
      resetConstruction();
    },
    running: () => timer !== null,
  };
  (window as unknown as Record<string, unknown>).__worldTimeLapse = handle;
  return () => {
    stop();
    installed = false;
    delete (window as unknown as Record<string, unknown>).__worldTimeLapse;
  };
}
