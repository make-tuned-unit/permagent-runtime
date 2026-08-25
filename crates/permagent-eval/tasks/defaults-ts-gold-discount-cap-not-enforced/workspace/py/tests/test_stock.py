import unittest

from bench_py.inventory.stock import available_quantity, is_low_stock


class TestStock(unittest.TestCase):
    def test_available_quantity(self):
        self.assertEqual(available_quantity(10, 3), 7)

    def test_is_low_stock(self):
        self.assertTrue(is_low_stock(2, 5))
        self.assertFalse(is_low_stock(10, 5))


if __name__ == "__main__":
    unittest.main()
