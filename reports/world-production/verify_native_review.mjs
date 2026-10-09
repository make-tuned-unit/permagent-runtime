// Verify the actual embedded frontend, including native (root-relative) URLs.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { brotliDecompressSync } from 'node:zlib';

const recordReview = process.argv.includes('--record-review');
if (process.argv.slice(2).some(arg => arg !== '--record-review')) throw new Error('Only --record-review is supported');
const identifier = recordReview ? 'ai.permagent.record-review-20261007' : 'ai.permagent.world-review-20261006';
const productName = recordReview ? 'Permagent Record Review' : 'Permagent World Review';
const lane = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const target = process.env.PERMAGENT_BUILD_TARGET || path.resolve(lane, '../.shared-target/world-production-20261006');
const dist = path.join(lane, 'ui/command-center/dist');
const binary = fs.readFileSync(path.join(target, 'debug/permagent-app'));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const files = dir => fs.readdirSync(dir, { withFileTypes: true }).flatMap(e =>
  e.isDirectory() ? files(path.join(dir, e.name)) : [path.join(dir, e.name)]);
const generated = fs.readdirSync(path.join(target, 'debug/build'))
  .filter(name => name.startsWith('permagent-app-'))
  .map(name => path.join(target, 'debug/build', name, 'out/tauri-codegen-assets'))
  .filter(dir => fs.existsSync(dir)).flatMap(files);
const embedded = new Map();
const html = [];
for (const filename of generated) {
  const compressed = fs.readFileSync(filename);
  let decoded;
  try { decoded = brotliDecompressSync(compressed); } catch { continue; }
  // Stale codegen files can survive a rebuild. Count only bytes in this binary.
  if (!binary.includes(compressed)) continue;
  embedded.set(sha(decoded), true);
  if (filename.endsWith('.html')) html.push(decoded.toString());
}
const exact = [], transformed = [];
const javascript = [];
for (const filename of files(dist)) {
  const relative = path.relative(dist, filename);
  const data = fs.readFileSync(filename);
  if (filename.endsWith('.html')) {
    const source = data.toString();
    const refs = [...source.matchAll(/(?:src|href)="(\/(?:ui\/)?assets\/[^"\s]+)"/g)].map(m => m[1]);
    if ((relative === 'index.html' && refs.length === 0) || refs.some(ref => ref.startsWith('/ui/'))) {
      throw new Error(`Incorrect native asset base in ${relative}: ${refs}`);
    }
    const title = source.match(/<title>([^<]+)<\/title>/i)?.[1];
    const scripts = [...source.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)].map(m => m[1].trim()).filter(Boolean);
    if (!title || !html.some(candidate => candidate.includes(title) && [...refs, ...scripts].every(ref => candidate.includes(ref)))) {
      throw new Error(`Current HTML entry references are not embedded: ${relative}`);
    }
    transformed.push(relative);
  } else {
    const hash = sha(data);
    if (!embedded.has(hash)) throw new Error(`Current asset is not embedded: ${relative}`);
    exact.push({ path: relative, sha256: hash, bytes: data.length });
    if (filename.endsWith('.js')) javascript.push({ path: relative, text: data.toString() });
  }
}
const configName = recordReview ? 'native-record-review-config.json' : 'native-review-config.json';
const config = JSON.parse(fs.readFileSync(path.join(lane, 'reports/world-production', configName), 'utf8'));
const reviewRoot = path.join(target, 'record-qa-runtime');
const frontendTargetSeen = javascript.some(asset => asset.text.includes('http://127.0.0.1:3002'));
const productionEndpoints = javascript.filter(asset => /(?:http|ws):\/\/(?:localhost|127\.0\.0\.1):3001(?:["'`/]|\b)/.test(asset.text)).map(asset => asset.path);
if (recordReview) {
  const csp = config.app?.security?.csp ?? '';
  if (config.identifier !== identifier || csp.includes(':3001') || !csp.includes('http://127.0.0.1:3002') || !csp.includes('ws://127.0.0.1:3002')) throw new Error('Record review CSP/identity mismatch');
  if (!binary.includes(csp)) throw new Error('Exact Record review CSP is not embedded in native shell');
  if (!binary.includes(reviewRoot)) throw new Error('Compiled isolated root is missing from native shell');
  if (!frontendTargetSeen || productionEndpoints.length) throw new Error(`Record frontend target mismatch; production endpoint literals: ${productionEndpoints.join(', ')}`);
}
const report = {
  record_review: recordReview,
  identifier,
  review_config_sha256: sha(fs.readFileSync(path.join(lane, 'reports/world-production', configName))),
  compiled_review_root: recordReview ? reviewRoot : null,
  embedded_review_csp: recordReview ? binary.includes(config.app.security.csp) : null,
  frontend_api_base: recordReview ? 'http://127.0.0.1:3002' : 'http://127.0.0.1:3001',
  frontend_production_endpoint_assets: recordReview ? productionEndpoints : null,
  native_build_sha256: sha(binary),
  embedded_review_identifier: binary.includes(identifier),
  embedded_review_name: binary.includes(productName),
  native_asset_base: '/',
  verified_exact_dist_assets: exact,
  transformed_html_assets: transformed,
  method: 'SHA256-match Brotli-decoded codegen assets to dist, require compressed bytes in current binary, and verify native HTML asset references.',
};
if (!report.embedded_review_identifier || !report.embedded_review_name) throw new Error('Review identity missing');
fs.writeFileSync(path.join(lane, 'reports/world-production', recordReview ? 'native-record-embedded-assets-verification.json' : 'native-embedded-assets-verification.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({ binary: report.native_build_sha256, exactAssets: exact.length, html: transformed, assetBase: '/' }));
