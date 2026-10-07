# Wizard lane report — 2026-10-07 (branch `wq/wizard`)

Design: `reports/WIZARD_DESIGN_2026-10-07.md`. Evidence: `reports/wizard-evidence/` (84 PNGs + `evidence.json` + 2 keyboard-walk PNGs).

## Commits (on 70a9f658)
| Commit | What |
|---|---|
| 4b6959b2 | Design doc |
| 9c690936 | `check_provider` makes one authenticated, inference-free model listing and returns `{live, model, models_available, note}`; refused/out-of-credit key = 400. `PUT /api/agent/identity` trims and rejects an empty or >40-char name (400 + message). Rust tests added. |
| 7160135f | Boot: BootScreen owns the only connect loop, uses the shared `classifyStartupConfig` (`lib/startupConfig.ts`), stable callbacks from App; copy says "Permagent's background service". |
| b821a889 | Wizard rebuilt: Welcome → Model → Agent → Code folders → Extras → Review. Old Moment* steps, fake chat, duplicate persona steps and auto-advance removed. |
| 5c063a83 | Tests: `WizardShell.flow.test.tsx` (8), `wizardLogic.test.ts` (12), `tokens.fitness.test.ts` (5) |
| 6d5d12ca | Polish found in screenshots (help scrolls into view, single memory-model confirmation) |
| da00872d | Evidence + the harness script (`ui/command-center/wizard-evidence.mjs`, kept in this commit for regeneration) |
| a6f7c834 | Harness removed, this report |

## Checks (from `ui/command-center`)
- `npx tsc --noEmit` → exit 0
- `npx vitest run src/components/wizard src/components/splash src/App.startup.test.tsx` → all pass (wizard 31, splash+startup 25)
- `npx vite build` → exit 0
- Full `npx vitest run` → exit 1: 3573 passed, 4 failed, all outside this lane and unchanged by it: `rawButton` and `spaceScale` (world/forum + automate offenders), `terminalChrome` (Terminal hash), `worldLabelStacking`. No file under `components/terminal` or `components/world` differs from 70a9f658.
- Harness (production build, real `api.ts`, Chrome headless, fixture daemon): exit 0; 84/84 shots pass the clipping audit (heading + primary inside the viewport, no horizontal overflow) at 1024×640 and 1440×900, dark and light; keyboard-only walk of the whole flow at 1024×640 completes (Enter/Space/Tab/arrows/Escape), identity PUT and `wizard_complete` reached.

## Not verified
- **Rust is uncompiled and its tests unrun.** The shared target was cold; `cargo check -p permagent-daemon` took free disk from 26 to 25.05 GiB within a minute, so it was stopped (rule: stay above 25 GiB). Changed: `routes/config_management.rs`, `routes/identity.rs`, `openapi.rs`. Needs `cargo check -p permagent-daemon` and `cargo test -p permagent-daemon --lib -- live_check first_name put_identity provider_check` on a machine with room.
- No real provider key, real Ollama or native app run; all behaviour above is against fixtures.
- The live check depends on each provider's `fetch_supported_models`; providers that do not override it return an empty list, which the UI reports as "saved, couldn't confirm live" rather than "works".

## Found, left out of scope
- `components/common/Button.tsx`: its `live` ref is set false by StrictMode's dev double-effect and never reset, so a caller-driven `pending` never clears in `vite dev` (production unaffected). The wizard no longer passes `pending` to its primary button (it showed a focus-loss bug too), but other callers are exposed in dev.
- `ui/command-center/ui-polish-onboarding-evidence.mjs` asserts the old 8 inert steps and will now fail; it is referenced by `docs/orchestrator/UI_POLISH_ONBOARDING_SUBDAG.md`, so it was left for its owner.
- No in-app "no model connected" banner outside the wizard (chat surfaces belong to other lanes); the limitation is stated on the review step.
- Splash still plays on every launch (unchanged).
