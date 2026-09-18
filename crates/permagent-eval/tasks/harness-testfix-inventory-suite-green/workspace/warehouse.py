"""A tiny in-memory warehouse used by the CLI and reports modules."""

from inventory import apply_restock, available_quantity, reserve_units


class Warehouse:
    def __init__(self, initial_stock: int, max_capacity: int):
        self.stock = initial_stock
        self.reserved = 0
        self.max_capacity = max_capacity

    def reserve(self, qty: int) -> int:
        self.reserved += qty
        return reserve_units(self.stock, qty)

    def restock(self, qty: int) -> int:
        self.stock = apply_restock(self.stock, qty, self.max_capacity)
        return self.stock

    def available(self) -> int:
        return available_quantity(self.stock, self.reserved)
