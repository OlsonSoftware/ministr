"""Online (single-pass) statistics — a distractor neighbourhood.

This module also talks about "variance", but it is the streaming Welford
estimator, unrelated to the batch estimators in `stats.py`. It exists so that
a keyword search for "variance" surfaces more than one plausible hit.
"""


class RunningMoments:
    """Welford's online algorithm for mean and variance."""

    def __init__(self):
        self.n = 0
        self.mean = 0.0
        self._m2 = 0.0

    def push(self, x):
        self.n += 1
        delta = x - self.mean
        self.mean += delta / self.n
        self._m2 += delta * (x - self.mean)

    def running_variance(self):
        """Population variance accumulated so far (0 until two samples seen)."""
        if self.n < 2:
            return 0.0
        return self._m2 / self.n
