# Mesh M0 Spike — LOG (epic #306)

Branch: `research/mesh-m0-results` · Worktree: `~/dev/permagent-worktrees/mesh-m0-spike`
Started: 2026-06-11 (evening, ADT)

Dispatch: three-device pool physics spike. Docs only — no permagent code changes.
Lab dirs (outside repo): `~/dev/mesh-spike` on each machine.

---

## Phase 0 — Inventory

### Worktree (Step 1)

```
cd ~/dev/permagent-runtime && git fetch origin
git worktree add ~/dev/permagent-worktrees/mesh-m0-spike -b research/mesh-m0-results origin/main
# HEAD = 8c8040d3c
```

### M1 Mac mini (local, this machine)

```
$ system_profiler SPHardwareDataType | grep -E "Model Name|Chip|Memory|Model Identifier"
      Model Name: Mac mini
      Model Identifier: Macmini9,1
      Chip: Apple M1
      Memory: 16 GB
$ sw_vers
ProductVersion: 26.3   Build: 25D125
$ df -h /System/Volumes/Data | tail -1
/dev/disk3s1   460Gi   359Gi    73Gi    84%   /System/Volumes/Data
$ sysctl hw.memsize
hw.memsize: 17179869184
```

Tenants (M1):

```
$ ps aux | grep permagentd
PID 1451   permagentd agent                    RSS 195728 KB  (~191 MB)  ← production daemon
PID 16306  permagentd agent --port 3011        RSS  24288 KB  (~24 MB)   ← world-baseline test daemon (/tmp/world-baseline/w3)
```

### M4 Mac mini (remote) — every SSH command verbatim, all read-only

SSH route: `Host henry` → `Jesses-Mac-mini-2.local`, user `henry` (from ~/.ssh/config).

```
$ ssh -o BatchMode=yes -o ConnectTimeout=8 henry 'system_profiler SPHardwareDataType | grep -E "Model Name|Chip|Memory|Model Identifier"; sw_vers; df -h /System/Volumes/Data | tail -1'
      Model Name: Mac mini
      Model Identifier: Mac16,10
      Chip: Apple M4
      Memory: 16 GB                 ← dispatch said 16–24 GB; it is 16 GB
ProductVersion: 26.2   Build: 25C56
/dev/disk3s5   460Gi   239Gi   179Gi   58%   /System/Volumes/Data
```

```
$ ssh -o BatchMode=yes -o ConnectTimeout=8 henry 'ps axo rss,pid,user,comm -m | head -12; echo "---tailscale---"; ls -d /Applications/Tailscale.app 2>/dev/null; which tailscale 2>/dev/null; echo "---network---"; for dev in en0 en1; do echo "$dev: $(networksetup -listallhardwareports | grep -B1 "Device: $dev" | head -1)"; ipconfig getifaddr $dev 2>/dev/null; done'
   RSS   PID USER   COMM
9713984 28923 henry  /Applications/Ollama.app/.../ollama        ← ~9.3 GB resident (loaded model)
308688  1334 henry  /Applications/Ollama.app/.../ollama         ← ~301 MB (server)
140160 53100 henry  Google Chrome Helper (Renderer)             ← ~137 MB
113152   664 henry  Google Chrome                               ← ~110 MB
 97856   584 henry  python3.11 (homebrew)                       ← ~96 MB
 38144 97837 henry  ~/.nvm/versions/node/v22.22.0/bin/node      ← ~37 MB
(remainder system processes < 50 MB each)
---tailscale--- (not installed)
---network---
en0 Ethernet:  169.254.233.220 (self-assigned — no usable link)
en1 Wi-Fi:     192.168.2.206   (active)
```

**ZeroClaw footprint (CANDIDATE, unconfirmed):** the henry-user stack —
Ollama (9.3 GB model + 0.3 GB server) + python3.11 + node + Chrome ≈ **~10.0 GB
resident**. ⚠️ AWAITING [JESSE]: confirm process names + DO-NOT-TOUCH paths.
No further M4 work until confirmed (stop condition). No ZeroClaw file, config,
or launchd reads were performed — process table only.

```
$ ssh -o BatchMode=yes -o ConnectTimeout=8 henry 'ping -c 20 -q 192.168.2.205 ...; sysctl hw.memsize; vm_stat | head -5'
hw.memsize: 17179869184
(ping results below)
```

### MacBook Pro

Unreachable / not inventoried. **AWAITING [JESSE]:** chip, RAM, free disk,
macOS version (About This Mac paste is fine).

### Network reality

Both minis are on **Wi-Fi** (192.168.2.0/24). Both have ethernet ports with
only self-assigned 169.254 addresses — no live ethernet link on either.
Tailscale: **not installed on M1 or M4** (no app, no binary, no process).

Ping matrix (20 packets):

| path | min/avg/max/stddev (ms) | loss |
|---|---|---|
| M1 → M4 (LAN Wi-Fi) | 0.406 / **0.553** / 0.667 / 0.054 | 0% |
| M4 → M1 (LAN Wi-Fi) | 6.076 / **39.614** / 106.090 / 34.884 | 0% |
| M1 ↔ M4 Tailscale | N/A — Tailscale not installed on either mini | — |
| MacBook → M4 on-LAN | pending MacBook window [JESSE] | — |

The asymmetry (0.55 ms vs 39.6 ms avg with 106 ms spikes) is Wi-Fi
power-save on the M1 radio — exactly the jitter profile that degrades
sharded inference. **Recommendation: wire both minis for the spike**
(both have ethernet ports; Wi-Fi delta can be captured as the optional cell).

### Engine versions (Phase 0 item 5)

```
$ gh api repos/ggml-org/llama.cpp/releases/latest
b9601  published 2026-06-11
$ gh api repos/exo-explore/exo/commits   (HEAD of default branch)
09f9ea313f72e261f40a94cea4c0e3681b31af23  2026-06-03  "libp2p -> zenoh (#2132)"
```

exo churn warning confirmed: its networking layer was swapped (libp2p → zenoh)
eight days ago.

### Ceiling math — see Phase 0 report (pending Jesse approval)

## Phase 0 decisions (Jesse, 2026-06-11)

1. **P2 = YES, agent-executed, bounded to Ollama model unload only.** Procedure
   (verbatim, every window):
   `ssh henry "ollama ps"` → `ssh henry "ollama stop <name>"` → `ssh henry "ollama ps"`
   (verify empty, log RAM). End of window re-warm:
   `ssh henry "ollama run <name> --keepalive 24h </dev/null"` + `ollama ps` verify.
   Ollama server, python/node/Chrome NOT touched. If `ollama stop` unavailable
   at installed version → STOP, no kill improvisation. Pause runs only inside
   approved windows. M4 contributes ~12 GB weights when paused.
2. **ZeroClaw = the henry-user Ollama stack + python3.11/node/Chrome (confirmed).**
   **DO-NOT-TOUCH (pinned):** everything on the M4 outside `~/dev/mesh-spike`,
   explicitly including `~/.ollama` (beyond the item-1 stop/re-warm commands),
   all henry app/config directories, and `~/Library/LaunchAgents`. P1 stands.
3. **MacBook specs: [JESSE PASTE PENDING].** Stretch tier auto-resolves:
   ≥36 GB → Llama-3.3-70B Q3_K_M; ≥48 GB → Q4_K_M; 16–18 GB → Stretch re-scopes
   to Qwen3-32B Q4_K_M and "70B out of reach" is recorded as a finding.
4. **Wiring = YES (Jesse's hands, before Phase 2).** On confirmation: re-run ping
   matrix wired; Wi-Fi rows kept as before-row. Wi-Fi delta cell approved optional.
5. **Tailscale = AGENT-INSTALLED with two human checkpoints** (amended). Scoped
   P1 exemption: Tailscale + Homebrew-if-absent may be installed globally on the
   M4 — first brick of its permanent headless-server role; exemption covers
   Tailscale/brew ONLY. Checkpoint 1: agent stages `sudo brew services start
   tailscale`, Jesse runs it (no agent password handling). Checkpoint 2: Jesse
   pastes a pre-auth key; key is used in the `tailscale up` invocation only,
   NEVER written to LOG/files/repo — logged REDACTED. Verify: `tailscale status`
   both minis + 20-packet pings both directions on 100.x; B/E unblock on green.
   M4 persistence: record what was done; proper persistence = M1 setup item.
   MacBook's own Tailscale: Jesse installs from App Store — no remote install.
6. **Evening window: [JESSE CONFIRMS].** Heavy cells (C/D/E/F) only inside it.
   **Ladder APPROVED:** MoE = Qwen3-30B-A3B IQ4_XS primary (Q4_K_M boosted cell);
   Dense = Qwen3-32B Q3_K_M (Q4_K_M boosted cell); Stretch per item 3.
   Downloads to M4 only; LAN transfer outward; df before each.
7. **exo:** zenoh HEAD 09f9ea313 primary; if hard-fail, ONE bounded retry at last
   pre-zenoh libp2p tag, then DNF. 2h/cell cap covers both attempts combined.

**Gating:** Phase 1 begins now for hands-free items (builds, exo installs, M4
downloads, M1/M4 control rows — M4 control row needs a pause window per item 1).
Items 3/4/5 unblock dependent cells as Jesse confirms.

---

## Phase 1 — Setup

### Toolchain inventory (2026-06-11)

```
M1: brew /opt/homebrew/bin/brew ✓ · cmake 4.3.2 ✓ · Xcode /Applications/Xcode.app ✓
M4 (ssh henry): brew /opt/homebrew/bin/brew ✓ (not on non-interactive PATH) ·
    cmake ✗ · CommandLineTools only · python3 = /usr/bin/python3
$ ssh henry 'which brew; ls /opt/homebrew/bin/brew; which git cmake python3 curl; xcode-select -p; mkdir -p ~/dev/mesh-spike; df -h /System/Volumes/Data | tail -1'
(df M4: 179 Gi free)
```

**Build strategy:** M4 has no cmake and the P1 exemption covers Tailscale/brew
only → llama.cpp is built ONCE on the M1 (static, `-DBUILD_SHARED_LIBS=OFF`,
Metal + RPC) and the binaries are copied to the M4's lab dir. No M4 installs.

### Re-scope after MacBook specs (Jesse, 2026-06-11)

MacBook Pro 13" 2018: Intel i5 quad 2.3 GHz, **8 GB** LPDDR3, Iris Plus 655,
Sonoma 14.6.1, 500 GB disk. → **CLIENT ONLY** (Intel: no MLX/exo; 8 GB: ~2 GB
net weight budget; slowest-node gating makes contribution an anti-goal).
- Cell C + Stretch tier **CANCELLED** → **C′**: core pod runs MoE Q4_K_M and
  Dense Q4_K_M (the quants that previously needed boosting) to find the core
  pod's true quality ceiling; OOM/failures recorded honestly.
- Cell D re-scoped: core-pod resilience — kill M1's rpc-server/exo worker by
  exact PID mid-generation; record M4-host failure mode, recovery, reload time.
  One trial per engine.
- Cell F **CANCELLED** (no viable contributor hardware); M2 remote-friend
  question stays open in the results doc.
- Cell B **PROMOTED** to headline: 2 client modes (OpenAI-compatible API;
  BROWSER → serving daemon /ui) × 2 networks (LAN; Tailscale off-LAN).
- NEW required results section: **"Minimum contributor spec"** (M2 pod-entry
  floor; below-spec devices auto-join as clients).
- Results doc states plainly: biggest-model-possible for this trio = 30B class
  at the best quant the core pod clears; 70B needs a future Apple Silicon node
  (RAM math shown).

### Build log (M1, 2026-06-11 ~22:18–22:27)

```
git clone --depth 1 --branch b9601 https://github.com/ggml-org/llama.cpp.git
cmake -B build -DGGML_METAL=ON -DGGML_RPC=ON -DBUILD_SHARED_LIBS=OFF -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build build -j 6 --target llama-cli llama-server llama-bench rpc-server
→ BUILD_OK: llama-bench 9.5 MB · llama-cli 11.4 MB · llama-server 20.7 MB · rpc-server 2.4 MB
Copied to M4: scp llamacpp-b9601-arm64.tgz henry:~/dev/mesh-spike/ → bin/
```

### exo setup

- **M1: environment built OK.** rustup nightly added; macmon v0.7.0 pinned
  (a1cd06b) via cargo install; dashboard `npm install && npm run build` ✓;
  `uv sync` ✓ (Python 3.13 env per uv.lock).
- **exo darwin dependency reality:** pyproject pins MLX to a *git fork built
  from source* (`rltakashige/mlx-jaccl-fix-small-recv`, branch
  `address-rdma-gpu-locks`) → **full Xcode Metal toolchain required on every
  macOS node**. Also requires rust *nightly*, node ≥18, uv, pinned macmon.
- **M4 has CommandLineTools only → exo on M4 = DNF candidate** under P1 (full
  Xcode is a global install outside the Tailscale/brew exemption). Recorded as
  operational-friction data per dispatch. ⚠️ [JESSE]: pooled exo cells (A/C′/D
  exo rows) are DNF unless the exemption is extended to Xcode on the M4.
- exo platform tiers: Tier 1 = M3 Ultra / M4 Pro / M4 Max / M5. Base M1 and
  base M4 are untiered ("no theoretical reason it shouldn't work").

### Model downloads (M4, screen/nohup saga)

1st attempt (`nohup … &` via SSH): process spawned but froze at 0 CPU, zero
output — killed by exact PID (28392, 28581; ps lines in transcript).
2nd attempt (detached `screen -dmS`): same silent freeze, session cleaned.
**Finding (M1 design input): non-interactive SSH background processes on the
M4 get QoS-frozen — a real obstacle for the headless-server role; daemonized
work on the M4 needs a proper launchd service (M1 build item), not ad-hoc
nohup/screen.**
3rd attempt: held-open SSH channel from the M1 — transfers at ~40 MB/s. Also
fixed a URL bug in the 1st script (`${f%%/Q*}` mangled repo paths → 29-byte
error bodies; rewritten with dirname/basename, HEAD-verified 200 + 16.38 GB).
Queue (all to `~/dev/mesh-spike/models/`, df-guard ≥50 G before each):
IQ4_XS 30B-A3B 16.38 G · 32B Q3_K_M 15.97 G · 14B Q4_K_M 9.0 G (M4 control) ·
8B Q4_K_M 5.03 G (M1 control) · 30B-A3B Q4_K_M 18.56 G (C′) · 32B Q4_K_M
19.76 G (C′) ≈ **84.7 GB total**.

### Tailscale (per amended item 5)

- M1: `brew install tailscale` ✓ (formula, headless tailscaled).
- M4: `/opt/homebrew/bin/brew install tailscale` in progress.
- CHECKPOINT 1 staged for Jesse (sudo, his hands):
  - M1 (local): `sudo brew services start tailscale`
  - M4 (his SSH/Screen Sharing): `sudo /opt/homebrew/bin/brew services start tailscale`
- CHECKPOINT 2 (auth) waits on checkpoint 1; key will be REDACTED in logs.

### Phase 1 decisions (Jesse, 2026-06-11 late evening)

1. **Checkpoint 1:** Jesse running `sudo brew services start tailscale` on M1
   now; M4 command runs (his SSH) once agent confirms M4 brew install done.
   Pre-auth key to be pasted at checkpoint 2.
2. **exo-on-M4 exemption DECLINED — DNF stands for all pooled exo cells.**
   Results doc records prominently: exo pooled mode requires full Xcode + rust
   nightly + node + uv on EVERY node → disqualified as a consumer pod engine
   at current state regardless of performance; M2 minimum-contributor-spec
   cites this. Q5 collapses to llama.cpp-RPC-only for pooled cells.
   **Consolation row:** exo single-node on M1 vs same model as M1 llama.cpp
   control row (MLX-vs-ggml per-node efficiency); "revisit at exo 1.0 /
   prebuilt binaries" note.
3. **Evening window confirmed for tonight.** P2 pause staged: at window open
   `ollama ps` → `ollama stop <name>` → verify + log RAM freed; re-warm at
   close. M4 control row + heavy cells inside window; M1 control row as soon
   as 8B lands.
4. **QoS/SSH finding promoted** to its own results-doc subsection under M1
   design inputs: "headless M4 requires launchd services; non-interactive SSH
   spawns are QoS-frozen" — seed of the headless-server setup spec, together
   with the Tailscale persistence note.
5. Downloads: all 6 must be size-sanity-checked against HF content-length
   before any transfer.

### Checkpoint 1/2 execution + wiring (2026-06-11 ~23:30–00:00)

**M4 brew QoS freeze — second confirmed instance (Jesse's archaeology):** the
original `brew install tailscale` over non-interactive SSH froze; recovery
required killing the full process tree from the henry account (lock-holder was
a ruby child invisible to 'brew' greps) and clearing
`/opt/homebrew/var/homebrew/locks/` wholesale (lock files are UNSUFFIXED — a
`*.lock` glob misses them). Reinstall running interactively by Jesse.
→ Results-doc hard rule (M1 design inputs): **nothing long-running spawns
non-interactively on the M4 without launchd**; recovery = kill full tree +
clear locks directory. henry is NON-SUDO by design; all privileged
provisioning routes through the admin account: provision once via admin +
launchd, zero interactive babysitting thereafter.

**ZeroClaw census (Jesse, tonight):** litellm, memory/knowledge/learning
servers, spend proxy, task runner, ~20 duplicate chroma-mcp processes (likely
leak). ~10 GB estimate stands; P2 pause frees the 9.3 GB Ollama model only.

**Tailscale M1 joined:**
```
$ tailscale up --auth-key=[REDACTED] --hostname=m1-mini
$ tailscale status → 100.77.38.66  m1-mini  jesse.sharratt@  macOS
```
M4 join HOLDS until Jesse posts "M4 ready" (install + admin sudo step).
Agent's parallel background brew retry was stopped (TaskStop) the moment Jesse
reported hands-on recovery — single writer on the M4.

**Wired path live (link-local — en0 has no DHCP lease on either mini; both
sit on the same isolated wired L2 segment, 169.254.233.101 ↔ .220; `route
get` confirms en0 both directions):**

| path | min/avg/max/stddev (ms) | loss |
|---|---|---|
| M1 → M4 WIRED en0 | 0.449 / **0.520** / 0.656 / 0.056 | 0% |
| M4 → M1 WIRED en0 | 0.417 / **0.465** / 0.558 / 0.043 | 0% |
| (before-row) M1 → M4 Wi-Fi | 0.406 / 0.553 / 0.667 / 0.054 | 0% |
| (before-row) M4 → M1 Wi-Fi | 6.076 / **39.614** / 106.090 / 34.884 | 0% |

Wi-Fi's 39.6 ms power-save asymmetry is gone on the wire. Throughput probe
(ssh pipe, 300 MB /dev/zero): 2.68 s ≈ **112 MB/s ≈ saturated GbE**.
Benchmarks will bind RPC to the 169.254 wired addresses.

### Download integrity (size vs HF Content-Length, byte-exact)

| file | bytes (HF) | downloaded | status |
|---|---|---|---|
| Qwen3-30B-A3B-IQ4_XS | 16378073664 | 16378073664 | ✓ |
| Qwen3-30B-A3B-Q4_K_M | 18556686912 | 18556686912 | ✓ |
| Qwen3-32B-Q3_K_M | 15971778208 | 15971778208 | ✓ |
| Qwen3-32B-Q4_K_M | 19762150048 | in flight (91%) | … |
| Qwen3-14B-Q4_K_M | 9001753984 | 9001753984 | ✓ |
| Qwen3-8B-Q4_K_M | 5027784512 | 5027784512 | ✓ |

8B control model transferring to M1 over the wire (~112 MB/s).

---

## Control rows + overnight events (2026-06-12 morning)

### M1 control row — llama.cpp (COMPLETE)

```
$ /usr/bin/time -l llama-bench -m Qwen3-8B-Q4_K_M.gguf -p 512 -n 256 -r 3
| qwen3 8B Q4_K - Medium | 4.68 GiB | 8.19 B | MTL,BLAS | pp512 | 119.32 ± 2.64 |
| qwen3 8B Q4_K - Medium | 4.68 GiB | 8.19 B | MTL,BLAS | tg256 |  10.95 ± 0.31 |
102.13 real · maximum RSS 4,720,377,856 (4.72 GB — inside P3 envelope)
```
Evidence: `mesh-m0-evidence/control-m1-llamacpp-qwen3-8b-q4km.txt`.
M1 solo at 8B Q4 just clears the 10 t/s interactive bar.

### M4 overnight reboot + state change

M4 rebooted ~01:24 (uptime 6:37 at 08:01), **0 console users**. Consequences:
- ZeroClaw only partially restarted: ollama server idle (~130 MB, **no model
  loaded**), python (~240 MB); Chrome/litellm/chroma-mcp absent. ~8.6 GB free.
- en0 link-local address CHANGED (169.254.233.220 → 169.254.65.157) — wired
  path moved. **M1-spec input: static IPs on the wired segment** (link-local
  is not stable across reboots).
- Tailscale M4: still not installed (reboot interrupted Jesse's interactive
  recovery install).

### llama-cli "hang" root-caused — NOT a QoS jail, NOT Metal

The 30-min 99.6%-CPU spinner (and its -ngl 0 repeat) was llama-cli dropping
into **conversation mode and busy-looping on closed stdin** after the SSH
channel died: log shows `> ` interactive prompts; 98.88 s sys vs 18.92 s user
(syscall spin); 5 GB RSS = KV cache at default context. Killed by exact PIDs
2587, 4178 (ps lines in transcript). Phase 2 rule: **benchmarks use
llama-bench / llama-server only; never llama-cli over SSH**. (`llama-bench
--help | head` also wedges on device enumeration + SIGPIPE — don't pipe its
help.)

### Metal-headless verification (KEY POSITIVE FINDING)

```
$ ssh henry llama-bench -m Qwen3-0.6B-Q4_K_M.gguf -p 128 -n 32 -r 1   # 0 console users
| qwen3 0.6B Q4_K | MTL,BLAS | pp128 | 2803.92 | tg32 | 188.09 |
```
**Metal compute works over non-interactive SSH with zero console users** on
macOS 26.2/M4. The headless daemon-only server role is GPU-viable. (The
QoS-freeze finding still stands for nohup/screen/brew-style spawns; held-open
SSH channels and llama-bench work.)

### M4 control row — justification

Jesse gated the M4 control row on the evening window, whose purpose was the
P2 Ollama pause. Post-reboot, `ollama ps` is verified EMPTY (no model
resident) — the pause is a no-op and ZeroClaw is idle. Control row (14B
Q4_K_M, ~9 GB, fits free RAM) run now on that basis; logged here explicitly.

### exo friction log (continued)

- `uv sync` default does NOT install the `mlx` extra → runner fails with
  ModuleNotFoundError. Correct: `uv sync --extra mlx`.
- `--extra mlx` builds MLX from the pinned git fork → **fails unless the
  Xcode "Metal Toolchain" component is separately downloaded**
  (`xcodebuild -downloadComponent MetalToolchain`) — full Xcode alone is not
  enough. More consumer-pod disqualification evidence.
- Custom HF model works: POST /place_instance accepted
  mlx-community/Qwen3-8B-4bit (4.61 GB, 36 layers) though absent from the
  curated /v1/models registry.
- Observed an UNREQUESTED `DownloadPending` for
  mlx-community/gemma-4-26b-a4b-it-6bit (21.8 GB) in node state — pending
  only (0 established connections, nothing on disk), but an alarming default
  for a disk-budgeted machine. Logged as friction.
- MLX runner needs the `mlx` extra (`uv sync --extra mlx`) AND the Metal
  Toolchain Xcode component (`xcodebuild -downloadComponent MetalToolchain`,
  687.9 MB) to build the pinned MLX fork. After both: `mlx OK Device(gpu,0)`,
  runner can load. Full friction chain to a working exo node on macOS:
  Xcode + Metal Toolchain component + rust nightly + node + uv + `--extra mlx`.

### Jesse's exo-process question — ANSWERED

The exo process/dashboard Jesse saw briefly on the M1's localhost (port 52415)
was **mine and expected**: I started `uv run exo` for the single-node
consolation row, discovered the runner failed with `ModuleNotFoundError: No
module named 'mlx'`, killed it by exact PID to fix the MLX/Metal-toolchain
dependency, then restarted it. The transient dashboard was that
start→kill→restart cycle. Nothing unaccounted-for; no second actor.

---

## Tailnet reality — DIAGNOSED (2026-06-12/15)

**The M4 is NOT on the tailnet, and the M1 is double-registered.** Jesse's
admin-console read ("two nodes online: jesses-mac-mini-2 @ 100.74.232.95 and
m1-mini @ 100.77.38.66") is **both the M1** — the M4 never connected.

Ground truth:
```
M1 ComputerName = "Jesse's Mac mini (2)"   ← the "(2)" is the M1, not the M4
M4 ComputerName = "Jesse's Mac mini"       (ssh henry: scutil --get ComputerName)
```
M1 runs a DUAL Tailscale stack:
- brew `tailscaled` (PID 47155, /var/run/tailscaled.socket) = node **m1-mini @
  100.77.38.66** — the one the agent `tailscale up`'d.
- standalone **Tailscale.app + network-extension** (PIDs 1672/3084) = node
  **jesses-mac-mini-2 @ 100.74.232.95** — pre-existing GUI install, surfaced
  after Phase 0. The default `tailscale` CLI talks to THIS one (version suffix
  `-g…` = GUI build), which is why early status reads were confusing.

Authoritative brew-daemon peer list shows only those two nodes — **no M4**:
```
$ tailscale --socket /var/run/tailscaled.socket status
100.77.38.66   m1-mini            jesse.sharratt@  macOS
100.74.232.95  jesses-mac-mini-2  jesse.sharratt@  macOS
```
M4 side: `Tailscale.app/.../Tailscale ip` returns **blank**; the tailscaled
system extension is **not installed** (`/Library/SystemExtensions/*/io.tailscale*`
→ no matches). The GUI standalone app installed, but with **0 console users**
the network extension can't be approved and login never completes.

**This explains every ping result:** my "M1→M4 tailnet" pings to 100.74.232.95
were the M1 hitting its OWN app-stack IP (loopback, 0.5 ms). The M4→M1 100%
loss was the M4 genuinely unable to reach the tailnet (it isn't on it).

→ **Headless-server finding (M1 design input):** neither brew nor the GUI
standalone Tailscale app is a viable headless provisioning path on the M4.
brew froze 4× (disqualified); the GUI app needs a console session to approve
its sysext. The headless path is **`tailscaled install-system-daemon`** (the
open-source CLI daemon as a launchd LaunchDaemon), provisioned once via the
admin account — consistent with the "vendor binary + launchd, provisioned
once" rule. M1 must also resolve its **own dual-stack** (pick one: keep the
brew daemon OR the GUI app, tear down the other, to stop the double node).

⚠️ **[JESSE] required** (GUI/sudo/admin-console — agent will not touch):
1. M1: choose ONE Tailscale stack, remove the other (recommend keeping the
   GUI app for an interactive box, or the CLI daemon for headless parity).
2. M4: install the CLI daemon — `sudo tailscaled install-system-daemon` then
   `sudo tailscale up --auth-key=… --hostname=m4-mini` — over SSH, no GUI
   needed. (The brew/GUI attempts can be discarded.)
3. Cells B (off-LAN client) and E (Tailnet daemon checks) stay BLOCKED until
   a real M1↔M4 (and MacBook) tailnet exists.

Wired LAN cells (A/C′/D) are unaffected — they run over the 169.254 wired
link, no tailnet needed.

---

## Post-reboot M4 state (for Phase 2 setup)

- ZeroClaw honest baseline after the clean reboot (chroma leak reset): only
  idle ollama server (~130 MB, **no model loaded**) + python (~258 MB);
  Chrome/litellm/chroma-mcp not yet relaunched. `ollama ps` EMPTY → P2 pause
  is currently a no-op; ceiling math's ~12 GB M4 contribution holds with
  margin while ZeroClaw is light.
- **Wired link-local IP changed across reboot: M4 en0 = 169.254.65.157**
  (was .220). Benchmarks bind the CURRENT address. (M1 design input already
  logged: static IPs on the wired segment.)

## Control rows (both COMPLETE)

| machine | engine | model | pp t/s | tg t/s | peak RSS |
|---|---|---|---|---|---|
| M1 (M1, 16 GB) | llama.cpp Metal | Qwen3-8B Q4_K_M | 119.32 ± 2.64 | 10.95 ± 0.31 | 4.72 GB |
| M4 (M4, 16 GB) | llama.cpp Metal | Qwen3-14B Q4_K_M | 121.66 ± 0.05 | 11.76 ± 0.04 | 9.07 GB |

(Each is the biggest model that machine holds comfortably solo. M4's newer GPU
edges the M1 on decode despite the larger model — the per-node baseline for
judging whether pooling actually buys anything.)
Evidence: `control-m1-llamacpp-qwen3-8b-q4km.txt`,
`control-m4-llamacpp-qwen3-14b-q4km.txt`.
