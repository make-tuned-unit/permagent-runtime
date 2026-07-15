#!/usr/bin/env python3
"""Pristine grader for the fix-median task (overlaid onto the workspace before
grading so the agent cannot weaken it). Exits 0 on success."""
import sys

from stats import median


def main() -> int:
    cases = [
        ([5], 5),
        ([3, 1, 2], 2),
        ([1, 2, 3, 4], 2.5),
        ([1, 2], 1.5),
        ([7, 7, 7], 7),
        ([10, 2, 8, 4], 6),
    ]
    for nums, want in cases:
        got = median(list(nums))
        if got != want:
            print(f"FAIL: median({nums!r}) = {got!r}, expected {want}", file=sys.stderr)
            return 1

    try:
        median([])
    except ValueError:
        pass
    except Exception as e:  # noqa: BLE001
        print(f"FAIL: median([]) raised {type(e).__name__}, expected ValueError", file=sys.stderr)
        return 1
    else:
        print("FAIL: median([]) did not raise ValueError", file=sys.stderr)
        return 1

    print("ok: median verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
