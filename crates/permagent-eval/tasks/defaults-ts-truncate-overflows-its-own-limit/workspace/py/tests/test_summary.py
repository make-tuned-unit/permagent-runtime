import unittest

from bench_py.reports.summary import rank_players


class TestSummary(unittest.TestCase):
    def test_no_ties_smoke(self):
        result = rank_players([("a", 10), ("b", 5)])
        self.assertEqual(len(result), 2)


if __name__ == "__main__":
    unittest.main()
