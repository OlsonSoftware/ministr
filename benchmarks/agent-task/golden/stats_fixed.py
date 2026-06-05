"""Descriptive statistics over numeric columns.

GOLDEN reference copy: identical to fixture/minicsv/stats.py EXCEPT that
`sample_variance` applies Bessel's correction (divide by N - 1). The harness
swaps this in for the deterministic self-test (broken fixture must fail; this
fixed version must pass) — it is the known-good solution, not part of the
fixture the agent sees.
"""

import math

from .errors import EmptyColumnError


def mean(values):
    """Arithmetic mean of a non-empty sequence."""
    values = list(values)
    if not values:
        raise EmptyColumnError("mean of an empty column")
    return sum(values) / len(values)


def _sum_squared_deviations(values, center):
    """Σ (x - center)² — the shared core of every variance estimator."""
    return sum((x - center) ** 2 for x in values)


def population_variance(values):
    """Population variance: the second moment about the mean (divide by N)."""
    values = list(values)
    if not values:
        raise EmptyColumnError("variance of an empty column")
    return _sum_squared_deviations(values, mean(values)) / len(values)


def sample_variance(values):
    """Unbiased sample variance (Bessel's correction: divide by N - 1)."""
    values = list(values)
    if len(values) < 2:
        raise EmptyColumnError("sample variance needs at least two values")
    return _sum_squared_deviations(values, mean(values)) / (len(values) - 1)


def stdev(values):
    """Sample standard deviation (the square root of the sample variance)."""
    return math.sqrt(sample_variance(values))


def population_stdev(values):
    """Population standard deviation (root of the population variance)."""
    return math.sqrt(population_variance(values))


def median(values):
    """Median; averages the two middle values for an even-length input."""
    values = sorted(values)
    n = len(values)
    if n == 0:
        raise EmptyColumnError("median of an empty column")
    mid = n // 2
    if n % 2 == 1:
        return values[mid]
    return (values[mid - 1] + values[mid]) / 2


def quantile(values, q):
    """Linear-interpolated quantile for q in [0, 1]."""
    values = sorted(values)
    if not values:
        raise EmptyColumnError("quantile of an empty column")
    if not 0.0 <= q <= 1.0:
        raise ValueError("q must be in [0, 1]")
    pos = q * (len(values) - 1)
    lo = math.floor(pos)
    hi = math.ceil(pos)
    if lo == hi:
        return values[int(pos)]
    return values[lo] * (hi - pos) + values[hi] * (pos - lo)
