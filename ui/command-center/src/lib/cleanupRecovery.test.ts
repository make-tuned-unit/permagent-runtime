// Issue #242 — recovery total accounting. Pins that the per-run sum only
// counts trashed findings, and that the displayed strings are explicit about
// the period they cover (per-run vs all-time).
import { describe, it, expect } from 'vitest';
import {
  sumRunRecovered,
  sumPendingBytes,
  recoveryHeadline,
  lifetimeRecoveryLine,
  formatBytes,
  type RecoveryFinding,
} from './cleanupRecovery';

const f = (over: Partial<RecoveryFinding>): RecoveryFinding => ({
  action_taken: null,
  size_recovered_bytes: null,
  size_bytes: 0,
  ...over,
});

describe('per-run recovery sum', () => {
  it('counts only trashed findings', () => {
    const findings = [
      f({ action_taken: 'trashed', size_recovered_bytes: 100 }),
      f({ action_taken: 'trashed', size_recovered_bytes: 50 }),
      f({ action_taken: 'keep', size_recovered_bytes: null, size_bytes: 999 }),
      f({ action_taken: null, size_bytes: 500 }),
      f({ action_taken: 'error', size_bytes: 123 }),
    ];
    expect(sumRunRecovered(findings)).toBe(150);
    expect(sumPendingBytes(findings)).toBe(500);
  });

  it('is zero for a scan with no actions', () => {
    expect(sumRunRecovered([f({ size_bytes: 10 })])).toBe(0);
  });
});

describe('recovery headline (period must be explicit)', () => {
  it('labels the completed total as this-run, not a bare "recovered"', () => {
    expect(recoveryHeadline(true, 2048, 0)).toBe('2 KB recovered this run');
  });

  it('shows pending bytes before all items are actioned', () => {
    expect(recoveryHeadline(false, 0, 3 * 1024 * 1024)).toBe('3.0 MB to clean up');
  });
});

describe('lifetime recovery line', () => {
  it('is hidden until totals load', () => {
    expect(lifetimeRecoveryLine(null)).toBeNull();
  });

  it('is hidden when nothing was ever recovered', () => {
    expect(
      lifetimeRecoveryLine({ total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 }),
    ).toBeNull();
  });

  it('reports the cumulative cross-run total with run/item counts', () => {
    expect(
      lifetimeRecoveryLine({
        total_recovered_bytes: 5 * 1024 * 1024 * 1024,
        runs_with_recovery: 3,
        items_trashed: 12,
      }),
    ).toBe('5.0 GB recovered all-time — 12 items across 3 runs');
  });

  it('uses singular forms for one item in one run', () => {
    expect(
      lifetimeRecoveryLine({ total_recovered_bytes: 100, runs_with_recovery: 1, items_trashed: 1 }),
    ).toBe('100 B recovered all-time — 1 item across 1 run');
  });
});

describe('formatBytes', () => {
  it('covers the unit ladder', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(10 * 1024)).toBe('10 KB');
    expect(formatBytes(1.5 * 1024 * 1024)).toBe('1.5 MB');
    expect(formatBytes(2.5 * 1024 * 1024 * 1024)).toBe('2.5 GB');
  });
});
