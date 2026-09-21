# Librarian model search — notes (2026-09-20)

## Task
Find next-best open-weight model for nightly Librarian memory-consolidation job.
Qwen3.8-27B split PAUSED (needs 9.5GB free, never available). Fallback: qwen2.5:7b via Ollama.
Hardware: M4 (16GB, dev workstation, 0MB free tonight, ~13GB swap, disk 95% full) + M1 (16GB, headless, idle, ssh via Tailscale).

## Status: STARTING

## Workload (read from librarian.rs, librarian_adjudicator.rs, librarian_atoms.rs)
- describe_one: prompt ~700 tokens (memory content, masked opaque IDs, optional cross-source context), output capped 150 tokens, temp 0.2. System prompt LIBRARIAN_SYSTEM_PROMPT demands EXACTLY 3 labeled fields: FACTS (<30 tok, 1 sentence, no speculation), TERMS (4-10 comma-sep incl inflections), CATEGORIES (2-5 comma-sep). Strict format; parser (extract_labeled_fields/parse_structured_description) accepts labeled lines, JSON, code-fenced JSON, or salvages a "first usable sentence" as last resort. MAX_TERMS=10/MAX_CATEGORIES=5/MIN_TERMS=4/MIN_CATEGORIES=2 enforced post-hoc since qwen3-coder:30b IQ2_M ignored range (18-23 items). 2 retry attempts. Salvage path exists but logged as data-quality loss (~12 memories went unsearchable 2026-08-22 from raw dumps).
- Adjudicator (librarian_adjudicator.rs): supersession Y/N — reply parsed as REPLACED <value> <confidence> / ALL_HOLD / UNKNOWN. Destructive if wrong (retires facts), gated by threshold + predicate allowlist (FUNCTIONAL_PREDICATES only, needs-confirm set disabled). Default OFF (PERMAGENT_LIBRARIAN_ADJUDICATOR unset = off; "shadow" logs only; "apply" retires). Conservative model strongly preferred since wrong REPLACED silently loses a fact.
- Atoms (librarian_atoms.rs): ATOM_SYSTEM_PROMPT — bigger job, consolidates clusters of up to 24 source memories into entity-keyed cited atom lines. Default OFF (flag-gated). This is NOT nightly-default work; only runs when flag on. Heavier prompt, more output.
- Backends (call_local_backends / call_ollama_streaming_pooled): order = PERMAGENT_LIBRARIAN_ENDPOINT (dedicated, OpenAI-compat, backend="ollama"|"llamacpp" via PERMAGENT_LIBRARIAN_BACKEND) > Apple on-device (apple_fm::generate, tried first within call_local_backends when no/failed dedicated endpoint) > Ollama pool default qwen2.5:7b. NOTE: order is: dedicated endpoint wins over mesh pool; if dedicated endpoint down -> call_local_backends which tries Apple on-device FIRST, then Ollama. So effectively Apple FM is already first choice when no explicit remote endpoint, confirming envelope C.
- No explicit "thinking" toggle found yet in this file — need to check if qwen3-class thinking mode is disabled anywhere (prompts have no <think> suppression visible in system prompt; will note as open question / rely on model API "enable_thinking" flags for candidates).
- Calls per night: schedule window default 240min (4hr) starting 02:00 (or 01:45 preflight per qwen38-split.sh comment); volume = "hundreds of calls a night" per task brief; batch runs via run_batch / describe_one loop, idempotent (skips already-described memories).
- PERMAGENT_LIBRARIAN_ENDPOINT confirmed OpenAI-chat-compatible (used for both ollama backend via /v1 and llamacpp backend) - i.e., remote M1 Ollama or llama-server both work through existing config, no code changes needed for envelope B relocation.

## Eval instrument: ~/spectral-local-bench/r40-gate-2026-08-19/gate27b.py
- Targets ANY OpenAI-compatible /v1/chat/completions endpoint (arg 2, default 127.0.0.1:8080). Model name is arg 1.
- Measures: HALL classification accuracy vs ground truth (approvals->fact, task records->event, 20 each), HALL self-consistency on chat turns (call twice, same prompt, compare), RELATIONS triple validity (subject|type|predicate|object|type schema check against predicates.txt/TYPES).
- This is the ADJUDICATOR/relation-extraction gate, not the describe_one FACTS/TERMS/CATEGORIES gate — but structurally reusable (swap SYSTEM prompt + parser) for a describe_one format-adherence + faithfulness bench. Also useful directly for ranking candidates on conservative adjudication (goal 4).
- Other dirs present: r37-librarian-2026-08-18, r38-resonance-2026-08-18 — worth noting but not opened (budget).

## Local hardware check pending next

## Hardware [LOCAL-OBSERVED] 2026-09-20
M4 (dev workstation, local):
- 16 GiB RAM (17179869184 bytes), Apple M4, macOS 26.2 (25C56)
- vm_stat: free pages ~6002*16KB=94MB; swap 12288M total, used 11093M, free 1195M (encrypted)
- disk: 460GB total, 405GB used, 20GB avail (96% full)
- pressure level: 1 at time of check (task brief says level 2 "tonight" - may fluctuate)
- ollama ps: nothing currently loaded (idle at check time)
- local ollama models: qwen2.5-coder:7b (4.7GB), minicpm5-2b-q8 (2.7GB), qwen25-16k (4.7GB), qwen2.5:7b (4.7GB) -- DEFAULT_MODEL

M1 (headless, ssh via Tailscale 100.74.232.95, user jessesharratt):
- 16 GiB RAM, Apple M1, macOS 26.6.2 (25G83)
- vm_stat: free pages ~5791*16KB=90MB but active/inactive modest; swap 3072M total, used 1803M, free 1269M (much healthier than M4)
- disk: 460GB total, 407GB used, 29GB avail (94% full)
- ollama installed at /opt/homebrew/bin/ollama, client v0.23.2, but daemon NOT running ("could not connect to ollama server, run 'ollama serve'") -- idle machine, would need to be started for a remote endpoint
- mainline llama.cpp build present at ~/.local/llama/bin/ (llama-server, llama-cli, etc, built 2026-08-18) -- used today only for the paused Qwen3.8-27B split's ggml-rpc-server + llama-server combo
- both stock Ollama AND mainline llama.cpp llama-server available on M1 -- satisfies hard requirement (7) for either backend

## Web research starting now (budget ~25 fetches)
