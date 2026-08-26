"""Hidden oracle for task-03. Copied over the finished workspace at grading time.

Deliberately imports only `sys` (a built-in, unshadowable) and the workspace
package the agent is meant to fix, which is declared under `deliverables:` in
task.yaml. `unittest` would be a shadowing hole: a workspace `unittest.py`
could feed the grader rigged results, so the import discipline in
permagent_eval::task rejects it -- correctly.
"""

import sys

from bench_py.utils.numbers import percentage

FAILURES = []


def check(label, got, want):
    if got != want:
        FAILURES.append(f"{label}: got {got!r}, want {want!r}")


check("quarter", percentage(1, 4), 25.0)
check("three quarters", percentage(3, 4), 75.0)
check("whole", percentage(1, 1), 100.0)
check("zero", percentage(0, 5), 0.0)

if FAILURES:
    for line in FAILURES:
        print("FAIL " + line, file=sys.stderr)
    sys.exit(1)
print("ok")
