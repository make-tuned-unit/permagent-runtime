"""Hidden oracle for task-07. Copied over the finished workspace at grading time.

Deliberately imports only `sys` (a built-in, unshadowable) and the workspace
package the agent is meant to fix, which is declared under `deliverables:` in
task.yaml. `unittest` would be a shadowing hole: a workspace `unittest.py`
could feed the grader rigged results, so the import discipline in
permagent_eval::task rejects it -- correctly.
"""

import sys

from bench_py.inventory.pricing import apply_bulk_discount

FAILURES = []


def check(label, got, want):
    if got != want:
        FAILURES.append(f"{label}: got {got!r}, want {want!r}")


check("unambiguous rounding", apply_bulk_discount(999, 3, 0.1), 2697)
check("unambiguous rounding", apply_bulk_discount(200, 4, 0.2), 640)
# 498 * 0.25 = 124.5 exactly -- must round UP to 125, not banker's-round to 124.
check("half a cent rounds up", apply_bulk_discount(498, 1, 0.75), 125)
# Same rule, a different exact-half case: 2002 * 0.25 = 500.5 -> 501.
check("half a cent rounds up", apply_bulk_discount(2002, 1, 0.75), 501)

if FAILURES:
    for line in FAILURES:
        print("FAIL " + line, file=sys.stderr)
    sys.exit(1)
print("ok")
