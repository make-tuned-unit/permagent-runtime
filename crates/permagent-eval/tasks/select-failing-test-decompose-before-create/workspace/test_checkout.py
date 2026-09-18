from pricing import apply_discount
from inventory import decrement_stock


def test_checkout():
    assert apply_discount(100.0, 0.10) == 90.0
    assert decrement_stock(10, 3) == 7


if __name__ == "__main__":
    test_checkout()
    print("ok")
