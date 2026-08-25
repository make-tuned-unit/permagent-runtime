from collections import deque


class SimpleQueue:
    def __init__(self):
        self._items = deque()

    def push(self, item):
        self._items.append(item)

    def pop(self):
        return self._items.popleft()

    def __len__(self):
        return len(self._items)
