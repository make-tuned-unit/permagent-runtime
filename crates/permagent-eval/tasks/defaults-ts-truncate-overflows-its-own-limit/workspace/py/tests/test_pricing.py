import unittest

from bench_py.inventory.pricing import apply_bulk_discount


class TestPricing(unittest.TestCase):
    def test_no_discount(self):
        self.assertEqual(apply_bulk_discount(500, 2, 0.0), 1000)

    def test_simple_discount(self):
        self.assertEqual(apply_bulk_discount(1000, 1, 0.5), 500)

    def test_rounds_half_up(self):
        # 25% off 2002 cents leaves exactly 500.5 cents, which must round
        # UP to 501 -- not down to Python's banker's-rounding 500.
        self.assertEqual(apply_bulk_discount(2002, 1, 0.75), 501)


if __name__ == "__main__":
    unittest.main()
