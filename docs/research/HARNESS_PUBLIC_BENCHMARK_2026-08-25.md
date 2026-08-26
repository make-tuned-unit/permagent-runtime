# Benchmarking the Permagent coding harness on a public suite

2026-08-25. Design + the infrastructure decision behind it. Every claim about
someone else's number carries the URL and the date it was read; where a page
could not be fetched it says so instead of guessing.

## 1. What harnesses actually publish

| who | suite | model | score | cost disclosed? | source (read 2026-08-25) |
|---|---|---|---|---|---|
| Anthropic | SWE-bench Verified | Claude 3.5 Sonnet (new) | 49% | no | [anthropic.com/engineering/swe-bench-sonnet](https://www.anthropic.com/engineering/swe-bench-sonnet), 2025-01-06 |
| OpenAI | SWE-bench Verified | GPT-5 | 74.9% | no | own launch materials; run on a **fixed n=477 subset**, not all 500 |
| Cursor | Terminal-Bench 2.0 / SWE-bench Multilingual | Composer 2 | 61.7% / 73.7% | token prices, not run cost | [cursor.com/blog/composer-2](https://cursor.com/blog/composer-2), 2026-03-19 |
| Aider | polyglot (225 tasks) | GPT-5 (high) | 88.0% | **yes — $29.08 per full run**, plus tokens and time | [aider.chat/docs/leaderboards](https://aider.chat/docs/leaderboards/), page last updated 2025-11-20 |
| OpenHands | OpenHands Index | — | — | yes, a `cost_per_instance` field | [index.openhands.dev](https://index.openhands.dev), blog 2026-01-29 |
| Goose (Block) | — | — | — | — | in-repo bench framework; no published headline number found. `block.github.io/goose/docs/guides/benchmarking/` 404s |

Two things fall out of that table. First, **almost nobody publishes cost**, so
a harness that does is saying something the leaderboards do not. Aider and
OpenHands are the exceptions and are therefore the models to copy. Second,
**the scaffold is doing the work and is never held constant** — OpenAI ran 477
of the 500 Verified tasks; Anthropic used a SWE-agent-derived scaffold with
"hundreds of turns"; Cursor declines to report Verified at all. A number
without its harness is not a number.

That instability is now official. OpenAI has stopped evaluating on SWE-bench
Verified — "flawed tests that reward shortcuts, plus training-data leakage
that inflates scores" — and recommends not using it as a single source for
product claims (`openai.com/index/why-we-no-longer-evaluate-swe-bench-verified/`;
the page returned HTTP 403 to us and this is quoted from a repost, so treat
the wording as secondary). Scale AI's SWE-bench Pro (1,865 tasks, 731 public)
answers the same problem by building its public and held-out splits from
**GPL-licensed repos** specifically to raise the cost of contamination
([labs.scale.com/papers/swe_bench_pro](https://labs.scale.com/papers/swe_bench_pro)).

## 2. Which suite we can actually run — and the numbers that decided it

SWE-bench's own README (read 2026-08-25) asks for **x86_64, 120 GB free
storage, 16 GB RAM, 8 CPU cores**, because evaluation builds a Docker image
per instance.

What we have:

| | M4 (workstation) | M1 (`jessesharratt@m1`, the Strix Docker host) |
|---|---|---|
| arch | arm64 | arm64 |
| RAM | 16 GB, shared with everything else | 16 GB |
| free disk | 68 GiB | **15 GiB** (97% full) |
| Docker | none installed | colima, **aarch64 VM, 2 CPU, 4 GiB RAM**, 40 GiB disk with 15.1 GB already in images |

Every one of the four requirements fails: wrong architecture on both hosts,
one eighth the RAM in the VM that has Docker, a quarter of the CPUs, and an
eighth of the disk on the machine that would have to hold the images. x86_64
emulation on a 2-CPU/4 GiB aarch64 VM is not a slow path, it is a
non-starter — and unofficial arm64 rebuilds of the instance images change
test outcomes, which would make the resulting number worse than no number.

**Decision: Aider polyglot.** It is a recognised public suite with a public
leaderboard that already reports the two things we care most about (cost and
tokens), and its Python exercises run on a stock toolchain. Verified on the
clone: all 34 Python exercises use plain stdlib `unittest`, so grading needs
`python3` and nothing else — no Docker, no network, no per-task install. The
suite is `Aider-AI/polyglot-benchmark` @ `7e0611e7`, frozen since 2024-12-22.

**SWE-bench Verified is deferred, not rejected.** It becomes runnable the day
one of: an x86_64 host with 120 GB free appears; the M1 is given disk and its
colima VM is resized; or we rent cloud eval (SWE-bench added Modal-based
cloud evaluation on 2025-01-11). That is a purchasing decision, not a
research one.

## 3. The reproducibility artefact

`scripts/bench/polyglot_bench.py`, four commands, and the reproducibility
lives in the seams between them:

- **`prepare`** — sorts the exercise names (so filesystem order cannot leak
  in), takes a seeded sample, and materialises each task into its own
  workspace with the reference solution (`.meta/`) deleted. Writes
  `manifest.json` recording the suite repo's git SHA, the harness commit, the
  seed, the selection expression, and the exact task list. Same seed + same
  suite SHA ⇒ same tasks on any machine.
- **`run`** — one task at a time (this Mac is memory-tight), each through
  `permagent run --recipe permagent-coding` in that task's own copy of the
  workspace. `--recipe` and `--text` are mutually exclusive, so the task
  statement is appended as a `prompt:` block to a **verbatim copy** of the
  shipped recipe; both the base recipe's sha256 and the derived file's are
  stored, so nobody has to take our word that the harness instructions were
  not edited to help the model.
- **`grade`** — never grades in the directory the agent touched. It copies the
  agent's solution file(s) into a **pristine** copy of the exercise and runs
  the suite's own tests there. Weakening or deleting a test is therefore
  worthless, and the tamper is recorded as its own field
  (`test_files_tampered`) rather than silently absorbed into a pass.
- **`report`** — markdown table plus the non-comparability clauses, emitted
  automatically rather than remembered.

Evidence per task comes from the runtime's own `cost_ledger` table in
`~/.permagent/spectral/permagent.db`, keyed by a per-run session name: cost,
input/output/cache tokens, call count, tool calls counted from the stored
messages, and **the provider/model that actually served each call**. That last
one earns its place immediately — asking for `deepseek-chat` bills
`deepseek-v4-flash`, which a self-report would never have told us. Subagent
cost (the recipe's cross-model reviewer) is summed separately so it cannot be
quietly folded into the main model's number. A field we cannot source is
`null`, never `0`; a measured zero and an unmeasured one must not look alike.

Results land in `docs/benchmarks/POLYGLOT_PUBLIC.md` as a leaderboard table
keyed by **(date, suite SHA, harness commit, model)** — four keys, because
three of them move independently and a row that loses any one of them stops
being reproducible.

## 4. Preventing contamination — and what we cannot prevent

What the harness controls:
1. `.meta/` (the reference solution) is deleted from the agent's workspace.
2. The workspace is a copy outside the suite clone, with no git history, so
   the solution is not recoverable from the repo.
3. The recipe under test loads only `developer`, `analyze`, `summon` — a
   recipe's extension list **replaces** the profile's, so the Brave and Tavily
   web-search extensions in the operator's config are not present. The agent
   cannot look the answer up.
4. Grading runs against a pristine checkout, so test tampering cannot buy a pass.
5. Every run records the suite SHA, harness commit, seed and task list.

What nothing here controls: these are **public Exercism exercises with public
solutions, frozen since 2024-12-22**, and every model tested was trained after
that. Memorisation is not merely possible, it is likely — the same critique
that made OpenAI retire SWE-bench Verified. The absolute number is an **upper
bound on capability**, and is only meaningful as a *relative* comparison
between two models measured on the identical task list through the identical
harness on the same day.

## 5. What we will not claim

- Not "the Permagent harness scores X on the Aider polyglot benchmark." We run
  a seeded subset of one of six languages under a different protocol. It is
  "the Permagent harness on N Python exercises from the Aider polyglot suite."
- Not a comparison against the Aider leaderboard. Aider allows two attempts
  with test output fed back once; this harness runs its own agentic loop with
  a verify tool and a retry budget bounded only by `--max-turns`. More
  iterations are available here than the leaderboard allows. Different
  protocol, different number.
- Not a model comparison. Two models measured through this harness are being
  compared *as run by this harness* — repo map, structured search, shell,
  verify tool, cross-model reviewer. The scaffold is most of what is measured.
- Not a SWE-bench number of any kind, until the hardware in §2 exists.
- Not any result at n<30 without the word PILOT next to it. At n=10 a single
  task is ten percentage points and the confidence interval is wider than any
  difference the table can show.
