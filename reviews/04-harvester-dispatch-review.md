# Review of DISPATCH-permagent-harvester.md

Checked every environmental premise in the dispatch against this machine before sending it to a worker. The design is sound and unusually well-specified — the constraints section, the phase gating, and "do not have a model invent the format library" are all correct calls. But **six factual premises are wrong**, and one of them breaks a stated security property.

## Blocking — must be resolved before the phase that depends on it

### B1. The Command Center is not Next.js. Phase 5's security design cannot work as written.

The dispatch says:

> New `/content` route in the Next.js Command Center … Next.js route handlers call `127.0.0.1:4007` server-side with the bearer token from env. **The token never reaches the browser.**

`ui/command-center` is **Vite 5 + React 18** (`package.json:53`, `"dev": "vite"`). There is no `next` dependency and no `next.config.*` anywhere. A Vite SPA has no server side, so there are no route handlers to hold a token. The existing app already loads the daemon token *into the browser* (`ui/command-center/src/lib/api.ts:397-407`: `loadDaemonToken()` then `authHeaders()`).

So Phase 5 as written is not implementable, and the property it promises ("token never reaches the browser") is not achievable by that route. Three options, needing a decision:

1. **Proxy through permagentd** — add a `/harvester/*` passthrough in `crates/goose-server` that injects the bearer token server-side. Preserves the stated property, one new route module, matches how everything else in the app already reaches the daemon.
2. **Follow the existing pattern** — the browser holds the harvester token exactly as it already holds the daemon token. Consistent with today's design, but drops the property the dispatch asked for. On a single-operator localhost box this is defensible; it should be a conscious choice, not a silent one.
3. **Vite dev-proxy only** — works in dev (`vite.config` already proxies `/api`, `/sessions`, … to `:3001`), does nothing in a built app. Not a real answer.

**Recommendation: option 1.** It is the only one that keeps the promise, and this repo has just been audited for exactly this failure mode — descriptors that claim a security property the code does not deliver.

This is the same false premise the audit already caught as claim **C6** (`README.md:5` also says "Next.js"). The dispatch inherited it from the README. Worth fixing the README in the same pass so the next document does not inherit it again.

### B2. Neither required model is installed.

Dispatch requires `gemma3n:e4b` (local extraction, every stage) and `qwen3-embedding:0.6b` (exemplar retrieval fallback). Ollama on 11434 currently has **only** `qwen25-16k:latest` and `qwen2.5:7b`.

Not blocking for Phase 0. Blocking for Phase 1 onward. Either pull the models or change the config defaults to what exists — but note the dispatch's own rule is explicit configuration over discovery, so the committed `harvester.example.toml` must name a model that is actually present or the first run fails confusingly.

### B3. Every cloud model route points at a service that is not running.

Dispatch states as fact: "Taken already: 4000 (LiteLLM), 4002 (spend proxy), 4004 (task runner), 4005 (TimesFM), 4006 (Knowledge API), 9843 (Command Center API)."

Actually listening right now: **only 11434 (ollama)**. Every other one of those ports is free. All cloud calls (correlation, drafting, critic) are specified to go through LiteLLM on 4000, and Phase 6 pulls spend from 4002.

Phases 3, 4 and 6 are buildable but not runnable until those come up. Their acceptance criteria ("token cost per pass logged in `runs`") cannot be met on this machine today.

### B4. `4007` is free. ✅

The one port claim that checks out. Nothing is listening on 4007.

## Non-blocking corrections

### B5. Every source path in the example config is wrong.

| Dispatch says | Reality |
|---|---|
| `~/projects/permagent` | **missing** — repo is at `~/Documents/dev/permagent-runtime` |
| `~/projects/spectral` | **missing** (Spectral is a git dependency, not a local checkout) |
| `~/projects/permagent-content` | **missing** — must be created |
| `~/notes` | **missing** |
| `~/.claude/projects` | exists ✅ |

The committed example config must use real paths or the first `once harvest` reads nothing and looks broken.

### B6. Workspace wiring is fine. ✅

Root `Cargo.toml` has `members = ["crates/*"]`, so a new crate at `crates/permagent-harvester` joins automatically with no root edit.

**Design note worth enforcing:** keep `permagent-harvester` **independent of the `permagent` crate**. That crate pulls llama.cpp, candle, and deno; depending on it turns a 2-minute build into a 13-minute one and drags the harvester into the disk-and-serialisation problems the rest of this repo has. A standalone crate with axum + rusqlite/sqlx + clap + serde + toml keeps this dispatch fast to iterate on. If it ever needs Spectral, put it behind the trait Phase 4 already calls for.

## Scope note

This is a six-phase greenfield build landing on top of an audit that produced 20 open remediation items, 9 of 23 promise claims failing, and five PRs currently in flight. Both can proceed — different crates, no file overlap — but the harvester should not jump the P0 queue. Phase 0 is dispatched now because it is self-contained and blocks nothing.

## What was dispatched

**Phase 0 only**, to Codex, with B1–B6 corrected in the brief. The dispatch's own rule is "do not proceed to the next phase until the previous one meets its acceptance criteria", so gating at Phase 0 is faithful to it, not a reduction of scope.
