/**
 * Coding-session capture helpers — regression tests.
 *
 * Pins: harness detection (first token, path prefixes, no substring false
 * positives), ANSI stripping (the daemon summarizes prose, not cursor
 * choreography), tail bounding, and the trivial-transcript guard that keeps
 * mis-fires (instant exits, --version) out of the Brain.
 */
import { describe, expect, it } from 'vitest';
import {
  buildCodingSessionPayload,
  isHarnessCommand,
  stripAnsi,
  transcriptTail,
} from './codingSession';

describe('isHarnessCommand', () => {
  it('matches the three harness CLIs by first token', () => {
    expect(isHarnessCommand('claude')).toBe(true);
    expect(isHarnessCommand('codex --resume')).toBe(true);
    expect(isHarnessCommand('permagent run --recipe x.yaml')).toBe(true);
    expect(isHarnessCommand('~/bin/claude --continue')).toBe(true);
  });
  it('never false-positives on substrings or other commands', () => {
    expect(isHarnessCommand('git clone claude-thing')).toBe(false);
    expect(isHarnessCommand('claudette')).toBe(false);
    expect(isHarnessCommand('ls')).toBe(false);
    expect(isHarnessCommand('')).toBe(false);
    expect(isHarnessCommand(null)).toBe(false);
  });
});

describe('stripAnsi', () => {
  it('removes CSI, OSC and control noise but keeps prose', () => {
    const raw = '\x1b[1;32mDone\x1b[0m\x1b]0;title\x07 building\r\n\x1b(Bnext\tstep';
    expect(stripAnsi(raw)).toBe('Done building\nnext\tstep');
  });
});

describe('transcriptTail', () => {
  it('keeps the end — the wrap-up lives there', () => {
    expect(transcriptTail('abcdef', 3)).toBe('def');
    expect(transcriptTail('abc', 10)).toBe('abc');
  });
});

describe('buildCodingSessionPayload', () => {
  const longTranscript = 'x'.repeat(50) + ' implemented the analytics drain fix. '.repeat(20);

  it('assembles a bounded, ANSI-free payload with duration', () => {
    const p = buildCodingSessionPayload({
      rawTranscript: `\x1b[2J${longTranscript}`,
      cwd: '/Users/x/dev/proj',
      command: 'claude',
      spawnedAtMs: 1_000,
      nowMs: 601_000,
    });
    expect(p).not.toBeNull();
    expect(p!.duration_secs).toBe(600);
    expect(p!.transcript).not.toContain('\x1b');
    expect(p!.command).toBe('claude');
  });

  it('refuses trivial transcripts — a mis-fire is not a memory', () => {
    expect(buildCodingSessionPayload({
      rawTranscript: 'claude 1.2.3\n',
      command: 'claude --version',
      spawnedAtMs: 0,
      nowMs: 1_000,
    })).toBeNull();
  });
});
