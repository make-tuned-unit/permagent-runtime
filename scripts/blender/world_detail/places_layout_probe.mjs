// Lane W4 helper: read the *real* runtime layout (habitatLayout.ts and
// habitatObservatoryWing.ts, read-only) by bundling them in memory with the
// esbuild already in ui/command-center/node_modules, then
//   probe  -> write layout.json (places, stations, beds, solids, a 0.25 m
//             walkability grid using the runtime's own habitatWalkable()).
//   check  -> re-run walkability + flood-fill reachability from HABITAT_START
//             with W4's proposed blocking envelopes added, and report every
//             destination that becomes unreachable plus per-envelope route
//             clearance. Nothing in ui/ is written.
// usage: node places_layout_probe.mjs probe <out.json>
//        node places_layout_probe.mjs check <layout.json> <envelopes.json> <report.json>
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../..');
const ui = join(repo, 'ui/command-center');
const require = createRequire(join(ui, 'package.json'));
const esbuild = require('esbuild');
const forum = join(ui, 'src/components/world/forum');

async function loadLayout() {
  const entry = `export * from ${JSON.stringify(join(forum, 'habitatLayout.ts'))};
export * as wing from ${JSON.stringify(join(forum, 'habitatObservatoryWing.ts'))};`;
  const res = await esbuild.build({
    stdin: { contents: entry, resolveDir: forum, loader: 'ts' }, bundle: true, write: false,
    format: 'esm', platform: 'node', nodePaths: [join(ui, 'node_modules')], logLevel: 'error',
    define: { 'import.meta.env.BASE_URL': '"/"' },
  });
  const dir = mkdtempSync(join(tmpdir(), 'w4-layout-'));
  const file = join(dir, 'layout.mjs');
  writeFileSync(file, res.outputFiles[0].text);
  try { return await import(pathToFileURL(file).href); } finally { rmSync(dir, { recursive: true, force: true }); }
}

const STEP = 0.25, MARGIN = 0.47;   // runtime habitatWalkable default body margin
const BOUNDS = { x0: -17, x1: 19, z0: -56, z1: 17 };

function grid(L, extra = []) {
  const nx = Math.round((BOUNDS.x1 - BOUNDS.x0) / STEP), nz = Math.round((BOUNDS.z1 - BOUNDS.z0) / STEP);
  const walk = new Uint8Array(nx * nz);
  for (let i = 0; i < nx; i++) for (let k = 0; k < nz; k++) {
    const x = BOUNDS.x0 + (i + .5) * STEP, z = BOUNDS.z0 + (k + .5) * STEP;
    let ok = L.habitatWalkable(x, z);
    if (ok) for (const e of extra) if (Math.abs(x - e.at[0]) < e.size[0] / 2 + MARGIN && Math.abs(z - e.at[2]) < e.size[2] / 2 + MARGIN) { ok = false; break; }
    walk[i * nz + k] = ok ? 1 : 0;
  }
  return { nx, nz, walk };
}
function cellOf(g, x, z) { return [Math.floor((x - BOUNDS.x0) / STEP), Math.floor((z - BOUNDS.z0) / STEP)]; }
function flood(g, start) {
  const seen = new Uint8Array(g.nx * g.nz), q = [];
  const [si, sk] = cellOf(g, start[0], start[2]);
  if (!g.walk[si * g.nz + sk]) return seen;
  seen[si * g.nz + sk] = 1; q.push([si, sk]);
  while (q.length) {
    const [i, k] = q.pop();
    for (const [di, dk] of [[1,0],[-1,0],[0,1],[0,-1]]) {
      const a = i + di, b = k + dk;
      if (a < 0 || b < 0 || a >= g.nx || b >= g.nz) continue;
      const id = a * g.nz + b;
      if (!seen[id] && g.walk[id]) { seen[id] = 1; q.push([a, b]); }
    }
  }
  return seen;
}
function nearestReached(g, seen, x, z, r = 2.0) {
  const [ci, ck] = cellOf(g, x, z), n = Math.ceil(r / STEP);
  let best = null;
  for (let i = ci - n; i <= ci + n; i++) for (let k = ck - n; k <= ck + n; k++) {
    if (i < 0 || k < 0 || i >= g.nx || k >= g.nz || !seen[i * g.nz + k]) continue;
    const px = BOUNDS.x0 + (i + .5) * STEP, pz = BOUNDS.z0 + (k + .5) * STEP, d = Math.hypot(px - x, pz - z);
    if (!best || d < best.d) best = { d, x: px, z: pz };
  }
  return best;
}

const [mode, a1, a2, a3] = process.argv.slice(2);
const L = await loadLayout();
if (mode === 'probe') {
  const g = grid(L);
  const seen = flood(g, L.HABITAT_START);
  let walkable = 0, reached = 0;
  for (let i = 0; i < g.walk.length; i++) { walkable += g.walk[i]; reached += seen[i]; }
  const W = L.wing;
  const out = {
    source: ['ui/command-center/src/components/world/forum/habitatLayout.ts', 'habitatObservatoryWing.ts'],
    note: 'read-only snapshot for W4 placement; W6 owns these files',
    start: L.HABITAT_START, step: STEP, margin: MARGIN, bounds: BOUNDS,
    places: L.HABITAT_PLACES.map(p => ({ id: p.id, name: p.name, position: p.position, agent: p.agent })),
    stations: L.HABITAT_STATIONS.map(s => ({ id: s.id, name: s.name, position: s.position })),
    beds: L.HABITAT_BEDS, solids: L.HABITAT_SOLIDS,
    decorSolids: L.HABITAT_DECOR.filter(p => p.solid).map(p => ({ asset: p.asset, position: p.position, size: L.decorSolidSize(p) })),
    decor: L.HABITAT_DECOR.map(p => ({ asset: p.asset, position: p.position, scale: p.scale, heading: p.heading })),
    wing: { bridge: W.OBSERVATORY_BRIDGE, pad: W.OBSERVATORY_PAD, plinth: W.OBSERVATORY_PLINTH, height: W.OBSERVATORY_WING_HEIGHT, benches: W.OBSERVATORY_BENCHES },
    gardenBridge: { x0: 1, x1: 7, z0: -3.2, z1: -1.2, deckMax: .45 },
    walkableCells: walkable, reachableCells: reached,
    grid: { nx: g.nx, nz: g.nz, reachable: Buffer.from(seen).toString('base64') },
  };
  writeFileSync(a1, JSON.stringify(out));
  console.log(`probe: ${walkable} walkable cells, ${reached} reachable from start`);
} else if (mode === 'check') {
  const layout = JSON.parse(readFileSync(a1, 'utf8'));
  const env = JSON.parse(readFileSync(a2, 'utf8'));
  const blocking = env.envelopes.filter(e => e.blocking);
  const g0 = grid(L), s0 = flood(g0, L.HABITAT_START);
  const g1 = grid(L, blocking), s1 = flood(g1, L.HABITAT_START);
  let r0 = 0, r1 = 0, lost = 0;
  for (let i = 0; i < s0.length; i++) { r0 += s0[i]; r1 += s1[i]; if (s0[i] && !s1[i]) lost++; }
  const dests = [...layout.places.map(p => ({ name: `place ${p.id}`, at: p.position })),
    ...layout.stations.map(s => ({ name: `station ${s.id}`, at: s.position })),
    { name: 'observatory bridge south end', at: [4, 3.4, layout.wing.bridge.z1 - .7] },
    { name: 'observatory bridge north end', at: [4, 3.4, layout.wing.bridge.z0 + .7] },
    { name: 'garden bridge west end', at: [1.4, 0, -2.2] }, { name: 'garden bridge east end', at: [6.6, 0, -2.2] },
    ...(env.destinations || [])];
  const destReport = dests.map(d => {
    const before = nearestReached(g0, s0, d.at[0], d.at[2]), after = nearestReached(g1, s1, d.at[0], d.at[2]);
    return { name: d.name, at: d.at, reachable_before: !!before, reachable_after: !!after,
      nearest_after_m: after ? +after.d.toFixed(2) : null, regression: !!before && (!after || after.d > before.d + 0.6) };
  });
  // Per-envelope clearance: free distance from each envelope edge to the nearest other blocker, sampled
  // along the four sides on the reachable grid (how wide the walkable gap next to it is).
  const clearance = blocking.map(e => {
    let minGap = Infinity;
    const sides = [];
    for (const [dx, dz] of [[1,0],[-1,0],[0,1],[0,-1]]) {
      const ox = e.at[0] + dx * e.size[0] / 2, oz = e.at[2] + dz * e.size[2] / 2;
      let gap = 0, d = STEP / 2;
      // walk outward until we hit a non-walkable cell (without this envelope)
      const others = blocking.filter(o => o !== e);
      while (d < 6) {
        const x = ox + dx * d, z = oz + dz * d;
        let ok = L.habitatWalkable(x, z, 0);
        if (ok) for (const o of others) if (Math.abs(x - o.at[0]) < o.size[0] / 2 && Math.abs(z - o.at[2]) < o.size[2] / 2) { ok = false; break; }
        if (!ok) break; gap = d; d += STEP / 2;
      }
      sides.push(+gap.toFixed(2)); minGap = Math.min(minGap, gap);
    }
    return { name: e.name, sides_free_m: sides, min_side_free_m: +minGap.toFixed(2) };
  });
  const rep = { reachable_cells_before: r0, reachable_cells_after: r1, cells_lost: lost,
    area_lost_m2: +(lost * STEP * STEP).toFixed(2), destinations: destReport,
    regressions: destReport.filter(d => d.regression).map(d => d.name), envelope_clearance: clearance,
    pass: destReport.every(d => !d.regression) };
  writeFileSync(a3, JSON.stringify(rep, null, 2) + '\n');
  console.log(`check: reachable ${r0} -> ${r1} cells (${rep.area_lost_m2} m2 lost); regressions: ${rep.regressions.length ? rep.regressions.join(', ') : 'none'}`);
} else { console.error('usage: probe <out> | check <layout> <envelopes> <report>'); process.exit(2); }
