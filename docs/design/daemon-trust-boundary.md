# What the daemon's local trust boundary actually is

Status: findings + partial mitigation, landed. Written 2026-08-19, after an architecture review.

## A note on what this document does and does not contain

This is a **public repository**. The architectural fact below is not sensitive — it is inherent to the design, and anyone who reads `middleware/auth.rs` can derive it. What is deliberately **not** written here is the route-by-route consequence analysis that sits behind the audit classifier: which specific endpoints reach which capability, and which second gates do or do not apply to each. That analysis exists, it is what `security/auth_audit.rs` encodes in code, and it is kept out of prose because publishing a curated map of unmitigated surface — while the mitigation is explicitly still blocked — hands a reader more than it hands a maintainer.

If you are working on this and need that analysis, read the classifier. If you are writing about it, keep it in code.

## The boundary

`permagentd` binds `127.0.0.1:3001` behind a bearer token at `~/.permagent/secrets/daemon_token.json`. The file is `0600` inside a `0700` directory, written atomically that way from the first byte (`crates/goose-server/src/state.rs`, via `crates/goose/src/config/secure_fs.rs`). That is correct and it is worth keeping. It is also narrower than it looks: it separates **other users** from the token. It does not separate **other processes running as this user**, because Unix file permissions have no sub-user granularity.

So the daemon's trust boundary is not "the Permagent app". It is **anything on this Mac running as this user**. Everything behind that line is in scope: the Brain, the user's files and projects, credential-backed tools, and provider API keys.

Stated at the level that matters: a process running as this user can read the token, present it, and is then indistinguishable from the app at the credential layer. It therefore reaches the control plane's full authenticated surface — code execution through the tool rail, secret read and write, provider spend, and user-data mutation. Two routes hold a genuine second gate: the browser and desktop bridges are unauthenticated but loopback-only (`middleware/loopback.rs`), and stream-scoped `/sse-token` credentials are admitted on the SSE and `/events` rails and refused everywhere else (tested directly in `routes/sse_token.rs`).

`/collect/{site_key}` is mounted outside both the bearer guard and the origin guard, and that is acceptable. It is the first-party analytics beacon: the user's own website posts to it cross-origin from visitors' browsers, which can never hold a daemon token. Its exposure is bounded by construction — a 128-bit random site key in the path, a 2 KiB body cap, a fixed field whitelist, a per-key rate limit, and INSERT-only access to one table. It is also irrelevant to this threat model: a same-user process already holds the master token, so an insert-only unauthenticated endpoint gives it nothing it did not already have. The reasoning it rests on is "this endpoint cannot do much", not "this endpoint is hard to reach", and that is the right shape for a route that must be publicly reachable.

## What is already right — do not rebuild it

The token file is `0600` in a `0700` directory, atomically. The auth layer is **fail-closed**: a daemon with no token returns 503 rather than allowing anonymous access, and that holds even when device tokens exist (`middleware/auth.rs`). Comparison is constant-time via `subtle`, and the master check and the device scan both run before the verdict so timing does not reveal which class matched. An origin guard fronts every route so a remote web page in the user's browser cannot reach the daemon cross-origin, token or not. `permagent doctor` already asserts the file mode is `0600`. None of this is the weak part.

## What was landed here, and exactly what it buys

**An auth audit, and nothing that pretends to be prevention.** `daemon_auth_audit` (schema v43) records every refused request, and every admitted request on a route whose consequence class is execute, secrets, spend or mutate. Each row carries the admitting principal (`master` or a device id), the credential class, the route's consequence class, the method, the path, and the status the caller actually received. Reads and status polls are not recorded on success — the desktop app polls status every second, and an append-only table with no retention story must not grow by a row per poll.

This makes same-user misuse **detectable**. It does not make it **preventable**, and the distinction is the whole point of this document. Specifically:

- A wrong token and a missing token are recorded as different events (`unrecognised` vs `none`), so a process probing the daemon is distinguishable from a client that forgot a header. That is the highest-value signal in the table.
- The rows are append-only at the database, by `BEFORE UPDATE`/`BEFORE DELETE` triggers, matching `egress_audit` and `decision_audit`. **That stops rewriting through SQL. It does not stop `rm`.** An attacker who can run code as this user can delete the database file. Treat the presence of a row as evidence and the absence of a row as no evidence either way.
- The audit is best-effort: a write failure is logged loudly at `error` and swallowed. This is the one place it deliberately differs from `sovereignty::record_egress`, which may refuse a cloud call it could not log. There is no promise here worth locking a user out of their own daemon to keep.
- The path is recorded, never the query string. Long-lived tokens ride `?token=` on the SSE and WebSocket rails, and an audit that leaked the credential it was auditing would be worse than no audit.

Read it at `GET /api/security/auth-log?limit=`, beside the egress log.

**A peer-verification seam, inert.** `crates/goose-server/src/middleware/peer_identity.rs` holds the policy, the verdict type, the verifier trait and the middleware, mounted on the protected router outside the bearer layer. It is **disabled on every build**, and disabled is a true no-op — the verifier is not consulted at all. Turning it on today refuses every request, deliberately and loudly, because of the two blockers below.

## What is NOT solved, and will not be by anything above

**No route was narrowed by one.** The token is still readable by any process with this uid; every route reachable before is reachable now; and the attacker can additionally read the audit that records them and delete the database that holds it. The only thing that changed is that, in the ordinary case where the attacker does not bother to clean up, there is now a record.

Three mitigations were evaluated and **rejected as not load-bearing against this threat**, and it is worth writing down why so they are not proposed again:

- **Scoped / capability-limited tokens do not help here.** They are real and worth building, but against a *different* threat: a lost phone or a compromised companion device. Right now a paired device token grants exactly what the master token grants, which is a genuine gap. It is not *this* gap. A same-user attacker reads the master token, so any capability limit placed on it constrains the desktop app by exactly as much as it constrains the attacker. Scoping was deferred rather than half-done because the iOS companion, the CLI and the in-process MCP tools all hold real tokens today, and narrowing them without a client-by-client audit would break the product to buy nothing.
- **Short-lived tokens and rotation do not help here.** Rotation defends against a token that leaked *once* — into a log, a screenshot, a URL fragment. It does nothing against a reader who can simply read the file again after every rotation. Rotating hourly against a local reader costs re-pairing and buys an hour of nothing.
- **Anything based on the token alone cannot work.** This is the general form of the two above. The token is a shared secret in a file the attacker can read. No policy expressed in terms of that secret can distinguish the app from the attacker, because at the credential layer they are the same principal. This is the same conclusion `berdctl` reached for its broker: any same-user process can bypass the CLI and hit the broker directly, so security has to live somewhere other than the caller's identity-by-secret. (That document is not in this repository; the lesson is recorded here as it was given, not as verified in-tree.)

There is one further gap this change does not close: the `/events` and `/voice` WebSocket upgrades authenticate inside their own handlers via `validate_stream_token` and therefore do **not** pass through the auditing middleware. Their use is not recorded. Closing that means auditing at the upgrade sites, and it should be done.

## What would actually narrow the boundary

Peer code-signature verification: the daemon asks the OS who is on the other end of the connection and refuses callers that are not the signed Permagent app. This is the only mechanism evaluated that genuinely changes the answer, because it does not depend on a secret the attacker can copy. It has **two** dependencies, not one.

### 1. A stable code-signing identity

`ui/desktop/src-tauri/tauri.conf.json` set `"signingIdentity": null`. The app was ad-hoc signed, so **every build produced a different code-signing identity** and there was nothing stable for a `SecRequirementCreateWithString` requirement string to pin. This is the same blocker `docs/design/update-integrity.md` names for keychain ACLs, and it clears the same way: a Developer ID certificate, now configured.

Once enforcement is written, the requirement string pins the Developer ID team identifier, and the allowlist must cover every binary that legitimately talks to the daemon — the Tauri app, the `permagent` CLI, and the daemon's own in-process MCP tools, which read the token file and call back over loopback.

### 2. A transport that carries peer identity (the one that is easy to miss)

macOS exposes peer credentials **only on UNIX-domain sockets**. `LOCAL_PEERPID`, `LOCAL_PEERCRED` and `LOCAL_PEERTOKEN` are defined in `<sys/un.h>` at level `SOL_LOCAL`; `<sys/socket.h>` has no `SO_PEERPID` or `SO_PEERCRED` equivalent for TCP. The daemon binds **TCP** on loopback (`crates/goose-server/src/commands/agent.rs`, `TcpListener::bind`), so `getsockopt` cannot name the caller at all. `ConnectInfo<SocketAddr>` gives an address and an ephemeral port, which identify nothing.

The workaround — walking every pid with `proc_pidfdinfo` to find who owns the matching 4-tuple — is **racy by construction**: the pid can exit and be recycled between the lookup and the check, and pid-keyed code-signature checks are exactly the pattern Apple warns against. A security control with a TOCTOU race in it is not a security control.

So enforcement requires moving the control plane onto a UNIX-domain socket (or adding one alongside the TCP listener for local clients), then reading the peer's `audit_token_t` via `LOCAL_PEERTOKEN`, building a `SecCodeRef` with `SecCodeCopyGuestWithAttributes` keyed on `kSecGuestAttributeAudit` — **not** `kSecGuestAttributePid` — and checking it with `SecCodeCheckValidity`. The seam is built and the trait is documented for exactly this implementation; no Security.framework FFI was shipped, because untestable unreachable FFI on a security path reads as a working control to the next person who greps for `SecCodeCheckValidity`.

Note what peer verification would and would not buy even then. It would stop an *arbitrary* same-user process — a malicious npm postinstall, a curious script. It would **not** stop code injected into the signed app itself, and it would not stop a user-launched copy of the real CLI being driven by an attacker. It raises the bar from "read a file" to "compromise or impersonate a signed binary". That is a large improvement and it is not a wall.

## Acceptance test for "the boundary is what we say it is"

1. With the daemon running normally, call a secrets-class route with no token, then with a wrong token, then with the real one.
2. `GET /api/security/auth-log` must show three rows for that path: `denied/none`, `denied/unrecognised`, `admitted/master` — with the class recorded as `secrets`.
3. Poll `/api/version` with the real token ten times. The auth log must show **no** new rows.
4. Re-derive the boundary statement above from `middleware/auth.rs` and `security/auth_audit.rs`. It must still be true.

If step 4 ever stops being true, this document is wrong and the change that made it wrong should say so here. Until then: the daemon is protected from other users, it is not protected from other processes, and the only thing that changed is that misuse now leaves a trace.
