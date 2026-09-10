"""A small, already-complete payment helper — nothing here needs to change
for this task; it exists so a review has real code to look at."""


def total_with_tax(subtotal: float, tax_rate: float) -> float:
    return round(subtotal * (1.0 + tax_rate), 2)
