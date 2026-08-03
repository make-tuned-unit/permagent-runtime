import { describe, it, expect } from 'vitest';
import { makeQrMatrix } from './qrMatrix';

/**
 * A hand-rolled QR encoder fails in a uniquely nasty way: the output looks like
 * a perfectly plausible QR code on screen and only reveals itself as garbage
 * when a phone refuses to scan it. Structural assertions (right size, finder
 * patterns present) would pass on a code whose payload bits are wrong, so they
 * are not enough on their own.
 *
 * These tests therefore pin the structure AND the byte-level encoding: the
 * data codewords are re-derived from the spec independently of the matrix
 * writer, so a bug in bit packing shows up here. The end-to-end proof that a
 * real scanner decodes the payload is a decode of the rendered matrix by
 * CoreImage; see `qrDecode.verify.md` for that run and its output.
 */

const PAIRING_URL = 'https://jesses-mac-mini-2.tail4429f1.ts.net:8443/pair?claim=8f3c1d2e9a7b4c60';

function moduleAt(m: boolean[][], x: number, y: number) {
  return m[y][x];
}

/** A finder pattern is a 7x7 ring: dark border, light gap, 3x3 dark core. */
function hasFinderPattern(m: boolean[][], ox: number, oy: number) {
  for (let dy = 0; dy < 7; dy++) {
    for (let dx = 0; dx < 7; dx++) {
      const ring = Math.max(Math.abs(dx - 3), Math.abs(dy - 3));
      const expected = ring !== 2;
      if (moduleAt(m, ox + dx, oy + dy) !== expected) return false;
    }
  }
  return true;
}

describe('makeQrMatrix', () => {
  it('produces a square matrix whose size is a legal QR version', () => {
    const m = makeQrMatrix(PAIRING_URL);
    expect(m.length).toBeGreaterThan(0);
    m.forEach(row => expect(row.length).toBe(m.length));
    // size = version * 4 + 17, versions 1-10 => 21..57
    expect((m.length - 17) % 4).toBe(0);
    const version = (m.length - 17) / 4;
    expect(version).toBeGreaterThanOrEqual(1);
    expect(version).toBeLessThanOrEqual(10);
  });

  it('picks the smallest version that fits the payload', () => {
    // 'A'.repeat(19) needs version 1 (19 data codewords at ECC-L) minus the
    // 2-byte mode+length header, so 17 bytes is the version-1 ceiling.
    expect(makeQrMatrix('A'.repeat(17)).length).toBe(21); // version 1
    expect(makeQrMatrix('A'.repeat(18)).length).toBe(25); // spills to version 2
  });

  it('places all three finder patterns', () => {
    const m = makeQrMatrix(PAIRING_URL);
    const n = m.length;
    expect(hasFinderPattern(m, 0, 0)).toBe(true);
    expect(hasFinderPattern(m, n - 7, 0)).toBe(true);
    expect(hasFinderPattern(m, 0, n - 7)).toBe(true);
  });

  it('draws both timing patterns as alternating modules', () => {
    const m = makeQrMatrix(PAIRING_URL);
    for (let i = 8; i < m.length - 8; i++) {
      expect(moduleAt(m, i, 6)).toBe(i % 2 === 0);
      expect(moduleAt(m, 6, i)).toBe(i % 2 === 0);
    }
  });

  it('sets the always-dark module below the top-left finder', () => {
    const m = makeQrMatrix(PAIRING_URL);
    expect(moduleAt(m, 8, m.length - 8)).toBe(true);
  });

  it('is deterministic for a given payload', () => {
    const a = makeQrMatrix(PAIRING_URL);
    const b = makeQrMatrix(PAIRING_URL);
    expect(a).toEqual(b);
  });

  it('encodes different payloads differently', () => {
    const a = makeQrMatrix('https://example.test/pair?claim=aaaa');
    const b = makeQrMatrix('https://example.test/pair?claim=bbbb');
    expect(a).not.toEqual(b);
  });

  it('round-trips the payload bits back out of the matrix', () => {
    // Read the data region back using the spec's zig-zag order and the mask
    // recovered from the format bits, then decode mode + length + bytes. This
    // walks the matrix independently of the writer, so a placement or masking
    // bug produces a mismatched payload here.
    const m = makeQrMatrix(PAIRING_URL);
    const size = m.length;
    const version = (size - 17) / 4;

    // Recover the mask from the top-left format information.
    const formatBit = (i: number) => {
      if (i <= 5) return m[i][8];
      if (i === 6) return m[7][8];
      if (i === 7) return m[8][8];
      if (i === 8) return m[8][7];
      return m[8][14 - i];
    };
    let format = 0;
    for (let i = 14; i >= 0; i--) format = (format << 1) | (formatBit(i) ? 1 : 0);
    format ^= 0x5412;
    const mask = (format >>> 10) & 0b111;

    // Rebuild the function-module map so we skip the same cells the writer did.
    const isFunction = Array.from({ length: size }, () => Array<boolean>(size).fill(false));
    const reserve = (x: number, y: number) => {
      if (x >= 0 && x < size && y >= 0 && y < size) isFunction[y][x] = true;
    };
    const reserveFinder = (cx: number, cy: number) => {
      for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) reserve(cx + dx, cy + dy);
    };
    reserveFinder(3, 3);
    reserveFinder(size - 4, 3);
    reserveFinder(3, size - 4);
    for (let i = 0; i < size; i++) { reserve(i, 6); reserve(6, i); }
    // Alignment patterns.
    if (version > 1) {
      const count = Math.floor(version / 7) + 2;
      const step = Math.floor((version * 4 + count * 2 + 1) / (count * 2 - 2)) * 2;
      const positions = [6];
      for (let pos = size - 7; positions.length < count; pos -= step) positions.splice(1, 0, pos);
      positions.forEach(x => positions.forEach(y => {
        const nearFinder = (x <= 8 && y <= 8)
          || (x <= 8 && y >= size - 9)
          || (x >= size - 9 && y <= 8);
        if (nearFinder) return;
        for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) reserve(x + dx, y + dy);
      }));
    }
    for (let i = 0; i < 9; i++) { if (i !== 6) { reserve(8, i); reserve(i, 8); } }
    for (let i = 0; i < 8; i++) { reserve(size - 1 - i, 8); reserve(8, size - 1 - i); }
    if (version >= 7) {
      for (let i = 0; i < 18; i++) {
        const a = size - 11 + (i % 3);
        const b = Math.floor(i / 3);
        reserve(a, b);
        reserve(b, a);
      }
    }

    const maskBit = (x: number, y: number) => {
      switch (mask) {
        case 0: return (x + y) % 2 === 0;
        case 1: return y % 2 === 0;
        case 2: return x % 3 === 0;
        case 3: return (x + y) % 3 === 0;
        case 4: return (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0;
        case 5: return (x * y) % 2 + (x * y) % 3 === 0;
        case 6: return ((x * y) % 2 + (x * y) % 3) % 2 === 0;
        default: return ((x + y) % 2 + (x * y) % 3) % 2 === 0;
      }
    };

    const bits: number[] = [];
    let upward = true;
    for (let right = size - 1; right >= 1; right -= 2) {
      if (right === 6) right--;
      for (let vert = 0; vert < size; vert++) {
        const y = upward ? size - 1 - vert : vert;
        for (let j = 0; j < 2; j++) {
          const x = right - j;
          if (!isFunction[y][x]) {
            bits.push((m[y][x] !== maskBit(x, y)) ? 1 : 0);
          }
        }
      }
      upward = !upward;
    }

    const take = (n: number) => {
      let v = 0;
      for (let i = 0; i < n; i++) v = (v << 1) | bits.shift()!;
      return v;
    };
    expect(take(4)).toBe(0b0100); // byte mode
    const expected = new TextEncoder().encode(PAIRING_URL);
    const length = take(version < 10 ? 8 : 16);
    expect(length).toBe(expected.length);
    const decoded = Array.from({ length }, () => take(8));
    expect(new TextDecoder().decode(Uint8Array.from(decoded))).toBe(PAIRING_URL);
  });

  it('refuses payloads too long for version 10 rather than emitting a broken code', () => {
    expect(() => makeQrMatrix('x'.repeat(400))).toThrow(/too long/i);
  });
});
