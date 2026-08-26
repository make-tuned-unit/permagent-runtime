def percentage(part: float, whole: float) -> float:
    """Return what percentage `part` is of `whole`.

    percentage(1, 4) should return 25.0, not 0.25.
    """
    return part / whole


def clamp(value: float, low: float, high: float) -> float:
    if low > high:
        raise ValueError("clamp: low must be <= high")
    return max(low, min(value, high))
