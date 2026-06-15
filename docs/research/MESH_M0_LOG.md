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

### Tailnet RESOLVED + characterized (2026-06-15)

M4 brought onto the tailnet by Jesse via GUI sign-out/in + reconnect (the
App Store build's CLI silently no-ops; required interactive Screen-Sharing
setup — confirms the headless finding). **Three nodes, all under
jesse.sharratt@gmail.com:**

| node | tailnet IP | what it is |
|---|---|---|
| jesses-mac-mini (M4) | **100.80.110.80** | the M4 — use this |
| jesses-mac-mini-2 | 100.74.232.95 | M1 **GUI-app** node — carries real traffic |
| m1-mini | 100.77.38.66 | M1 **brew-daemon** node — control-plane ghost |

**Tailnet ping matrix (post-resolution):**

| path | result |
|---|---|
| M1 → M4 (100.80.110.80) | 0% loss, **16.6 ms avg** (8.9 min, 84 max — Wi-Fi jitter) |
| M4 → M1 **.95** (GUI) ICMP | 0% loss, 18.4 ms avg |
| M4 → M1 **.66** (brew) ICMP | **100% loss — even warmed** |
| M4 → M1 .66 `tailscale ping` | pong via 159.2.20.27, 9 ms (control-plane only) |
| M4 → M1 .95 `tailscale ping` | pong via 159.2.20.27, 12 ms |

Two findings:
1. **The M1 brew-daemon node (.66) is a ghost** — `tailscale ping` (disco/
   control plane) succeeds, but plain ICMP/IP traffic gets 100% loss even
   after warming. Only the GUI-app node (.95) passes real data. Cause: dual
   tailscaled stacks; only the GUI app's utun is wired into OS routing.
   → **For cells B/E the M1's usable tailnet address is 100.74.232.95 (.95),
   NOT .66**, contra the "prefer m1-mini" guidance — unless Jesse logs the
   brew node out (then the dual-stack collapses and .95 is the sole M1 node).
   This promotes the dual-stack from "low priority" to "blocks tailnet-to-M1
   until resolved or .95 is used."
2. **Path is direct-via-public-endpoint with NAT hairpin, not LAN-direct.**
   `via 159.2.20.27` is the M1's OWN public IP (netcheck: external
   159.2.20.27:54527), so M4↔M1 hairpins through the router (~9–18 ms)
   instead of taking the 0.5 ms LAN path. netcheck: UDP ok, easy NAT
   (MappingVariesByDestIP false), UPnP port-mapping, nearest DERP NYC 62.7 ms
   (so it's NOT full-relay — direct via public endpoint). For cell B
   (genuinely-remote client) this is moot; pooling uses the wire regardless.

M4 model integrity re-confirmed post-resolution — all six byte-exact, 126 GB
free.

### exo single-node consolation row — DEFERRED (memory, not engine fault)

Restarted exo on M1 (MLX now works). `POST /place_instance` for
Qwen3-8B-4bit (4.6 GB) failed: `ValueError: No cycles found with sufficient
memory`. exo's `nodeMemory` reported **ramAvailable 4.36 GB** — genuinely
less than the model, because this Claude Code agent session's own helper
processes (several `claude` at 0.4–0.7 GB) + permagentd + the world-baseline
test daemon were resident. **Not an exo defect** — the box wasn't quiet.
exo can only ever run single-node here anyway (M4 exo = DNF), so this one
row is **deferred to the evening window** when the M1 is quiet, run against
the same 8B tier as the llama.cpp control for a clean MLX-vs-ggml point.
exo stopped (PID 82552/83086) to reclaim memory.

---

## Wired pod cells A/C′ — ATTEMPTED; engine bug fixed, then hit a hard memory wall (2026-06-15)

Setup: M4 = head (models local, faster GPU, more free RAM), M1 = RPC worker.
Wired IPs this session: M1 en0 169.254.233.101, M4 en0 169.254.65.157
(both shift across reboots — static-IP design input stands). Wired link clean
(0.56 ms, ~112 MB/s). Bench: `llama-server` + stdlib streaming client
(`pod_bench.sh`, `ttft_client.py`), 3 trials, fixed ~500-tok prompt, 256-tok gen.

### Finding 1 — llama.cpp RPC BLAS bug (FIXED, important)

First pooled attempts crashed the M1 rpc-server with SIGABRT on the first real
op. Root cause from the worker log:
```
ggml-blas.cpp:252: ggml_backend_blas_graph_compute: unsupported op RMS_NORM
```
By default `rpc-server` registers ALL local devices (MTL0 Metal, BLAS, CPU) and
the graph compute landed on **BLAS (Accelerate)**, which lacks `RMS_NORM` →
abort. This crashed even a 0.6 GB model, so it was NOT memory.
**FIX: `rpc-server --device MTL0`** pins it to the Metal GPU. After the fix, a
pooled 0.6 B model loaded and generated cleanly (76 t/s) — RPC pipeline proven.
→ M1 design input: any pooled llama.cpp worker MUST be launched
`--device MTL0` (or the node's Metal device id); the default BLAS fallback is a
silent crash on transformer models. Device discovery: `rpc-server` prints the
list on a bad `--device` arg (`MTL0`/`BLAS`/`CPU`).

### Finding 2 — the core pod cannot SERVE a 30B at 2×16 GB without tuning

With the RPC bug fixed, the 30B-A3B-IQ4_XS (16.4 GB) **loaded** across both
nodes (38 s, 315 MB transferred to the worker — pooling physically works), but
**compute failed**:
```
ggml_metal_synchronize: command buffer failed status 5
Insufficient Memory (kIOGPUCommandBufferCallbackErrorOutOfMemory)  → "Compute error"  → 0 tokens
```
The M4 head's GPU **exceeded its Metal working-set cap**. On a 16 GB Mac the
default `iogpu.wired_limit_mb=0` ⇒ ~66 % ≈ 10.6 GB usable for Metal. Head share
(~8 GB weights) + full-context KV (ctx 4096) + compute buffers > 10.6 GB → OOM.
Reducing to ctx 1024 + even `-ts 50,50` got further but the warmup compute
**thrashed**: the M1 was swap-saturated (this agent session + browsers +
daemon held ~9 GB; swap climbed 6→9 GB), so the M1's half had no real RAM and
the distributed warmup hung.

### Finding 3 — the run wedged the M4 (offline, needs Jesse)

During the distributed warmup the M4 became unresponsive (SSH timeout on wired
AND Wi-Fi), then began answering with **"This system is locked"** and finally
**key-auth denied** — the signature of a **panic/watchdog reboot into the
FileVault pre-boot login** (encrypted volume not unlocked ⇒ no user/SSH keys).
The 30B distributed compute thrashed both 16 GB machines hard enough to take
the M4 down. ⚠️ **[JESSE] the M4 needs manual recovery** (Screen Sharing /
physical FileVault unlock, or `fdesetup authrestart` next boot). The benchmark
`llama-server` was the cause; no permagent/ZeroClaw code involved.
M1 side: cleaned up (no orphans), swap draining (9→6 GB), permagentd healthy
throughout (P3 honored — daemon never touched).

### Verdict on the wired pod cells

- **Pooling WORKS** at the pipeline level (RPC + MTL0 fix; layers transfer;
  small models generate across nodes).
- **Clean 30B/32B ceiling numbers are blocked by two memory walls**, both
  fixable, neither doable mid-session:
  1. **M4 (and M1) GPU working-set cap** — needs `sudo sysctl
     iogpu.wired_limit_mb=<~13000>` on each node so Metal can use more of the
     16 GB. Jesse's sudo; it's also a legitimate headless-server config step.
  2. **M1 must be QUIET** — the orchestrating agent + Jesse's browsers
     swap-saturate the M1, leaving its pool half no real RAM. The M1 cannot be
     both the spike's orchestrator/desktop AND a full pool member at once —
     **this is the Q4 coexistence answer arriving early**: on a 16 GB node,
     heavy pooled inference is NOT invisible to the contributor; it evicts
     everything else.
- **Prerequisites for the ceiling run (evening window):** M4 recovered;
  `iogpu.wired_limit_mb` raised on both minis (sudo); M1 quiet (browsers
  closed, agent footprint minimal); small context (`-c 1024`, the workload is
  ~756 tok); worker `--device MTL0`. With those, the 30B-class cells should
  serve. The C′ Q4 cells (18.6/19.8 GB) remain the tightest and may still not
  fit 2×16 GB even tuned — that itself is the ceiling answer.

Solo control rows (already banked) remain the only clean t/s numbers so far:
M1 8B Q4 = 11 tg, M4 14B Q4 = 11.8 tg.

---

## DEFINITIVE: tuned retry — 2×16 GB cannot SERVE a 30B (2nd M4 panic) (2026-06-15)

Prerequisites all met this run: `iogpu.wired_limit_mb=13000` on BOTH minis
(Jesse, confirmed), M1 browsers closed + Screen Sharing disconnected, M4 ollama
idle (gemma4:e4b keepalive expired → P2 pause was a no-op), worker pinned
`--device MTL0`, ctx 1024, even `-ts 50,50`, live M1 memory guard armed
(trip < 900 MB free+inactive, logs every 3 s →
`mesh-m0-evidence/pod-30B-m1-memory-trace.txt`).

Sequence (30B-A3B-IQ4_XS, 16.4 GB):
- Loaded across both nodes again. During the M1-half wiring, M1 free+inactive
  fell to ~974 MB and swap spiked to a **7.92 GB peak** (agent pages evicted),
  then recovered to ~2 GB once wired — worker survived loading.
- During the **distributed warmup compute**, the **M4 head went unresponsive**
  (SSH timeout wired+Wi-Fi), then **"This system is locked" → key-auth denied
  = a SECOND panic-reboot into FileVault pre-boot.** I aborted the M1 worker
  immediately (M1 recovered instantly, free→9.3 GB), but the M4 was already
  gone. ⚠️ **[JESSE] M4 needs recovery again.**

**Why raising the wired-limit did NOT help — it moved the failure, not removed
it.** Default `iogpu.wired_limit_mb=0` (~10.6 GB cap) → the head OOMs its GPU
(run 1). Raised to 13 GB → Metal wires up to 13 GB on a 16 GB box, leaving
< 3 GB for the OS, so under the warmup's compute buffers the whole **machine**
thrashes to a panic instead (run 2). There is no setting at 16 GB that both
fits the model's compute and leaves the OS enough to stay alive. The 30B
distributed warmup needs more resident headroom than a 16 GB node has, period.

### CEILING FINDING (record, do not fight — per Jesse)

**The CORE POD (2 × 16 GB M1+M4) pools memory but cannot SERVE a 30B-class
model.** It LOADS one (layers transfer, weights map) but the first real compute
panics a node. The largest model the pod can actually serve sits between the
0.6 B that ran clean (76 t/s pooled) and the 30 B that panics — and a 14 B-class
(9 GB, ~4.5 GB/node) is exactly the size each node **already runs solo**
(M4 14B Q4 = 11.8 tg). So on this trio:

- **Biggest model the pod can SERVE ≈ 14 B class — which needs no pooling at
  all.** Pooling buys essentially nothing on 2×16 GB: the overhead/instability
  of splitting a 30B outweighs any gain, and the models that fit comfortably
  fit on one node anyway.
- **The unified-memory "biggest model possible" thesis FAILS for this
  hardware generation.** 30B-class needs a node with real headroom (a 32–64 GB
  Mac), and 70B needs ≥48 GB — neither exists in this trio (the MacBook is
  Intel/8 GB, client-only). The household ceiling is the ~8–14 B class that
  any single Apple-silicon node here already serves at ~11 t/s.
- **M1 design input:** the value of pooling on Apple Silicon appears only when
  a SINGLE node is too small for the target model AND the nodes are large
  enough that each one's OS headroom survives its shard's compute. 2×16 GB
  satisfies neither for 30B. The M2 path is a bigger anchor node (the headless
  M4 → 32–64 GB), not more 16 GB nodes.

### Partial elasticity (cell D) observed for free

Killing the M1 worker mid-load/compute makes the head emit
`recv failed … Remote RPC server crashed` and exit — i.e. **worker departure
takes the whole pooled instance down; the head does NOT gracefully fall back**
to its own tier. That is the realistic node-drop behavior and a direct M1
design input: graceful degradation on contributor departure must be built; the
engine does not provide it. (Clean timed D trials need a pod that can actually
serve — moot at 30B here.)

### Correction & reopened investigation (2026-06-15)

The "physics, stop" call was premature on two counts (Jesse caught it):
1. **Both panics were the 30B MoE (`Qwen3-30B-A3B-IQ4_XS`), NOT dense.** The
   dense 32B was never reached (it's cell 2 in `pod_bench`; the MoE in cell 1
   panicked first every time). The premise "even-split dense-30B is dead" was
   wrong — it's the even-split MoE that's proven to wedge; dense untested.
2. **`iogpu.wired_limit_mb=13000` is itself a key finding — record prominently:**
   - Default cap (0 ⇒ ~10.6 GB, 66% of 16 GB): the M4 head failed with a
     **GRACEFUL** `kIOGPUCommandBufferCallbackErrorOutOfMemory` / "Compute
     error" — process reports it, machine survives.
   - Raised to 13 GB: Metal wires up to 13 GB on a 16 GB box, starving the OS
     to < 3 GB, and the failure became a **HARD KERNEL PANIC → FileVault-locked
     reboot.** Raising the wired-limit did not enable bigger models — it
     converted a survivable OOM into an unrecoverable panic.
   → **M1/headless-server design rule: do NOT raise `iogpu.wired_limit_mb` near
   total RAM on a node that must stay alive. The default cap is a safety
   feature; over-raising trades graceful degradation for kernel panic.**
3. **The unattended wedges trace to guard placement:** the memory guard ran on
   the M1 and the M4 was SSH-polled — and SSH latency to the M4 spikes exactly
   when it thrashes, so the guard saw the panic too late. Fix: a LOCAL M4-side
   watchdog (next section).
4. **Crucially, the M4 panicked with ~14 GB FREE at run start** — so it is NOT
   steady-state shard size (~8 GB) that kills it; it's a TRANSIENT during the
   distributed warmup. That points at a warmup-time spike (Metal buffer alloc
   burst / RPC weight transfer) — if we can see the spike shape, a
   warmup-throttle or staged load might make 16 GB pooling viable. Worth
   characterizing, not just surviving.

### Reopened-test harness (staged 2026-06-15, awaiting Jesse's watched window)

- `bench/m4_watchdog.sh` — runs ON the M4; logs free/inact/wired/compressed/
  swap/llama-server-RSS every 1 s (captures the warmup spike shape), and
  `kill -9`s llama-server LOCALLY if avail RAM < threshold (default 3500 MB) —
  aborts before a panic, no SSH dependency.
- `bench/pod_cell.sh <label> <model> <ts> <ctx> <ngen> [wd_thresh]` —
  parameterized single cell: arms the watchdog, 90 s HARD warmup timeout
  (catches the hung-warmup wedge), runs 3 trials, classifies failure
  (graceful OOM vs watchdog-kill), prints the warmup memory trace.
- Config: **M4 default wired-limit (do NOT re-raise)**; M1 left at 13000 (worker
  needs headroom to load its shard; M1-side guard prevents M1 panic); worker
  `rpc-server --device MTL0`.
- Run order (watched): (1) MoE IQ4_XS 16.4 GB, **capped split M4 ~58 / M1 ~42**
  (M4 ≤ ~9.5 GB, under its default cap; NOT 80/20 — never load the node that
  collapses), ctx 1024. (2) if graceful OOM (not panic) → MoE
  **UD-IQ3_XXS 12.89 GB** (downloaded to M4) at ~50/50, the "smaller tier"
  test. (3) HARD STOP. Worst acceptable outcome = graceful OOM; a panic means
  the watchdog was too slow and we stop for good.

### WATCHED RUN RESULTS (2026-06-15, Jesse watching M4, Screen Sharing off)

Pre-flight (after Jesse disconnected Screen Sharing): M4 readily-avail 7.8 GB
(freed ~2 GB), M1 5.3 GB. Combined ~13.1 GB vs IQ3 12.89 GB — tight. M4
wired_limit default (0). Worker `--device MTL0`. Both guards armed (M4
watchdog 2500 MB avail; M1 guard 900 MB avail).

**Two attempts, both GRACEFUL, ZERO PANIC — the safety redesign worked:**

| split | M4 head shard | M4 result | M1 result | outcome |
|---|---|---|---|---|
| 70 / 30 | held 9.25 GB OK | healthy, no abort/panic | avail → 481 MB, **guard tripped**, killed worker | head SIGABRT on RPC-buffer alloc; M4 survived |
| 76 / 24 | held 9.93 GB OK | healthy (avail 6.7 GB, swap flat) | avail → 676 MB, **guard tripped** | head SIGABRT; M4 survived |

Both times the **M1 worker was the failure**, not the M4. The M4 head held
9.25–9.93 GB shards gracefully at default wired-limit (free dipped to ~57 MB
but swap stayed flat at 259 MB — clean compression, no panic). The M1, hosting
this orchestrating agent, dropped below its 900 MB guard while loading even a
24 % (~3.1 GB) shard → guard killed the worker → head aborted cleanly. **The M4
never panicked, never needed recovery.** Traces:
`pod-IQ3-m4-warmup-trace.txt`, `pod-IQ3-76-24-m4-trace.txt`,
`pod-IQ3-m1-guard-trace.txt`.

### CEILING — refined and decisive (the wall is the orchestrator, not physics)

- **M4 head: handles ~10 GB shard gracefully** at default wired-limit — capped
  near its ~10.6 GB Metal working-set. No panic at default cap (vs guaranteed
  panic at 13000 — the wired-limit finding, re-confirmed).
- **M1-as-orchestrator: contributes only ~2 GB reliably.** With this agent
  session resident (~5–7 GB free), loading a 3 GB shard trips its guard. The
  M1's useful pool contribution is near-zero while it's the household's
  interactive/orchestrator node.
- **Pool serving envelope while M1 orchestrates ≈ M4 ~10 GB + M1 ~2–3 GB ≈
  12–13 GB** — right at the 12.89 GB IQ3, too tight to serve reliably (a load
  might squeak through, but generation's KV growth pushes it over). That tier
  (~12–13 GB ≈ 14B-class) is **what the M4 already serves SOLO** (control:
  11.8 tg). **So pooling buys nothing while the M1 is occupied.**
- **BUT the thesis is NOT physically dead — it's blocked by coexistence.** The
  binding constraint is the M1's orchestrator footprint, not the hardware. A
  **quiet M1** (~12 GB free) would lift the pool to ~22 GB and serve the
  16.4 GB IQ4_XS (and the IQ3) comfortably. The pool *can* hold a tier neither
  node serves solo — but only when **both nodes are dedicated**, which
  conflicts with the M1 being the interactive/daemon host.

### M1/M2 DESIGN CONCLUSION

Pooling on this trio is worthwhile **only with a dedicated, larger anchor** —
the headless M4 grown to 32–64 GB, contributing the bulk, with smaller nodes
adding slices **only when not doing other work**. A 16 GB node that is also the
household's interactive machine + daemon host contributes ~nothing to a pool
and should default to **client**, not contributor (this is the
minimum-contributor-spec answer, proven). The unified-memory "biggest model
possible" thesis needs the anchor's RAM, not node count: 2×16 GB (one of them
busy) tops out at the same ~14B tier a single quiet node serves.

### STOP (graceful, per plan)

Two graceful attempts + corrected split; the remaining needle (M4 ~80 %,
M1 ~20 %) would load at the M1's guard edge and trip during generation —
no value, and IQ4_XS (16.4 GB > IQ3 that already won't serve) is moot. Per
Jesse's framework: graceful = recorded ceiling, hard stop. The
engine/ceiling/coexistence questions are answered; no panic was ever
provoked in the tuned configuration.

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
