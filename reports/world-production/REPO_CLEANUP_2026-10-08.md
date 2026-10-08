# Repository cleanup — October 8

The user explicitly requested stale builds, tests, worktrees and caches be cleaned to relieve disk pressure. Two verified removal passes increased observed free space by **12.33 GiB** combined. Concurrent work also changed disk usage, so the final free-space reading is not attributed entirely to this cleanup.

- Removed ten inactive, clean worktrees with their branch refs and commits retained.
- Archived and removed nine inactive worker checkouts: seven `dagK-*` lanes last edited September 26, `home-layout-fix-20260928`, and the completed `wq-record-20261007` lane. Their complete staged/unstaged binary patches, modified working files, untracked files and ignored harness source were preserved. Every archive member was hash-verified and both patches were validated through a disposable Git index before removal. Original branch refs remain intact.
- Removed 1,399 rebuildable artifact paths: local Cargo registry source/download caches, unused build-script binaries/static intermediates/test executables, and duplicate old review/preservation backups. Retained registry indexes, runtime libraries, current daemon/review app, all models, current Record evidence and test source.
- The first preflight detected an active `permagent-eval` executable under the older harness target. It stopped before deleting anything. That entire target was excluded; no evaluation process was stopped.
- Main worktree's previous dirty/untracked status entries all remain. A separate active worker added one brief during verification; it was left intact. Current protected binaries match their pre-cleanup hashes.

Seven worktrees remain: main, `h1-eval-ab`, active `harness-fix-20260928`, `harness-verify-20260928`, integration `world-production-20261006`, `wq-calendar`, and `wq-world`. No branches, installed apps, shared production services, global caches or unrelated repositories were deleted.

## Recovery and audit

Private archives live at `/Users/j/Documents/dev/permagent-worktrees/.shared-target/cleanup-archives-20261008/dirty-worktrees/`. Each checkout has `manifest.json`, `staged.patch`, `working.patch`, `files.tar.gz` and `RESTORE.txt`. Recreate from its retained branch, apply the staged patch with `git apply --index`, apply the working patch, then restore the archived files. The manifest preserves deleted paths, modes, symlink targets and file hashes. Rebuildable `.hermit`, `node_modules` and generated `dist` were excluded from archives. Two old voice-test log archives are preserved in the parent archive directory.

[Initial inventory](REPO_CLEANUP_INVENTORY_2026-10-08.json), [artifact plan](REPO_CLEANUP_PLAN_2026-10-08.json), [first pass receipt](REPO_CLEANUP_RESULT_2026-10-08.json), [archive plan](REPO_WORKTREE_ARCHIVE_PLAN_2026-10-08.json), [archive removal receipt](REPO_WORKTREE_ARCHIVE_RESULT_2026-10-08.json).

After cleanup, the isolated daemon successfully started under its unchanged 25GiB startup guard in supervisor session 15006, one-hour bound. Remaining verification continues from source; deleting build artifacts does not imply new native acceptance.
