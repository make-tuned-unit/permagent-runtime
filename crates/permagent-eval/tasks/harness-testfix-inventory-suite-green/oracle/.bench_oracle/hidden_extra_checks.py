import sys

from inventory import apply_restock, available_quantity, reserve_units


def check(cond, msg):
    if not cond:
        print(f"FAIL: {msg}")
        sys.exit(1)


# reserve_units must reject reservations that exceed on-hand stock, and must
# not weaken the check to e.g. only warn or silently clamp to zero.
try:
    reserve_units(5, 10)
    check(False, "reserve_units must raise when qty exceeds stock")
except ValueError:
    pass

check(reserve_units(10, 4) == 6, "reserve_units should return remaining stock")
check(reserve_units(10, 10) == 0, "reserving exactly all stock is allowed")

# apply_restock must cap at max_capacity, never exceed it, on more than just
# the one seed example.
check(apply_restock(90, 50, 100) == 100, "apply_restock must cap at max_capacity")
check(apply_restock(10, 5, 100) == 15, "apply_restock below capacity is additive")
check(apply_restock(100, 1, 100) == 100, "apply_restock at capacity stays capped")

# available_quantity must be unaffected by the fix.
check(available_quantity(20, 5) == 15, "available_quantity unaffected by the fix")

print("all hidden checks passed")
