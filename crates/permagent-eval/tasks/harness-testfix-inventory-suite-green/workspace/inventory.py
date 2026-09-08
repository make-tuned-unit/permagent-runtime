"""Stock-tracking helpers for the warehouse service."""


def available_quantity(on_hand: int, reserved: int) -> int:
    """Units on hand minus units already reserved for pending orders."""
    return on_hand - reserved


def reserve_units(on_hand: int, qty: int) -> int:
    """Reserve `qty` units from `on_hand` stock, returning what remains."""
    remaining = on_hand - qty
    return remaining


def apply_restock(current_stock: int, incoming_qty: int, max_capacity: int) -> int:
    """Add incoming stock to current stock."""
    return current_stock + incoming_qty


def low_stock_alert(on_hand: int, reorder_threshold: int) -> bool:
    """Whether on-hand stock is at or below the reorder threshold."""
    return on_hand <= reorder_threshold
