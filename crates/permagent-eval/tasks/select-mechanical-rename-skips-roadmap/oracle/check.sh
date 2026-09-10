#!/usr/bin/env bash
# Deterministic grader: old_name must be gone, new_name must be used
# consistently across both files (and b.py must still import correctly).
set -euo pipefail

if grep -rq "old_name" a.py b.py 2>/dev/null; then
  echo "FAIL: old_name is still present" >&2
  exit 1
fi

if ! grep -q "def new_name" a.py; then
  echo "FAIL: a.py does not define new_name" >&2
  exit 1
fi

if ! grep -q "new_name" b.py; then
  echo "FAIL: b.py does not use new_name" >&2
  exit 1
fi

python3 -c "
import b
assert b.use_it(3) == 7, b.use_it(3)
print('ok: rename verified')
"
