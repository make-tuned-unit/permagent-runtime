# W3 Cave Detail-Pass Evidence — Tending, Librarian Mining, Henry Site-Walk, Construction

Re-spec source: `THE_CAVE_vision_bible.md` §4 (the third agent state: tending; the
construction system; Henry presides, never lifts) and §5 (the Librarian's mining/describe
loop). Engineering law: `WORLD_VIEW_BIBLE.md` §4 (sim-state clamp), §2/§8 (HUD color law),
§6 (frozen contracts), §8 (perf budget — 0 per-frame allocations, ≤ 12 DC/agent).

Captured against an ISOLATED daemon (`PERMAGENT_PATH_ROOT=/tmp/world-baseline/w3/daemon-root`,
port 3011, permagentd 1.31.0) behind the kill-switch TCP proxy (3010 → 3011). The live
daemon on 3001 was NEVER touched. Vite on its own port (5381); other lanes' servers left
running. The page reached the daemon only through the proxy (evidence-only vite proxy
retarget to :3010, reverted before commit).

## The tending bank — what REAL events feed it

The third agent state (tending) and all construction are driven by a **bank of real
banked describe/ingest events**, never a usage progress bar (bible §4). The bank
(`shared/tendingBank.ts`) accepts a deposit ONLY from:

| Real daemon event (`/events` wire type) | Banks as | Meaning (bible §4) |
|---|---|---|
| `librarian_describe_completed` | one `describe` unit | a quarried stone (a described memory) |
| `memory_added` | one `ingest` unit | ore (an ingested memory) |

Replayed `/events` history (timestamp before page mount) is filtered in `stateSources.tsx`
and never banks — the bank only counts work seen live, so a stale buffer can never fake it.
**No bank ⇒ idle, NOT tending.** When the bank empties, tenders stand down and resume idle
wander; construction stops rising. The dev time-lapse driver deposits through the SAME door
but tags every unit `source: 'dev'` (never `daemon`) — provenance is in the `bank` ledger
lines so a time-lapse capture can never be read as a real run.

## 1. Librarian mining loop — REAL describe run (`detail-mining/`)

`events.log` records a real chat turn (creates a describable Brain memory; Henry → working)
then `POST /api/librarian/run-now` — a real qwen2.5 describe run on the isolated daemon.
The live (page-mount-or-later timestamp) `librarian_describe_*` events drive the mining
loop (`librarianMining.ts`): pull a dim tablet → brighten it violet during the describe →
reshelve it glowing on `_completed` (bible §5). Each `_completed` also banks one `describe`
unit — the `bank` ledger lines show the level climbing as real stones are quarried.

- `01-mining-henry-available` · `02-mining-henry-working` — Henry leaning at a W2 anchor
  during the real reply stream (state `daemon`).
- `03-mining-librarian-idle` — no tablet (idle until first live event).
- `04-mining-librarian-working-shelf` · `04-mining-librarian-describe-2` — the Librarian on
  the mezzanine in the working/amber register (amber visor + cape circuits + feet-aura ring),
  the standing lean-in at the shelf during the real `describing` window.
- `05-mining-librarian-reshelved` · `06-mining-librarian-final` — after
  `librarian_describe_completed`.
- `events-mining.log` proves it: live `librarian_describe_completed` for
  `chat-20260615_3-2` (ts 2026-06-15T22:43, after page mount) → `bank after-describe`
  `totalBanked:1` unit `source:'daemon'` ref `chat-20260615_3-2` → construction `built:1`.
  (An earlier run of the same script banked 5 daemon units → built 5 — same mechanism, more
  material; this committed run banks the one memory the isolated Brain had newly to describe.
  See `bank-ledger-excerpt.log`.) The `events-ws` lines with 2026-06-11..13 timestamps are
  the daemon `/events` buffer replay — filtered in `stateSources.tsx`, they never bank.

## 2. Tending sequence + bank ledger (`detail-tending/`)

Demonstrates the tending → construction loop at demo speed via the DEV time-lapse (bible
§4/§7: "Demos use the time-lapse, not accelerated live rates"). The `bank` ledger lines are
dumped alongside the frames — every unit is `source: 'dev'`, the construction `built` count
climbs as the bank is spent, and the scaffold stones rise (`tickConstruction` spends one
banked unit per stage, gated by a cooldown so growth stays unhurried).

- `10-tending-wide` (sequence) — idle workers walk to a construction tend anchor and tend in
  the gray-warm register (NOT amber); scaffold courses rise as the bank is spent.
- `11-tending-closeup` — a tending agent's body language + warm-gray state channels.
- `12-construction-build-site` — the scaffold + set stones at a build footprint.
- ledger: `t0-before` (bank/built) → `t-final` (bank drained, stones set), all `dev`-tagged.

## 3. Henry site-walk (`detail-sitewalk/`)

Henry presides — he walks the site, never engages a tend anchor, never lifts. A soft warm
floor pool gathers where he stands (brighter when he stops to inspect, fading while
walking). `henryPresence.ts` publishes his live position every frame for W4 to gather light
there (the W3 → W4 coordinate); `window.__worldHenry()` shows the published read.

- `20-henry-sitewalk` (sequence) — Henry strolling, light pool tracking his feet.
- `21-henry-presence-light` — the gathered pool close-up.

## 4. Four HUD states still correct (`detail-states/` + §1 frames)

The detail pass did not regress the four-state machine: Henry shows
idle/available/working/error from real daemon signals (the working frames in `detail-mining`
are a real reply stream); sim agents (Aria/Nova/Felix) stay clamped to gray/cyan — the
tending register is a SEPARATE warm-gray channel and is never amber (bible §4/§8).

## Perf (bible §8)

`perf` lines in each `events.log` report fps / draw calls / triangles / geometries from the
shared `PerfSampler`. Construction adds ~3 draw calls total (instanced poles + survey + one
rising-stones InstancedMesh across both sites). Per-agent draw calls unchanged at ≤ 6
(Henry 6 incl. crown + presence light; Librarian 6 incl. tablet; others 5). Geometry count
is flat across each capture (tablet/light geo owned per-rig and disposed; no leak). Zero
per-frame allocations — tending sway, the mining tablet, Henry's pool, and the construction
rise all use module-level scratch / scalar math only.

## Files

- `detail-mining/` — real describe run: mining loop + bank fill (frames + events.log)
- `detail-tending/` — tending + construction via dev time-lapse, bank ledger dumped
- `detail-sitewalk/` — Henry site-walk + presence light
- `detail-states/` — four-state re-verify
- each `events.log`: `events-ws` (real wire events), `henry-poll`/`librarian-poll`,
  `bank` (the ledger with provenance), `perf`, `meshcount`, `frame`
