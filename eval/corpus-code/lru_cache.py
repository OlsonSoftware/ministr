"""Fixed-capacity least-recently-used cache.

Evicts the entry that has gone unused the longest once the cache is full.
Part of the code-heavy evaluation corpus (eval/corpus-code) for benchmarking
embedders on natural-language-to-code retrieval.
"""

from collections import OrderedDict
from typing import Any, Hashable, Optional


class LRUCache:
    """A mapping that keeps only the most recently accessed `capacity` items.

    Backed by an ordered dictionary so that recency is tracked in O(1) per
    access: reads and writes move the touched key to the most-recent end, and
    eviction always removes the least-recent end.
    """

    def __init__(self, capacity: int) -> None:
        if capacity <= 0:
            raise ValueError("capacity must be positive")
        self.capacity = capacity
        self._store: "OrderedDict[Hashable, Any]" = OrderedDict()

    def get(self, key: Hashable) -> Optional[Any]:
        """Return the cached value and mark the key as most recently used.

        A cache miss returns None and does not change recency ordering.
        """
        if key not in self._store:
            return None
        self._store.move_to_end(key)
        return self._store[key]

    def put(self, key: Hashable, value: Any) -> None:
        """Insert or update a value, evicting the oldest entry if over capacity."""
        if key in self._store:
            self._store.move_to_end(key)
        self._store[key] = value
        if len(self._store) > self.capacity:
            self._store.popitem(last=False)

    def __len__(self) -> int:
        return len(self._store)


def memoize_fibonacci(n: int, cache: Optional[LRUCache] = None) -> int:
    """Compute the nth Fibonacci number, caching overlapping subproblems.

    Demonstrates how a small LRU cache collapses the exponential recursion of
    the naive definition down to linear time by reusing previously computed
    values instead of recomputing them.
    """
    if cache is None:
        cache = LRUCache(64)
    if n < 2:
        return n
    hit = cache.get(n)
    if hit is not None:
        return hit
    value = memoize_fibonacci(n - 1, cache) + memoize_fibonacci(n - 2, cache)
    cache.put(n, value)
    return value
