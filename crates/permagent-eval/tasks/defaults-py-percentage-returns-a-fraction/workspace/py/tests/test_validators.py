import unittest

from bench_py.api.validators import validate_username


class TestValidators(unittest.TestCase):
    def test_valid(self):
        self.assertTrue(validate_username("jesse_1"))

    def test_too_short(self):
        self.assertFalse(validate_username("ab"))


if __name__ == "__main__":
    unittest.main()
