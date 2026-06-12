# W3 Evidence — Agent Characters Bound to Real Daemon State

Captured 2026-06-12 against an ISOLATED daemon (`PERMAGENT_PATH_ROOT=/tmp/world-baseline/w3/daemon-root`,
port 3011) behind a kill-switch TCP proxy (3010) — the live daemon was never touched. Every state
transition below was driven by a real daemon event; the matching wire/log lines are in the
`events-*.log` files (UTC timestamps, same clock as the frame filenames' epoch-ms).

## All four states, real events

| still | state | driven by | log line (events-record.log / events-phaseE.log) |
|---|---|---|---|
| 02-henry-available-closeup | available | `/api/henry/status` poll: `idle` | `00:20:02 [henry-poll] current_state=idle` |
| 03-henry-working-seated | working (seated lean-in) | real chat turn on session `20260612_2` | `00:20:45 [henry-poll] current_state=in_conversation` (reply stream `Finish=true`) |
| 04/05-henry-error-flicker-off/on | error (2 Hz visor flicker) | proxy kill-switch → real fetch failure | `00:38:55 [action] phaseE: touching proxy-off` |
| 06-henry-error-steady | error (slump, 20° head bow, red channels) | same severed window | `E-error-steady` frame 26 s after sever |
| 07-henry-recovered | available again (cyan cape grid + aura) | proxy restored → poll succeeds | `00:39:21 [action] phaseE: removing proxy-off` |
| 08-librarian-idle | idle (gray underfoot ring) | no event yet — "idle until first event" | frame at 00:22:35, before run-now |
| 09-librarian-working | working (amber visor + aura) | `/events` WS: `librarian_describe_started` | `00:24:21 [events-ws] librarian_describe_started … chat-20260612_1-2` |
| 10-librarian-available | available (cyan ring), 3 s after completion | `/events` WS: `librarian_describe_completed` | `00:25:45 [events-ws] librarian_describe_completed` |

The Librarian describe run was real Ollama work (`run-now -> {"success":true,"model":"qwen2.5:7b"}`,
`state=describing "Describing memory 1 of ~3"` in the same log). The `events-ws` lines with
2026-06-11 timestamps at connect time are the daemon's `/events` buffer replay — the page skips
replayed history (timestamp < mount) so the Librarian stays idle until the first live event.

## Sim agents stay gray/cyan (the LAW, bible §4)

`11-sim-wide-wander`, `12-sim-felix-closeup`, `13-sim-nova-closeup`, `14-sim-aria-available`:
Aria/Felix/Nova run the local wander sim (`source: 'sim'`) and the shared clamp holds their STATE
channels (visor / cape grid / foot aura) to idle-gray or available-cyan only — including frame 14,
captured while Henry was in a real error state two meters away.

Reading note: the constant per-agent toga-trim colors are IDENTITY, not state — Aria amber
`#FFB347`, Felix pink `#FF6B9D`, Nova lavender `#A78BFA` (`shared/palette.ts AGENT_TRIM`). State
never repaints identity trim.

## Draw calls per agent (perf-ledger.json, measured)

| agent | v1 meshes (CyborgCharacter) | v2 skinned draws | v2 full-frame DC delta (incl. post passes) |
|---|---|---|---|
| Henry | 54 static + crown/state extras | 6 | 8 |
| Librarian / Aria / Felix / Nova | 54 static + state extras | 5 each | 7 each |
| all five | 418 scene meshes (measured 1274→856) | 26 | 36 |

Scene wide-view totals: 1340 → 920 calls; geometry cached app-lifetime (`geometries` flat at 841
across the whole capture).

## Files

- `events-record.log` — full four-phase capture (H: chat, L: describe run, E: sever/restore)
- `events-phaseE.log` — re-aimed error close-up pass (frames 02,04–07)
- `events-sim.log` — wander + closeup pass for the sim trio
- `events-dc-census.log` — per-agent draw-call census raw output
- `perf-ledger.json` — before/after + per-agent numbers, measurement method
