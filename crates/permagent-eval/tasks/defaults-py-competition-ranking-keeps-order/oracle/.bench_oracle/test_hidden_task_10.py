"""Hidden oracle for task-10. Copied over the finished workspace at grading time.

Deliberately imports only `sys` (a built-in, unshadowable) and the workspace
package the agent is meant to fix, which is declared under `deliverables:` in
task.yaml. `unittest` would be a shadowing hole: a workspace `unittest.py`
could feed the grader rigged results, so the import discipline in
permagent_eval::task rejects it -- correctly.
"""

import sys

from bench_py.reports.summary import rank_players

FAILURES = []


def check(label, got, want):
    if got != want:
        FAILURES.append(f"{label}: got {got!r}, want {want!r}")


check("ties share rank and skip",
      rank_players([("a", 10), ("b", 9), ("c", 9), ("d", 7)]),
      [("a", 1), ("b", 2), ("c", 2), ("d", 4)])
check("preserves input order",
      rank_players([("d", 7), ("a", 10), ("c", 9), ("b", 9)]),
      [("d", 4), ("a", 1), ("c", 2), ("b", 2)])
check("all tied",
      rank_players([("x", 5), ("y", 5), ("z", 5)]),
      [("x", 1), ("y", 1), ("z", 1)])
check("no ties",
      rank_players([("a", 3), ("b", 2), ("c", 1)]),
      [("a", 1), ("b", 2), ("c", 3)])

if FAILURES:
    for line in FAILURES:
        print("FAIL " + line, file=sys.stderr)
    sys.exit(1)
print("ok")
