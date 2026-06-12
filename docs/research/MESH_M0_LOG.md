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
