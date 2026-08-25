def apply_bulk_discount(unit_price_cents: int, quantity: int, discount_rate: float) -> int:
    """Total price in cents after applying a percentage discount for a bulk order.

    discount_rate is the fraction taken off, e.g. 0.25 means 25% off.
    """
    raw_total = unit_price_cents * quantity
    discounted = raw_total * (1 - discount_rate)
    return round(discounted)
