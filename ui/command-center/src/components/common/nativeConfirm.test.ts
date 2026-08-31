/**
 * No native `confirm()` anywhere in the app.
 *
 * An OS `confirm()` bypasses the theme, the focus ring and the interface voice
 * all at once, and it collapses the three destructive tiers into one generic
 * browser dialog. The three that existed (rotate the analytics drain key,
 * delete an automation, remove a custom provider) are now a `ConfirmDialog` or
 * an inline two-step, per tier. This test exists so a fourth cannot quietly
 * appear: it is the gate, not the fix.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../..', import.meta.url));

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

/** `confirm(` / `window.confirm(` as a CALL — not `onConfirm(`, not a method on
 *  something else, and not the word in a sentence (prose puts a space before
 *  its parenthesis; a call never does). */
const NATIVE_CONFIRM = /(?:^|[^.\w])(?:window\.)?confirm\(/;

describe('destructive confirmations', () => {
  it('never uses the native browser confirm()', () => {
    const offenders = sourceFiles(SRC)
      .filter(f => NATIVE_CONFIRM.test(readFileSync(f, 'utf8')))
      .map(f => f.slice(SRC.length));
    expect(offenders, 'use ConfirmDialog (Tier 3) or an inline two-step (Tier 2)').toEqual([]);
  });
});
