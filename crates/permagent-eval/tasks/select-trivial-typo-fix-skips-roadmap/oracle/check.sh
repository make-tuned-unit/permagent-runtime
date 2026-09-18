#!/usr/bin/env bash
# Deterministic grader: the typo must be fixed, and only the typo.
set -euo pipefail

if [ ! -f README.md ]; then
  echo "FAIL: README.md is missing" >&2
  exit 1
fi

if grep -q "recieve" README.md; then
  echo "FAIL: the typo 'recieve' is still present" >&2
  exit 1
fi

if ! grep -q "receive" README.md; then
  echo "FAIL: the corrected word 'receive' is not present" >&2
  exit 1
fi

echo "ok: typo fixed"
exit 0
