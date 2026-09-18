"""Inventory module — has a stock-decrement bug (see test_checkout.py)."""


def decrement_stock(stock: int, quantity: int) -> int:
    # BUG: increments instead of decrementing.
    return stock + quantity
