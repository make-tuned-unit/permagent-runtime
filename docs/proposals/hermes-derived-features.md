# Features to take from Hermes Agent v0.19 "Quicksilver"

Reviewed 2026-08-14 against Hermes Agent v0.19.0 (Nous Research, MIT). Hermes is
a peer, not a model: a model-agnostic harness with persistent memory across
terminal/desktop/messaging. Their release notes are the source; every claimed
gap below was checked against this codebase by grep before being listed, and the
three that turned out to be partially present are marked as such.

Ordering is by **dependency and blast radius**, not by appeal. Each phase is
defined by an end-to-end acceptance test, because a half-wired feature is worse
than an absent one — it reads as present and fails silently, which is the exact
class this repo keeps paying for (`~/dev` guessing, the resident-agent extension
set, `test-daemon.sh` scope).

---

## Phase 1 — `SecretSource`: pluggable secret backends

**Why first.** It is the only item that retires a *recurring* failure class
rather than adding surface. Secrets today are keychain-or-nothing (`grep` for
`SecretSource|1password|bitwarden|op://` returns zero hits). Ad-hoc signing
gives the app a new `cdhash` on every build, which invalidates the keychain ACL
and partition list, which produced `Unable to obtain authorization` and then
`In dark wake, no UI possible` — three separate wrong diagnoses before the cause
was instrumented. A password-manager reference resolved at load time does not
touch the ACL at all.

**The keychain stays the default.** This adds sources; it does not replace one.

### Scope

* `crates/goose/src/config/secret_source.rs`
  * `enum SecretSource { Keychain, OnePassword { reference }, Bitwarden { item, field }, File }`
  * Resolution precedence: explicit per-key source → configured default → keychain.
* Config surface: `secret_sources: { OPENAI_API_KEY: "op://Personal/OpenAI/credential" }`.
* Availability probe: `op` / `bw` on PATH **and** signed in. Probe and read must
  both be time-bounded. The dark-wake incident was an unbounded blocking call
  that could not be timed out because the future was awaited inline — see
  `config_management.rs::PROBE_TIMEOUT` for the shape to copy.
* Single read path: everything resolves through `Config::get_secret`
  (`config/base.rs`). Extension `env_keys` injection must not grow a second path.
* Settings → API keys shows the **source** per key ("macOS Keychain",
  "1Password") and allows switching.
* Onboarding offers a detected, signed-in manager; never blocks on one.

### Not allowed

* No secret value in any log, error, or trace — including CLI stderr passthrough.
* No silent fallback. If a configured `op://` reference fails to resolve, the key
  is **unavailable and says so**; it must not quietly fall back to keychain and
  leave the user believing the reference works.

### Acceptance (end to end)

A user with 1Password sets `OPENAI_API_KEY` to an `op://` reference in Settings,
restarts the app, and chat works — with the value never written to the keychain
or to disk. Killing `op` mid-session produces an honest "couldn't read the key
from 1Password" rather than a generic provider error.

---

## Phase 2 — Byte-stable system prompts

**Partially present.** Prompt-cache machinery exists (`cost_router/cache.rs`,
provider `cache_control`). What is missing is the property that makes it pay:
Hermes pins session-context *rendering* so the prompt prefix is byte-identical
turn to turn. Any drift — a timestamp, a recall count, a map iteration order —
invalidates the cache for the whole turn.

**Why second.** Independent of Phase 1, small blast radius, and it is the
cheapest large latency/cost win available.

### Scope

* Audit `PromptManager` rendering for nondeterminism: wall-clock timestamps,
  relative dates ("2 hours ago"), `HashMap` iteration order, "N memories
  recalled" counts, anything derived from the current turn.
* Split into a **stable prefix** (identity, persona, tool list, skills, policy)
  and a **volatile suffix** (turn-specific context). Only the prefix carries
  `cache_control`.
* Emit the prefix hash per turn at debug level, plus provider-reported cache
  hit/miss, so a future regression is visible instead of silently expensive.

### Acceptance (end to end)

Two consecutive turns in one session render byte-identical stable prefixes
(asserted in a test, not by inspection), and the provider reports a cache hit on
turn 2. A deliberate perturbation of persona text produces a miss on the next
turn and a hit on the one after.

---

## Phase 3 — Deny rules that carry a reason

**Absent** (`deny_rule|user_deny` → zero hits). Decision Inbox, autonomy trust
modes and `tool_inspection` all exist, so this is composition rather than new
machinery.

### Scope

* `deny_rules: [{ pattern, scope, reason }]`, evaluated at tool dispatch
  **before** permission mode — a deny rule that permissive mode can override is
  not a deny rule.
* `/deny <reason>`: records the refusal against the tool request and injects the
  reason into the agent's next context so it course-corrects rather than
  retrying. This is the half Hermes gets right and a plain block does not.
* Decision Inbox distinguishes denied-by-rule from denied-by-user.
* A rule that matches nothing is **reported**, not silently inert.

### Acceptance (end to end)

A rule added in Settings blocks the tool while the agent is in `auto` mode, and
the next assistant message references the stated reason.

---

## Phase 4 — Delivery-obligation ledger

**Partially present.** `proactive.rs`, decisions and `turn_outcome_e2e` cover
adjacent ground; the durable obligation itself is missing. Hermes keeps a ledger
in `state.db` so a final response cannot be lost between generation and platform
confirmation.

**Why fourth.** It touches the reply path, which is the highest-risk surface in
the daemon. It should land after the lower-risk phases have settled.

### Scope

* Spectral table: `delivery_obligations(id, session_id, turn_id, surface,
  payload_ref, created_at, delivered_at, attempts)`.
* Written when a final assistant message is produced; cleared only on confirmed
  delivery.
* Daemon start resolves unfinished obligations: redeliver, or surface in the
  Inbox if the surface is gone.
* Ownership check so two processes cannot double-deliver.

### Acceptance (end to end)

Killing the daemon between generation and delivery results in the message
arriving after restart — **exactly once**, proven by a test that asserts the
duplicate case fails.

---

## Phase 5 — Live subagent transcripts

**Absent as a readable stream.** Worker plumbing exists (`goal_engine`), and the
roster is real (claude_code, codex, cursor, librarian, reviewer, steward, strix),
but a running worker cannot be watched. This is why a worker once surfaced to the
user as "one of your workers" with nothing behind it.

### Scope

* Append-only transcript per worker run: tool calls, results, streamed text.
* Workers panel gains a live "watch" view.
* **Bounded** growth — size cap or ring buffer. An unbounded transcript on a
  long-running worker is a disk bug wearing an observability costume.

### Acceptance (end to end)

Clicking a running worker shows its live output, and a worker producing 100 MB of
output does not produce a 100 MB file.

---

## Explicitly not taking

* **Smart approvals by default** (LLM reviewer silently assessing flagged
  commands). The Decision Inbox with explicit escalation is more honest, and it
  is already built. Adopting an auto-approver would undo that.

## Lower value, revisit later

Session export breadth + `--redact` (export exists; only formats are missing);
reasoning-effort tiers beyond the current `cost_router` roles; MCP OAuth;
`excluded_providers` filtering; profile-based gateway routing (matters when the
waitlist lands); LM Studio JIT loading; API content sidecars for audit.

## Sources

* <https://github.com/NousResearch/hermes-agent/releases/tag/v2026.7.20>
* <https://hermes-agent.nousresearch.com/>
