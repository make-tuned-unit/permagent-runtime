"""Pricing module — has a discount calculation bug (see test_checkout.py)."""


def apply_discount(price: float, percent_off: float) -> float:
    # BUG: subtracts the raw percent instead of the fractional discount.
    return price - percent_off
