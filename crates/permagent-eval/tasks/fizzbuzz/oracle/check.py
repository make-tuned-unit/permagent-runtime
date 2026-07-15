#!/usr/bin/env python3
"""Deterministic grader for the fizzbuzz task. Exit 0 = solved."""
import sys


def main() -> int:
    try:
        from fizzbuzz import fizzbuzz
    except Exception as e:  # noqa: BLE001
        print(f"FAIL: could not import fizzbuzz.fizzbuzz: {e}", file=sys.stderr)
        return 1

    expected = {
        1: "1",
        2: "2",
        3: "Fizz",
        5: "Buzz",
        6: "Fizz",
        9: "Fizz",
        10: "Buzz",
        15: "FizzBuzz",
        30: "FizzBuzz",
        7: "7",
        14: "14",
        45: "FizzBuzz",
        20: "Buzz",
    }
    for n, want in expected.items():
        try:
            got = fizzbuzz(n)
        except Exception as e:  # noqa: BLE001
            print(f"FAIL: fizzbuzz({n}) raised {e}", file=sys.stderr)
            return 1
        if got != want:
            print(f"FAIL: fizzbuzz({n}) = {got!r}, expected {want!r}", file=sys.stderr)
            return 1

    print("ok: fizzbuzz verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
