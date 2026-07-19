#!/usr/bin/env python3
"""Runnable test for total_score. Exits 0 on success, non-zero on failure."""
import sys

from scoring import total_score


def main() -> int:
    cases = [
        ([(3, 2), (4, 1)], 60),          # 10 base + 50 flawless bonus
        ([(1, 1)], 51),                  # 1 base + 50 bonus
        ([(2, 3), (1, 1), (5, 2)], 67),  # 17 base + 50 bonus
        ([(0, 5), (4, 1)], 4),           # a zero-hit round => no bonus
        ([(4, 1), (0, 9)], 4),           # zero hits anywhere => no bonus
        ([], 0),                         # empty => 0, no bonus
        ([(10, 0)], 50),                 # hits>0 (bonus) but multiplier 0 base
    ]

    for rounds, want in cases:
        got = total_score(list(rounds))
        if got != want:
            print(f"FAIL: total_score({rounds!r}) = {got!r}, expected {want}", file=sys.stderr)
            return 1
    print("ok: scoring verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
