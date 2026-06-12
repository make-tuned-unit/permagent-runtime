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

Pinned decisions awaiting [JESSE]:
- [ ] P2: may ZeroClaw be cleanly stopped during benchmark windows?
- [ ] ZeroClaw process names + DO-NOT-TOUCH path list
- [ ] MacBook specs (chip / RAM / disk / macOS)
- [ ] Wire both minis for the spike? (strongly recommended)
- [ ] Tailscale: install on M1 (and M4?) or drop the Tailnet cells
- [ ] P6: evening benchmark window
- [ ] Ladder approval before any download
