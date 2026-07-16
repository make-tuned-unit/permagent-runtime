"""Scoring for a small round-based game.

Implement `total_score` so that `python3 test_scoring.py` passes.
"""

from typing import List, Tuple


def total_score(rounds: List[Tuple[int, int]]) -> int:
    """Return the total score for a list of (hits, multiplier) rounds.

    Base score is the sum of hits * multiplier across all rounds. If the list is
    non-empty and every round has hits > 0, add a flat bonus of 50.
    """
    raise NotImplementedError("implement total_score")
