/**
 * The three seats J11 added have no emitter yet (agent-QA D-N5-1, D22), so the
 * only thing that keeps them honest is what their pill is allowed to say.
 *
 * The rule these pin: a pill is LIVE only when a real reading backs it, and a
 * reading that failed is never rendered as a desk being switched off. "The
 * daemon didn't answer" and "Polybot is stopped" are different sentences about
 * different things, and collapsing them is how a dead connection comes to look
 * like a deliberate setting.
 */

import { describe, expect, it } from 'vitest';
import { councilStatus, polybotStatus, pickerStatus } from './deskStatus';

describe('Polybot pill', () => {
  it('says so, statically, when Polybot is switched off — a setting, not a status', () => {
    const s = polybotStatus({ board: { polybotEnabled: false }, error: null, asOf: 1 });
    expect(s).toMatchObject({ label: 'OFF', live: false });
  });

  it('never reports OFF on a failed read', () => {
    const s = polybotStatus({ board: null, error: 'connection refused', asOf: null });
    expect(s.label).not.toBe('OFF');
    expect(s).toMatchObject({ live: false, unreachable: true });
  });

  it('waits rather than guessing before the first reading lands', () => {
    expect(polybotStatus({ board: null, error: null, asOf: null }).label).toBe('CHECKING…');
  });

  it('separates "not installed" from "installed but stopped"', () => {
    const missing = polybotStatus({
      board: { polybotEnabled: true, polybot: { found: false, paused: false, stale: false } },
      error: null, asOf: 1,
    });
    const stopped = polybotStatus({
      board: { polybotEnabled: true, polybot: { found: true, running: false, paused: false, stale: false } },
      error: null, asOf: 1,
    });
    expect(missing.label).toBe('NOT INSTALLED');
    expect(stopped.label).toBe('STOPPED');
  });

  it('is live only when a real reading says the process is up', () => {
    const s = polybotStatus({
      board: { polybotEnabled: true, polybot: { found: true, running: true, paused: false, stale: false } },
      error: null, asOf: 1,
    });
    expect(s).toMatchObject({ label: 'RUNNING', live: true });
  });

  it('reports a paused process as paused, never as running', () => {
    const s = polybotStatus({
      board: { polybotEnabled: true, polybot: { found: true, running: true, paused: true, stale: false } },
      error: null, asOf: 1,
    });
    expect(s.label).toBe('PAUSED');
  });
});

describe('Picker pill', () => {
  it('never reports OFF on a failed read', () => {
    const s = pickerStatus({ board: null, error: 'timeout', asOf: null });
    expect(s.label).not.toBe('OFF');
    expect(s.unreachable).toBe(true);
  });

  it('says the scanner is down when the scanner is down — not that the desk is off', () => {
    const s = pickerStatus({
      board: { pickerEnabled: true, picker: { reachable: false, baseUrl: 'http://localhost:8080', scanInProgress: false } },
      error: null, asOf: 1,
    });
    expect(s).toMatchObject({ label: 'SCANNER DOWN', unreachable: true });
  });

  it('pulses only while a scan is genuinely in flight', () => {
    const idle = pickerStatus({
      board: { pickerEnabled: true, picker: { reachable: true, baseUrl: '', scanInProgress: false } },
      error: null, asOf: 1,
    });
    const scanning = pickerStatus({
      board: { pickerEnabled: true, picker: { reachable: true, baseUrl: '', scanInProgress: true } },
      error: null, asOf: 1,
    });
    expect(idle.pulse).toBe(false);
    expect(scanning).toMatchObject({ label: 'SCANNING', pulse: true, live: true });
  });
});

describe('Council pill', () => {
  it('states the cadence as a standing fact — never as a live status', () => {
    const s = councilStatus(true);
    expect(s.label).toContain('SUNDAYS 22:00');
    // No emitter exists for the Council anywhere in the daemon (D-N5-1): a
    // schedule is a fact about the calendar, and this pill must never claim to
    // be watching one convene.
    expect(s.live).toBe(false);
  });

  it('says OFF when the Council is switched off, and waits while unknown', () => {
    expect(councilStatus(false).label).toBe('OFF');
    expect(councilStatus(null).label).toContain('SUNDAYS 22:00');
  });
});
