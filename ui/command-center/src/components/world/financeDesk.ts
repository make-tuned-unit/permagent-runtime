/**
 * The finance board, read for the World's two Finance desks (Polybot and the
 * Picker).
 *
 * Neither desk has an `agent_state_changed` emitter — Polybot has none at all
 * and the Picker announces under the `financier` id (agent-QA D22). So the orbs
 * stay `wire: 'static'` and their state is NOT invented here. What IS real and
 * readable today is the board itself: `/api/finance` carries `polybot.status()`
 * and `picker.status()`, the same source the Finance tab renders. This hook
 * reads that, on open and on a 60s cadence while the HUD is up (the same
 * cadence FinanceView uses), and hands back WHEN it read — so every claim the
 * HUDs make can be stamped with the moment it was last confirmed rather than
 * presented as a live wire it does not have.
 *
 * A failed read is an error, never an OFF: "the daemon didn't answer" and "the
 * process is stopped" are different sentences and the HUDs say the right one.
 */

import { useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';

export interface PolybotDesk {
  /** Is the Polybot checkout present on this machine at all? */
  found: boolean;
  running?: boolean;
  paused: boolean;
  credentialsReady?: boolean;
  quietHours?: boolean;
  tradeCount?: number | null;
  currentBalance?: number | null;
  lastUpdated?: string | null;
  stale: boolean;
  staleDays?: number | null;
  detail?: string | null;
}

export interface PickerDesk {
  /** Is the scanner service answering? */
  reachable: boolean;
  baseUrl: string;
  scanInProgress: boolean;
  scanDate?: string | null;
  results?: number | null;
  detail?: string | null;
}

interface FinanceDeskBoard {
  polybot?: PolybotDesk;
  polybotEnabled?: boolean;
  picker?: PickerDesk;
  pickerEnabled?: boolean;
  pickerUniverseCount?: number | null;
}

export interface FinanceDeskReading {
  board: FinanceDeskBoard | null;
  /** The read failed — NOT the same thing as a desk being off. */
  error: string | null;
  /** Epoch ms of the last successful read; null until one lands. */
  asOf: number | null;
}

const POLL_MS = 60_000;

/** Reads only while `visible` — a closed HUD costs nothing. */
export function useFinanceDesk(visible: boolean): FinanceDeskReading {
  const [reading, setReading] = useState<FinanceDeskReading>({
    board: null,
    error: null,
    asOf: null,
  });

  useEffect(() => {
    if (!visible) return;
    let active = true;
    const load = async () => {
      try {
        const board = await apiFetch<FinanceDeskBoard>('/api/finance');
        if (active) setReading({ board, error: null, asOf: Date.now() });
      } catch (e) {
        // Keep the last good board — a dropped poll does not un-know what was
        // true a minute ago — but say the reading failed, and leave `asOf`
        // where it was so nothing stale reads as fresh.
        if (active) {
          setReading(prev => ({
            ...prev,
            error: e instanceof Error ? e.message : 'the daemon did not answer',
          }));
        }
      }
    };
    void load();
    const t = setInterval(load, POLL_MS);
    return () => { active = false; clearInterval(t); };
  }, [visible]);

  return reading;
}
