/**
 * What the three emitter-less seats (J11: Council, Polybot, Picker) are allowed
 * to say about themselves.
 *
 * Pure on purpose. These are the app's liveness claims for agents whose
 * `agent_state_changed` emitters do not exist yet (agent-QA D-N5-1, D22), which
 * makes them exactly the place a claim can quietly outgrow its evidence — so
 * the decision is made here, in one testable function per desk, rather than in
 * a chain of ternaries inside a panel.
 *
 * Two rules run through all three:
 *
 *   1. A `live` pill means a real reading backs it AND that reading is stamped
 *      with when it was taken. Everything else is `static` — drawn as a caption
 *      rather than a signal, per the Chip contract.
 *   2. A failed read is `unreachable`, never OFF. "The daemon didn't answer" is
 *      a claim about the connection; "Polybot is stopped" is a claim about the
 *      machine, and rendering the first as the second is how a dead wire comes
 *      to look like a deliberate setting.
 */

import type { FinanceDeskReading } from './financeDesk';

export interface DeskStatus {
  label: string;
  /** True ⇒ render as `Chip kind="state"` with an `asOf`. */
  live: boolean;
  /** True ⇒ a claim that something is happening right now. */
  pulse: boolean;
  /** The read failed, or the desk's own service is not answering. */
  unreachable: boolean;
}

const CHECKING: DeskStatus = { label: 'CHECKING…', live: false, pulse: false, unreachable: false };

export function polybotStatus(reading: FinanceDeskReading): DeskStatus {
  const { board, error, asOf } = reading;
  if (board === null) {
    return error === null
      ? CHECKING
      : { label: 'NO ANSWER', live: false, pulse: false, unreachable: true };
  }
  // A setting, not a status — the Guard's HUD draws its own OFF the same way.
  if (board.polybotEnabled === false) {
    return { label: 'OFF', live: false, pulse: false, unreachable: false };
  }
  const p = board.polybot;
  if (!p) return CHECKING;
  const live = asOf !== null;
  if (!p.found) return { label: 'NOT INSTALLED', live: false, pulse: false, unreachable: false };
  if (p.paused) return { label: 'PAUSED', live, pulse: false, unreachable: false };
  if (p.running !== true) return { label: 'STOPPED', live, pulse: false, unreachable: false };
  // Up. NOT pulsing: the board says the process exists, which is not the same
  // as saying it is doing something this second — no emitter reports that yet.
  return { label: 'RUNNING', live, pulse: false, unreachable: false };
}

export function pickerStatus(reading: FinanceDeskReading): DeskStatus {
  const { board, error, asOf } = reading;
  if (board === null) {
    return error === null
      ? CHECKING
      : { label: 'NO ANSWER', live: false, pulse: false, unreachable: true };
  }
  if (board.pickerEnabled === false) {
    return { label: 'OFF', live: false, pulse: false, unreachable: false };
  }
  const p = board.picker;
  if (!p) return CHECKING;
  const live = asOf !== null;
  if (!p.reachable) {
    return { label: 'SCANNER DOWN', live, pulse: false, unreachable: true };
  }
  // The one genuine in-flight claim these desks can make: the scanner itself
  // reports `scan_in_progress`, read fresh from the board.
  if (p.scanInProgress) return { label: 'SCANNING', live, pulse: true, unreachable: false };
  return { label: 'READY', live, pulse: false, unreachable: false };
}

/**
 * The Council convenes Sunday 22:00 local, with a Monday catch-up if the
 * machine slept (`council/due.rs`). That is a fact about the calendar, and it
 * stays a STATIC pill however true it is: no council event constructor exists
 * in the daemon at all, so nothing can report a session actually starting.
 *
 * @param enabled `council_enabled`, or null while the read is outstanding.
 */
export function councilStatus(enabled: boolean | null): DeskStatus {
  if (enabled === false) return { label: 'OFF', live: false, pulse: false, unreachable: false };
  return { label: 'SUNDAYS 22:00', live: false, pulse: false, unreachable: false };
}
