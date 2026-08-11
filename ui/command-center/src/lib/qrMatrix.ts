// Dependency-free QR encoder for the one-time device-pairing URL (#628).
// Byte mode, error-correction level L; versions 1-10 cover payloads up to 271
// UTF-8 bytes - comfortably beyond a MagicDNS hostname plus a claim code.
// Extracted from SettingsView so the encoder can be tested independently of
// React: a mis-encoded QR looks perfectly fine on screen and only fails at
// scan time, so qrMatrix.test.ts asserts a real round-trip decode.

const QR_BLOCKS_L: readonly (readonly number[])[] = [
  [19], [34], [55], [80], [108], [68, 68], [78, 78], [97, 97], [116, 116],
  [68, 68, 69, 69],
];
const QR_ECC_L = [7, 10, 15, 20, 26, 18, 20, 24, 30, 18] as const;

function qrMultiply(x: number, y: number) {
  let z = 0;
  for (let i = 7; i >= 0; i--) {
    z = (z << 1) ^ ((z >>> 7) * 0x11d);
    z ^= ((y >>> i) & 1) * x;
  }
  return z;
}

function qrDivisor(degree: number) {
  const result = Array<number>(degree).fill(0);
  result[degree - 1] = 1;
  let root = 1;
  for (let i = 0; i < degree; i++) {
    for (let j = 0; j < result.length; j++) {
      result[j] = qrMultiply(result[j], root);
      if (j + 1 < result.length) result[j] ^= result[j + 1];
    }
    root = qrMultiply(root, 2);
  }
  return result;
}

function qrRemainder(data: readonly number[], divisor: readonly number[]) {
  const result = Array<number>(divisor.length).fill(0);
  for (const byte of data) {
    const factor = byte ^ result.shift()!;
    result.push(0);
    for (let i = 0; i < result.length; i++) result[i] ^= qrMultiply(divisor[i], factor);
  }
  return result;
}

function qrAlignmentPositions(version: number, size: number) {
  if (version === 1) return [];
  const count = Math.floor(version / 7) + 2;
  const step = version === 32
    ? 26
    : Math.floor((version * 4 + count * 2 + 1) / (count * 2 - 2)) * 2;
  const positions = [6];
  for (let pos = size - 7; positions.length < count; pos -= step) positions.splice(1, 0, pos);
  return positions;
}

export function makeQrMatrix(text: string): boolean[][] {
  const bytes = Array.from(new TextEncoder().encode(text));
  const versionIndex = QR_BLOCKS_L.findIndex((blocks, i) => {
    const countBits = i < 9 ? 8 : 16;
    return 4 + countBits + bytes.length * 8 <= blocks.reduce((a, b) => a + b, 0) * 8;
  });
  if (versionIndex < 0) throw new Error('Pairing URL is too long to encode as a QR code');
  const version = versionIndex + 1;
  const dataLengths = QR_BLOCKS_L[versionIndex];
  const dataCapacity = dataLengths.reduce((a, b) => a + b, 0);
  const bits: number[] = [];
  const appendBits = (value: number, length: number) => {
    for (let i = length - 1; i >= 0; i--) bits.push((value >>> i) & 1);
  };
  appendBits(0b0100, 4);
  appendBits(bytes.length, version < 10 ? 8 : 16);
  bytes.forEach(byte => appendBits(byte, 8));
  appendBits(0, Math.min(4, dataCapacity * 8 - bits.length));
  while (bits.length % 8) bits.push(0);
  const data: number[] = [];
  for (let i = 0; i < bits.length; i += 8) {
    data.push(bits.slice(i, i + 8).reduce((value, bit) => (value << 1) | bit, 0));
  }
  for (let pad = 0; data.length < dataCapacity; pad++) data.push(pad % 2 ? 0x11 : 0xec);

  const divisor = qrDivisor(QR_ECC_L[versionIndex]);
  const blocks: { data: number[]; ecc: number[] }[] = [];
  let offset = 0;
  dataLengths.forEach(length => {
    const block = data.slice(offset, offset + length);
    blocks.push({ data: block, ecc: qrRemainder(block, divisor) });
    offset += length;
  });
  const codewords: number[] = [];
  for (let i = 0; i < Math.max(...dataLengths); i++) {
    blocks.forEach(block => { if (i < block.data.length) codewords.push(block.data[i]); });
  }
  for (let i = 0; i < divisor.length; i++) blocks.forEach(block => codewords.push(block.ecc[i]));

  const size = version * 4 + 17;
  const base = Array.from({ length: size }, () => Array<boolean>(size).fill(false));
  const isFunction = Array.from({ length: size }, () => Array<boolean>(size).fill(false));
  const setFunction = (x: number, y: number, dark: boolean) => {
    if (x >= 0 && x < size && y >= 0 && y < size) {
      base[y][x] = dark;
      isFunction[y][x] = true;
    }
  };
  const drawFinder = (cx: number, cy: number) => {
    for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) {
      const distance = Math.max(Math.abs(dx), Math.abs(dy));
      setFunction(cx + dx, cy + dy, distance !== 2 && distance !== 4);
    }
  };
  drawFinder(3, 3);
  drawFinder(size - 4, 3);
  drawFinder(3, size - 4);
  for (let i = 0; i < size; i++) {
    if (!isFunction[6][i]) setFunction(i, 6, i % 2 === 0);
    if (!isFunction[i][6]) setFunction(6, i, i % 2 === 0);
  }
  const align = qrAlignmentPositions(version, size);
  align.forEach(x => align.forEach(y => {
    if (isFunction[y][x]) return;
    for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) {
      setFunction(x + dx, y + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
    }
  }));
  // Reserve format/version cells before placing payload bits.
  for (let i = 0; i < 9; i++) {
    if (i !== 6) { setFunction(8, i, false); setFunction(i, 8, false); }
  }
  for (let i = 0; i < 8; i++) {
    setFunction(size - 1 - i, 8, false);
    setFunction(8, size - 1 - i, false);
  }
  setFunction(8, size - 8, true);
  if (version >= 7) {
    let rem = version;
    for (let i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >>> 11) * 0x1f25);
    const versionBits = (version << 12) | rem;
    for (let i = 0; i < 18; i++) {
      const dark = ((versionBits >>> i) & 1) !== 0;
      const a = size - 11 + (i % 3);
      const b = Math.floor(i / 3);
      setFunction(a, b, dark);
      setFunction(b, a, dark);
    }
  }

  let bitIndex = 0;
  let upward = true;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right--;
    for (let vert = 0; vert < size; vert++) {
      const y = upward ? size - 1 - vert : vert;
      for (let j = 0; j < 2; j++) {
        const x = right - j;
        if (!isFunction[y][x]) {
          const byte = codewords[bitIndex >>> 3];
          base[y][x] = byte !== undefined && ((byte >>> (7 - (bitIndex & 7))) & 1) !== 0;
          bitIndex++;
        }
      }
    }
    upward = !upward;
  }

  const maskBit = (mask: number, x: number, y: number) => {
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
  const withFormat = (mask: number) => {
    const modules = base.map(row => row.slice());
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
      if (!isFunction[y][x] && maskBit(mask, x, y)) modules[y][x] = !modules[y][x];
    }
    const formatData = (1 << 3) | mask; // Error-correction level L.
    let rem = formatData;
    for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
    const format = ((formatData << 10) | rem) ^ 0x5412;
    const bit = (i: number) => ((format >>> i) & 1) !== 0;
    for (let i = 0; i <= 5; i++) modules[i][8] = bit(i);
    modules[7][8] = bit(6); modules[8][8] = bit(7); modules[8][7] = bit(8);
    for (let i = 9; i < 15; i++) modules[8][14 - i] = bit(i);
    for (let i = 0; i < 8; i++) modules[8][size - 1 - i] = bit(i);
    for (let i = 8; i < 15; i++) modules[size - 15 + i][8] = bit(i);
    modules[size - 8][8] = true;
    return modules;
  };
  const penalty = (modules: boolean[][]) => {
    let score = 0;
    const lines = [...modules, ...Array.from({ length: size }, (_, x) => modules.map(row => row[x]))];
    lines.forEach(line => {
      let run = 1;
      for (let i = 1; i < size; i++) {
        if (line[i] === line[i - 1]) { run++; if (run === 5) score += 3; else if (run > 5) score++; }
        else run = 1;
      }
      const compact = line.map(v => v ? '1' : '0').join('');
      score += (compact.match(/10111010000/g)?.length ?? 0) * 40;
      score += (compact.match(/00001011101/g)?.length ?? 0) * 40;
    });
    for (let y = 0; y < size - 1; y++) for (let x = 0; x < size - 1; x++) {
      const value = modules[y][x];
      if (modules[y][x + 1] === value && modules[y + 1][x] === value && modules[y + 1][x + 1] === value) score += 3;
    }
    const dark = modules.flat().filter(Boolean).length;
    score += Math.floor(Math.abs(dark * 20 - size * size * 10) / (size * size)) * 10;
    return score;
  };
  return Array.from({ length: 8 }, (_, mask) => withFormat(mask))
    .reduce((best, candidate) => penalty(candidate) < penalty(best) ? candidate : best);
}
