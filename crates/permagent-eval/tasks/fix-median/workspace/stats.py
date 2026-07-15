"""Simple statistics helpers."""

from typing import List


def median(nums: List[float]) -> float:
    """Return the median of a list of numbers.

    BUG: this returns the upper-middle element for even-length lists instead of
    averaging the two middle values, and it does not handle the empty list.
    """
    ordered = sorted(nums)
    n = len(ordered)
    return ordered[n // 2]
