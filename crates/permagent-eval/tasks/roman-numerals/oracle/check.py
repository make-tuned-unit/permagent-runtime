#!/usr/bin/env python3
"""Deterministic grader for the roman-numerals task. Exit 0 = solved."""
import sys


def reference(n: int) -> str:
    table = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"),
        (100, "C"), (90, "XC"), (50, "L"), (40, "XL"),
        (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ]
    out = []
    for value, sym in table:
        while n >= value:
            out.append(sym)
            n -= value
    return "".join(out)


def main() -> int:
    try:
        from roman import to_roman
    except Exception as e:  # noqa: BLE001
        print(f"FAIL: could not import roman.to_roman: {e}", file=sys.stderr)
        return 1

    spot = {1: "I", 4: "IV", 9: "IX", 40: "XL", 90: "XC", 400: "CD",
            900: "CM", 58: "LVIII", 1994: "MCMXCIV", 2024: "MMXXIV",
            3888: "MMMDCCCLXXXVIII", 3999: "MMMCMXCIX"}
    for n, want in spot.items():
        try:
            got = to_roman(n)
        except Exception as e:  # noqa: BLE001
            print(f"FAIL: to_roman({n}) raised {e}", file=sys.stderr)
            return 1
        if got != want:
            print(f"FAIL: to_roman({n}) = {got!r}, expected {want!r}", file=sys.stderr)
            return 1

    # Exhaustive cross-check against a reference implementation.
    for n in range(1, 4000):
        if to_roman(n) != reference(n):
            print(f"FAIL: to_roman({n}) = {to_roman(n)!r}, expected {reference(n)!r}", file=sys.stderr)
            return 1

    print("ok: roman numerals verified 1..3999")
    return 0


if __name__ == "__main__":
    sys.exit(main())
