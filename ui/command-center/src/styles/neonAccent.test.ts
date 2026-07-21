// Issue #193 — one canonical neon accent (#00D5FF, ruled 2026-06-23).
// Pins the constant, every token that must derive from it, and guards the
// whole src tree against the historical drift value (#00D9FF) creeping back.
import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { NEON_ACCENT, color } from './tokens';
import { STATE, ENV } from '../components/world/shared/palette';
import { COLORS } from '../components/world/constants';

const SRC_DIR = join(dirname(fileURLToPath(import.meta.url)), '..');
const THIS_FILE = fileURLToPath(import.meta.url);
const SOURCE_EXT = /\.(ts|tsx|css|js|jsx|html)$/;

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (SOURCE_EXT.test(name)) out.push(p);
  }
  return out;
}

describe('canonical neon accent (#193)', () => {
  it('NEON_ACCENT is the ruled canonical value', () => {
    expect(NEON_ACCENT).toBe('#00D5FF');
  });

  it('every neon-cyan token derives from NEON_ACCENT', () => {
    expect(color.cyan).toBe(NEON_ACCENT);
    expect(ENV.neonCyan).toBe(NEON_ACCENT);
    expect(STATE.available).toBe(NEON_ACCENT);
    expect(COLORS.neonCyan).toBe(NEON_ACCENT);
  });

  it('the drift value #00D9FF appears nowhere in src', () => {
    const offenders: string[] = [];
    for (const file of walk(SRC_DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      // Hex form and its rgb triplet form (0,217,255) both count as drift.
      if (/00d9ff/i.test(text) || /rgba?\(\s*0\s*,\s*217\s*,\s*255/.test(text)) {
        offenders.push(file);
      }
    }
    expect(offenders).toEqual([]);
  });
});
