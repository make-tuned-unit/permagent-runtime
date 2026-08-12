# Handoff — 2026-08-11 → rebuild on the other mini

For the Claude Code session doing the rebuild on the second Mac mini. Authored
on the primary Mac at the end of the 08-11 session. **Per-machine state
(~/.permagent, config.yaml, keychain, Claude memory files) does NOT travel
with this repo** — everything you need is in this note.

## What is on main (merge order today)

1. **#977** `03cdb3829` — Kinrows analytics failure fix set:
   - `openai.rs stream()` logs every endpoint-routing decision
     (`"openai endpoint routing"`, fields: model / base_path / endpoint) and
     WARNS when a responses-preferred model (gpt-5.6*) falls to
     chat/completions.
   - `extract_reasoning_effort` parses `-none` / `-minimal` suffixes;
     `create_request` downgrades reasoning_effort to `none` for gpt-5.6* when
     function tools are present (that combination 400s on chat/completions).
   - Session provider/model sync posts a persisted "Model switched: …" notice
     into the chat instead of swapping silently.
   - `ProviderError::RequestFailed` (4xx) no longer invites "please retry".
   - `session_name.md` names sessions as efforts ("Setting up Analytics for
     Kinrows") so Settings → Spend rows read as named work + cost.
2. **#978** `ee6f1e8c2` — Steward git-health lane (harvested from the stalled
   worktree agent): sweep is **default OFF** (`steward_scan_enabled`),
   detect/propose only, Tier-2 Jesse-only Decision-Inbox approvals, full
   second-look re-verification at effect time, v40→v41 risk_policy reconcile.
   Includes the macOS path-canonicalization fix (worktree list prints
   /private/var, approvals may carry /var).
3. **#979** — BUILT_NOT_SERVED wave 1 (six items): attention-goal Inbox
   bucket; incident resolve (route + Settings→Activity triage strip + status
   CHECK rebuild adding 'resolved'); Home→Dashboard teaching fix + guard test;
   notifications.ts consumes `notification_routed`; briefing ack route +
   HUD button; sidebar daemon-connection dot.

## The rebuild (documented chain — do not deviate)

```
cd ui/command-center && npm ci      # node_modules often absent
cd ../desktop && npm ci
npm run build:ui
npm run build:daemon
npm run build:sidecar               # NEVER skip — stages the fresh daemon
npm run build:mcp-runtime           # NEVER skip
npx tauri build --bundles app       # skips the ~2G DMG; "no private key" = benign updater warning
```
`build:all` dies at build:icons (PIL) — run steps individually.
Install: quit Permagent → replace /Applications/Permagent.app →
`launchctl kickstart -k gui/501/ai.permagent.daemon`.
If the app comes up blank: WebKit cache clear
(`rm -rf ~/Library/WebKit/ai.permagent.* ~/Library/Caches/ai.permagent.*`)
and verify the embedded JS name matches dist/index.html.

## Post-install verification (the actual point of the rebuild)

1. **gpt-5.6 endpoint routing — the unresolved mystery.** This morning's
   live daemon sent `openai/gpt-5.6-terra` to `/v1/chat/completions` with
   `reasoning_effort: medium` + tools (400), even though
   `should_use_responses_api` at HEAD routes gpt-5.6* to `/v1/responses`
   (unit-tested, and no OPENAI_HOST/OPENAI_BASE_PATH override existed in
   config.yaml, daemon env, or keychain on the primary Mac). After install:
   switch a session to openai/gpt-5.6-terra, send one message that has tools
   loaded, then check `~/.permagent/logs/daemon.err` for the
   `openai endpoint routing` line. Expected: `endpoint=responses`. If you see
   `endpoint=chat_completions` plus the warn line, the mystery is real and
   machine-independent — investigate `base_path` at runtime; the log line now
   gives you the evidence that was missing.
2. Model switch: change the default model, send a message in an existing
   session → a visible "Model switched: …" assistant notice must appear.
3. Wave-1 surfaces: sidebar shows a green daemon dot; Settings → Activity
   shows the incident strip only when incidents exist; the Decision Inbox
   shows a "Parked goals — waiting on you" bucket when a goal has
   needs_human_attention set.
4. Steward (only when Jesse wants it): set `steward_scan_enabled: true` in
   config.yaml. First honest proposals should be the LANDED leftover
   worktrees on the primary Mac (not this mini):
   `.claude/worktrees/wf_4694f994-535-{1,2}`, `.claude/worktrees/agent-a42ada52193213c0c`
   (the Steward's own birth worktree — its content is #978),
   `.codex-lanes/{needsmerge,observability}`, `permagent-worktrees/initiative-wiring`,
   `.permagent-goal-worktrees/build-full-loop`.
   **Never touch**: `.permagent-goal-worktrees/cli-a8c393d4-…` (Aug-4 iOS/voice
   work, landed-ness unverified) and `permagent-worktrees/spectral-pin-028a286`
   (queued: retarget to spectral dc7d6b0, don't merge).

## Known local-gate traps (this repo, any machine)

- Daemon lib/integration tests SIGKILL locally (dylib-signature) — compile
  them (`cargo test -p permagent-daemon --lib --no-run`) and let CI run them.
- Snapshot regen: pin `HOME` AND `PERMAGENT_PATH_ROOT` (sequenced exports —
  a single `export HOME=$(mktemp -d) X="$HOME/…"` expands the OLD $HOME) AND
  keep `CARGO_HOME`/`RUSTUP_HOME` pointed at the real ones, AND reuse one
  stable fake-home dir (fresh mktemp HOMEs invalidate rusty_v8's cache →
  10-minute rebuilds). Live-config leak symptom: "The Guard … (now: on)"
  appearing in prompt snapshots.
- Heavy `--all-targets` gates are CI-only. Local gate = cargo check, fmt,
  targeted tests, single-crate clippy, tsc + vite build. Re-run the gate
  after the LAST edit.

## Loose ends that are NOT yours unless asked

- Goal "Add a 'Changelog' section to README.md" (e2e-harness-probe) sits in
  triage — the engine's own land→promote→dispatch chain is still unvalidated
  live; let the engine take it.
- Bench re-runs (the 08-10 verdicts are VOID — isolation bug) become
  meaningful after this rebuild.
- The Kronos → Financier integration plan (uncertainty/scenario sidecar,
  3-step spike, kill criteria) is recorded in the primary Mac's session
  memory; ask Jesse before starting it.
