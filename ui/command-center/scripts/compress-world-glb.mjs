#!/usr/bin/env node
/**
 * Runtime compression for the Solar Forum GLB.
 *
 * Blender exports an uncompressed GLB (~62 MB: float32 geometry plus embedded
 * JPEG textures). That file is too large to ship in git or in the Tauri bundle,
 * and Git LFS is deliberately not used here. This step rewrites it as
 * EXT_meshopt_compression + KHR_mesh_quantization, which three's
 * `MeshoptDecoder` expands on load in both the browser and Node.
 *
 * Two passes, in this order:
 *   1. `jpeg`    — re-encodes the embedded textures with sharp at quality 90.
 *                  Format and resolution are unchanged (JPEG in, JPEG out), so
 *                  no new runtime decoder is needed. Blender's own JPEG writer
 *                  is wasteful; this alone removes ~7 MB.
 *   2. `meshopt` — quantizes and meshopt-encodes geometry. It must run LAST:
 *                  any later gltf-transform pass decodes the meshopt buffers
 *                  and writes them back out uncompressed.
 *
 * Deliberately NOT used: `gltf-transform optimize`. Its defaults flatten the
 * scene graph, join meshes, build palette textures and simplify geometry. The
 * forum's walk collision (walkCollision.ts) and surface pass
 * (ForumSurfaceMaterials.ts) both key off per-mesh material names and exact
 * exported geometry, so every one of those passes would silently break them.
 *
 * Usage: node scripts/compress-world-glb.mjs [input.glb] [output.glb]
 * Defaults to public/world/solar-forum.glb in place.
 */
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, statSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const UI_ROOT = resolve(HERE, '..');
const CLI = join(UI_ROOT, 'node_modules', '.bin', 'gltf-transform');

const TEXTURE_QUALITY = 90;
/** 16-bit positions: ~0.6 cm over the 400 m campus, inside the walk tests' tolerance. */
const POSITION_BITS = 16;

const argv = process.argv.slice(2);
const force = argv.includes('--force');
const [inputArg, outputArg] = argv.filter(arg => !arg.startsWith('--'));
const input = resolve(inputArg ?? join(UI_ROOT, 'public/world/solar-forum.glb'));
const output = resolve(outputArg ?? input);

if (!existsSync(input)) {
  console.error(`world:compress: input not found: ${input}`);
  process.exit(1);
}
if (!existsSync(CLI)) {
  console.error(`world:compress: @gltf-transform/cli is not installed (${CLI} missing). Run \`npm install\` in ui/command-center.`);
  process.exit(1);
}

function run(args) {
  const result = spawnSync(CLI, args, { stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8' });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    process.stderr.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
    throw new Error(`gltf-transform ${args[0]} exited ${result.status}`);
  }
}

// Running this twice over the same file would decode the meshopt buffers and
// put the textures through a second lossy JPEG generation for no size win, so
// an already-compressed input is refused rather than quietly degraded.
const head = readFileSync(input);
const jsonChunk = head.subarray(20, 20 + head.readUInt32LE(12)).toString('utf8');
if (!force && jsonChunk.includes('EXT_meshopt_compression')) {
  console.error(`world:compress: ${input} is already meshopt-compressed. Re-run the Blender builder for a fresh export, or pass --force.`);
  process.exit(1);
}

const scratch = mkdtempSync(join(tmpdir(), 'world-compress-'));
const staged = join(scratch, 'staged.glb');
try {
  const rawBytes = statSync(input).size;
  run(['jpeg', input, staged, '--quality', String(TEXTURE_QUALITY)]);
  run(['meshopt', staged, output, '--level', 'high', '--quantize-position', String(POSITION_BITS)]);
  const bytes = statSync(output).size;
  console.log(JSON.stringify({ input, output, rawBytes, bytes, ratio: +(bytes / rawBytes).toFixed(4) }));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
