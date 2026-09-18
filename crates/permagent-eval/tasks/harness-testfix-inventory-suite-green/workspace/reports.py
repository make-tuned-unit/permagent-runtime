"""Simple text reports built on top of the warehouse."""

from warehouse import Warehouse


def stock_report(warehouse: Warehouse) -> str:
    return (
        f"stock={warehouse.stock} reserved={warehouse.reserved} "
        f"available={warehouse.available()}"
    )
