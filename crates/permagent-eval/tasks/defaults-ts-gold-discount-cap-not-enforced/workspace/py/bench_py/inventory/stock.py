def available_quantity(total: int, reserved: int) -> int:
    return max(0, total - reserved)


def is_low_stock(available: int, threshold: int) -> bool:
    return available <= threshold
