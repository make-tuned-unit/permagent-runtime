/**
 * The People tab reads your macOS Calendar on every mount. The pins here are
 * that it says so, that a failure is never rendered as a quiet success, and
 * that it never claims to know something the daemon does not report — a
 * permission failure comes back as `imported: 0`, identical to an empty week,
 * so "access is off" is a guess and must not be stated as a fact.
 */

import { describe, expect, it } from 'vitest';
import { calendarImportLine } from './calendarImport';

describe('calendar auto-import acknowledgment', () => {
  it('says an import is running', () => {
    expect(calendarImportLine({ phase: 'importing' }).text).toContain('Checking your calendar');
  });

  it('acknowledges what it added', () => {
    const line = calendarImportLine({ phase: 'done', imported: 3, at: 0 });
    expect(line.text).toContain('3 new meetings');
    expect(line.tone).toBe('muted');
  });

  it('says one meeting in the singular', () => {
    expect(calendarImportLine({ phase: 'done', imported: 1, at: 0 }).text)
      .toContain('1 new meeting');
  });

  it('does not claim access is off when it cannot know', () => {
    const line = calendarImportLine({ phase: 'done', imported: 0, at: 0 });
    expect(line.text).toContain('no new meetings');
    expect(line.text.toLowerCase()).not.toContain('access is off');
    // The ambiguity is named where it applies rather than guessed at.
    expect(line.title ?? '').toContain('Privacy & Security');
  });

  it('surfaces a real failure with a way back', () => {
    const line = calendarImportLine({ phase: 'failed', message: 'the daemon did not answer' });
    expect(line.tone).toBe('warning');
    expect(line.retry).toBe(true);
    expect(line.text).toContain("Couldn't check your calendar");
  });
});
