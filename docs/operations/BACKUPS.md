# Automated Database Backups

## What is backed up

| Database | Path | Owner |
|----------|------|-------|
| memory.db | `~/.permagent/brain/memory.db` | Spectral Brain (knowledge graph) |
| permagent.db | `~/.permagent/spectral/permagent.db` | SessionStorage (sessions, tasks, recipes) |

**Not backed up**: `~/.permagent/secrets/` (keychain material, excluded by design).

## When backups run

1. **Startup (pre-migration)**: If the newest snapshot of a database is older than 20 hours (or absent), a snapshot is taken *before* any schema migration runs. This guarantees a pre-migration snapshot on every daemon version change.
   - `permagent.db` is snapshotted before `SessionStorage::pool_clone()` triggers lazy schema migration.
   - `memory.db` is snapshotted before `Brain::builder().build()` triggers Spectral auto-migration.
2. **Hourly scheduler**: A background loop ticks every hour and snapshots any database whose newest snapshot is older than 20 hours. Dev restarts within the window are a no-op.

## Where backups are stored

```
~/.permagent/backups/
  brain/     memory-20260609T080000Z-daily.db
  spectral/  permagent-20260609T080000Z-daily.db
```

## Rotation policy

- **7 daily** snapshots retained per database.
- **4 weekly** snapshots retained per database (the newest daily per ISO week is promoted).
- Older snapshots are pruned automatically after each successful snapshot.

## Disk safety

Before each snapshot, free space on the destination volume is checked via `statvfs` (queries the actual macOS Data volume, not the sealed system volume). The snapshot is skipped if free space is less than 1.5x the source database file size.

## Snapshot mechanism

**VACUUM INTO** via a read-only `rusqlite` connection. Both databases use WAL journal mode, so VACUUM INTO produces a consistent, compacted single-file snapshot without conflicting with live writers. Each snapshot is written to a `.tmp` file first, integrity-checked with `PRAGMA integrity_check`, then atomically renamed into place. Failed integrity checks delete the `.tmp` file and log an error.

## API endpoints (authenticated)

- `GET /api/backups` — List all snapshots with timestamp, tier, size, and integrity status.
- `POST /api/backups/run` — Force an immediate snapshot of both databases. Returns per-DB result. Use this before any Spectral pin bump or data migration.

## Manual restore procedure

1. Stop the daemon:
   ```sh
   launchctl unload ~/Library/LaunchAgents/ai.permagent.daemon.plist
   # If the daemon is still running:
   pkill -f permagentd
   sleep 2
   ```

2. Copy the snapshot over the live database:
   ```sh
   # Example: restore brain/memory.db from a specific snapshot
   cp ~/.permagent/backups/brain/memory-20260609T080000Z-daily.db \
      ~/.permagent/brain/memory.db

   # Example: restore spectral/permagent.db
   cp ~/.permagent/backups/spectral/permagent-20260609T080000Z-daily.db \
      ~/.permagent/spectral/permagent.db
   ```

3. Restart the daemon:
   ```sh
   launchctl load -w ~/Library/LaunchAgents/ai.permagent.daemon.plist
   ```
   Note: the first load after a restore may fail silently. If the daemon doesn't start, run `launchctl unload` then `launchctl load -w` again with a `sleep 2` between.

4. Verify by hitting the version endpoint:
   ```sh
   curl -s http://localhost:PORT/api/version | jq .
   ```
