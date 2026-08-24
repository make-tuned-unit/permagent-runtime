// Per-agent phase offset — WORLD_VIEW_BIBLE.md §4/§8 (desynced idle life).
// Every agent's idle sway, breathing, blink, and head-nod ultimately read from
// the SAME shared r3f clock (`useFrame`'s elapsed time), because that clock is
// the only per-frame time source the scene has and reusing it is what keeps
// this file allocation-free. Left alone, seven `Math.sin(t * k)` calls with the
// same `k` produce seven agents breathing and rocking in perfect lockstep,
// which reads as one machine wearing seven bodies rather than seven people.
//
// The fix is a per-agent phase offset baked into every sine argument
// (`Math.sin(t * k + phase)`), and the offset has to come from somewhere
// stable — NOT `Math.random()`, which would reshuffle on every reload and make
// two runs of the same scene look different for no reason, and would make
// evidence recordings (bible §8 "every lane PR includes before/after ... from
// shared/perf.ts") non-reproducible. Hashing the agent's id gives a value
// that's stable across reloads, stable across machines, and unique enough
// per id that seven ids land at different points around the circle.
//
// The id is the stable KEY here — 'henry', 'librarian', 'reader', 'watcher',
// 'steward', 'strix', 'financier' (roster.ts) — never the display name. A
// display name can be renamed by the user (Henry's is; see roster.ts's
// comment on `/api/agent/identity`); this hash must keep producing the same
// phase for 'henry' no matter what name currently sits on top of that id, so
// an agent's breathing/blink rhythm doesn't jump the moment someone renames it.

/**
 * FNV-1a — a small, well-distributed, non-cryptographic string hash. There is
 * no security requirement here, only "different id strings land in different
 * places," which FNV-1a gives cheaply and deterministically. Exported mainly
 * so the phase function's own unit test can assert on the hash directly.
 */
export function hashId(id: string): number {
  let h = 0x811c9dc5; // FNV offset basis
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0; // FNV prime, wrapped to uint32
  }
  return h >>> 0;
}

/**
 * Stable phase offset in [0, 2π) for an agent id. Pure and deterministic:
 * calling this twice with the same id — in the same session, after a reload,
 * on a different machine — always returns the same number. Feed the result
 * straight into a sine argument alongside the shared clock: `Math.sin(t * k +
 * getIdPhase(id))`.
 */
export function getIdPhase(id: string): number {
  return (hashId(id) / 0xffffffff) * Math.PI * 2;
}
