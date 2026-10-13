// Validate every W3 GLB with the Khronos glTF-Validator bundled in
// ui/command-center/node_modules (no install, no network). Writes
// <out>/validation.json and exits non-zero on any error.
//   node scripts/blender/world_detail/validate_glb.mjs [outDir]
import { createRequire } from 'node:module';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../..');
const require = createRequire(join(repo, 'ui/command-center/package.json'));
const validator = require('gltf-validator');
const out = resolve(process.argv[2] ?? join(repo, 'assets/world/detail-v4/hero'));
const files = readdirSync(out).filter(f => f.endsWith('.glb')).sort();
const results = [];
let failed = 0;
for (const f of files) {
  const report = await validator.validateBytes(new Uint8Array(readFileSync(join(out, f))), { maxIssues: 50 });
  const { numErrors, numWarnings, numInfos, numHints } = report.issues;
  if (numErrors) failed++;
  results.push({
    file: f, errors: numErrors, warnings: numWarnings, infos: numInfos, hints: numHints,
    messages: report.issues.messages.filter(m => m.severity <= 1).map(m => `${m.code}: ${m.message} @ ${m.pointer ?? ''}`),
  });
  console.log(`${numErrors ? 'FAIL' : 'ok  '} ${f} errors=${numErrors} warnings=${numWarnings}`);
}
writeFileSync(join(out, 'validation.json'), JSON.stringify({
  validator: `gltf-validator ${validator.version()}`, files: results, allValid: failed === 0,
}, null, 2) + '\n');
process.exit(failed ? 1 : 0);
