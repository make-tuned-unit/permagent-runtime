# defaults-fixture

A throwaway fixture repo for benchmarking coding agents. It bundles three tiny,
self-contained projects (TypeScript, Rust, Python) and ten small task prompts
under `tasks/`. Each task is solved by editing 1-3 files; grading tests live
outside this repo (see `RUNBOOK.md`) so agents can't read them.

## Layout

- `ts/` -- plain TypeScript (no React runtime, no build step, no `npm install`)
- `rs/` -- a zero-dependency Cargo crate
- `py/` -- a stdlib-only Python package
- `tasks/task-01` .. `tasks/task-10` -- one `PROMPT.md` + `meta.json` per task
- `RUNBOOK.md` -- how the bench driver uses this repo (grading tests are NOT
  in this working copy; see that file)

## Running each language's tests locally

TypeScript (Node >= 22, no install step):

```
cd ts
node --experimental-strip-types some_script.ts
```

Rust (offline, zero external dependencies):

```
cd rs
cargo test --offline
```

Python (stdlib only):

```
cd py
python3 -m unittest discover -s tests -t .
```
