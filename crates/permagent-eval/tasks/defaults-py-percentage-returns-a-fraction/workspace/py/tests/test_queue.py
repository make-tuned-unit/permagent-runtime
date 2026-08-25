import unittest

from bench_py.scheduler.queue import SimpleQueue


class TestQueue(unittest.TestCase):
    def test_fifo_order(self):
        q = SimpleQueue()
        q.push("a")
        q.push("b")
        self.assertEqual(q.pop(), "a")
        self.assertEqual(len(q), 1)


if __name__ == "__main__":
    unittest.main()
