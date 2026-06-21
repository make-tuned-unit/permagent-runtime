#!/usr/bin/env bash
# PostToolUse hook: auto-format Rust files after Claude edits/writes them.
# Makes "always run cargo fmt" deterministic (100%) instead of advisory (~80%
# via CLAUDE.md). Reads the hook payload from stdin, formats only the edited
# file if it's a .rs file. Fast, scoped, non-blocking on non-Rust files.

set -euo pipefail

# Claude Code passes the tool payload as JSON on stdin.
payload="$(cat)"

# Extract the edited file path (Edit/Write tools put it in tool_input.file_path).
file_path="$(printf '%s' "$payload" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("tool_input",{}).get("file_path",""))' 2>/dev/null || true)"

# Only act on Rust files.
case "$file_path" in
  *.rs) ;;
  *) exit 0 ;;
esac

[ -f "$file_path" ] || exit 0

# Format just this file (fast; doesn't touch the whole tree).
# rustfmt respects the project's rustfmt.toml / edition via the nearest config.
if command -v rustfmt >/dev/null 2>&1; then
  rustfmt --edition 2021 "$file_path" 2>/dev/null || true
fi

exit 0
