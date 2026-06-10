# permagent doctor

Deterministic diagnostic command that checks daemon health, databases, services,
and local environment. All checks are **read-only** — doctor never writes,
migrates, deletes, or restarts anything.

## Usage

```sh
permagent doctor          # human-readable table
permagent doctor --json   # machine-readable JSON array
permagent doctor --interactive  # LLM-assisted diagnosis (legacy)
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0    | All checks passed (WARNs are allowed) |
| 1    | One or more checks FAILed |
| 2    | Doctor itself errored before completing |

## Checks

| # | Name | Verifies | Possible status |
|---|------|----------|-----------------|
| 1 | `launchd-plist` | `~/Library/LaunchAgents/ai.permagent.daemon.plist` exists and is loaded via launchctl | PASS / FAIL |
| 2 | `daemon-process` | `permagentd` is in the process list (pgrep) | PASS / FAIL |
| 3 | `daemon-reachable` | `GET /status` on localhost returns 200 | PASS / FAIL |
| 4 | `token-file` | `~/.permagent/secrets/daemon_token.json` exists, parses, has 0600 permissions | PASS / WARN / FAIL |
| 5 | `auth-roundtrip` | Protected endpoint returns 401 without token, 200 with token | PASS / WARN / FAIL |
| 6 | `version-info` | Reports daemon version, git SHA, dirty flag, and spectral pin | PASS / WARN |
| 7 | `websocket` | `/events` WebSocket upgrade succeeds | PASS / FAIL |
| 8 | `ui-served` | `GET /ui/` returns 200 (Command Center dist is present) | PASS / WARN / FAIL |
| 9 | `permagent-db` | `permagent.db` exists, passes quick_check, schema version matches compiled constant | PASS / WARN / FAIL |
| 10 | `memory-db` | `memory.db` exists, passes quick_check, reports core tables | INFO / FAIL |
| 11 | `ollama` | Ollama reachable, configured Librarian model present | PASS / WARN / FAIL |
| 12 | `disk-space` | Free space on ~/.permagent volume via statvfs (not df) | INFO / WARN / FAIL |
| 13 | `webkit-caches` | Presence of `~/Library/WebKit/ai.permagent.*` and `~/Library/Caches/ai.permagent.*` | INFO |
| 14 | `backups` | If `~/.permagent/backups` exists, reports newest snapshot age | PASS / WARN / INFO |

## Remediations

Each FAIL or WARN row includes a `->` remediation line in table output, or a
`"remediation"` field in JSON output. Common remediations:

- **launchd not loaded**: `launchctl unload ...; pkill -f permagentd; sleep 1; launchctl load -w ...`
- **token mismatch**: Delete `daemon_token.json` and restart daemon
- **schema version mismatch**: Restart daemon to apply pending migrations
- **Ollama model missing**: `ollama pull <model>`
- **Disk space critical**: Free space; databases and models need room

## JSON schema

```json
[
  {
    "name": "check-name",
    "status": "PASS" | "WARN" | "FAIL" | "INFO",
    "detail": "human-readable description",
    "remediation": "optional fix instruction"
  }
]
```

The `remediation` field is omitted when null (status is PASS or INFO without
action needed).

## Notes

- All database checks use read-only SQLite connections (`SQLITE_OPEN_READ_ONLY`).
- `memory.db` is Spectral-owned — doctor observes but never writes or expects a
  schema_version table.
- The Ollama check reads the configured Librarian model from
  `~/.permagent/librarian_schedule.json`, falling back to the compiled default
  (`qwen2.5:7b`).
- Disk space uses `statvfs` on the volume containing `~/.permagent`, avoiding
  the misleading `/` sealed system volume on macOS.
- The backups check is soft: if the backups directory doesn't exist, it reports
  INFO (the feature may not be merged yet).
