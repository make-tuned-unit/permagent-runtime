# Permagent Runtime

Fork of Goose (Apache 2.0, AAIF/Linux Foundation) evolved into the Permagent agent OS.

**Stack:** Rust execution engine + Tauri shell + MCP toolsheds + Spectral memory (SQLite temporal KG) + Command Center (Vite + React SPA on localhost)

**Phase 1:** macOS ship target (workspace also builds/tests on Linux in CI), web-based Command Center, repetition-only auto-skills detection, Gmail + Slack integrations, no Mesh sharing, desktop app shipped as a `.dmg` on the GitHub release.

**Canonical spec:** `specs/SPEC_PHASE1_CANONICAL.md`
**Fork strategy:** `specs/FORK_STRATEGY_AND_AUTOSKILLS_SPEC.md`
**Gap decisions:** `specs/SPEC_OPUS_GAP_FIXES.md`

**Goose upstream:** `github.com/block/goose` (forked, do not track upstream except for security patches)
**Goose clone for audit reference:** `/Users/henry/projects/goose`
