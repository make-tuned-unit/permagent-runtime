#!/usr/bin/env python3
"""Deterministic grader for the palindrome task. Exit 0 = solved."""
import sys


def main() -> int:
    try:
        from palindrome import is_palindrome
    except Exception as e:  # noqa: BLE001
        print(f"FAIL: could not import palindrome.is_palindrome: {e}", file=sys.stderr)
        return 1

    cases = [
        ("A man, a plan, a canal: Panama", True),
        ("race a car", False),
        ("", True),
        (" ", True),
        (".,", True),
        ("0P", False),
        ("aa", True),
        ("ab", False),
        ("Able was I, ere I saw Elba", True),
        ("No lemon, no melon", True),
        ("12321", True),
        ("12345", False),
    ]
    for s, want in cases:
        try:
            got = is_palindrome(s)
        except Exception as e:  # noqa: BLE001
            print(f"FAIL: is_palindrome({s!r}) raised {e}", file=sys.stderr)
            return 1
        if bool(got) != want:
            print(f"FAIL: is_palindrome({s!r}) = {got!r}, expected {want}", file=sys.stderr)
            return 1

    print("ok: palindrome verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
