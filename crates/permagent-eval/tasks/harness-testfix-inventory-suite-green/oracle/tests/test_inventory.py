"""Inventory regression checks. Run directly: `python3 tests/test_inventory.py`.

Deliberately plain assert-style checks (not unittest) and imports only `sys`
(built-in) plus `inventory`, the workspace module under test -- unittest
itself would be a shadowing hole here (a workspace `unittest.py` could feed
the grader rigged results), so this file avoids it entirely.
"""

import sys

from inventory import apply_restock, available_quantity, low_stock_alert, reserve_units

FAILURES = []


def check(label, cond):
    if not cond:
        FAILURES.append(label)


check("available_quantity", available_quantity(20, 5) == 15)
check("low_stock_alert reports true when at/below threshold", low_stock_alert(2, 5) is True)
check("low_stock_alert reports false when above threshold", low_stock_alert(10, 5) is False)
check("reserve_units reduces stock", reserve_units(10, 4) == 6)

try:
    reserve_units(5, 10)
    check("reserve_units rejects overreservation", False)
except ValueError:
    check("reserve_units rejects overreservation", True)

check("apply_restock is additive below capacity", apply_restock(10, 5, 100) == 15)
check("apply_restock caps at max_capacity", apply_restock(90, 50, 100) == 100)

if FAILURES:
    for label in FAILURES:
        print(f"FAIL: {label}", file=sys.stderr)
    sys.exit(1)
print("ok: 7 checks passed")
