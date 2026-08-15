import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const [key, raw] = process.argv.slice(2);
const measured = Number(raw);
if (!Number.isFinite(measured)) {
  console.error(`Invalid measured percentage: ${raw ?? '(missing)'}`);
  process.exit(1);
}
const root = fileURLToPath(new URL('../', import.meta.url));
const baseline = JSON.parse(readFileSync(`${root}.github/coverage-baseline.json`, 'utf8'));
const expected = baseline[key];
if (!Number.isFinite(expected)) {
  console.error(`Unknown or invalid coverage baseline key: ${key ?? '(missing)'}`);
  process.exit(1);
}
const delta = measured - expected;
console.log(`${key}: measured ${measured.toFixed(2)}%, baseline ${expected.toFixed(2)}%, delta ${delta.toFixed(2)}pp`);
console.log(`::notice::${key} measured coverage: ${measured.toFixed(2)}%`);
if (measured < expected - baseline.tolerance_pct) {
  console.error(`${key} regressed beyond tolerance; restore coverage or, if intentional, lower the baseline in the same PR.`);
  process.exit(1);
}
if (measured > expected + baseline.tolerance_pct)
  console.log(`::notice::Raise ${key} baseline to ${measured.toFixed(2)}% to advance the coverage ratchet.`);
