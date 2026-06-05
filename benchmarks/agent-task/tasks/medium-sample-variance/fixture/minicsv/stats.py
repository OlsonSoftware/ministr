"""Descriptive statistics over numeric columns.

Both population and sample estimators live here. Several functions mention
"variance" / "deviation", so finding *the one* a given task means is a real
navigation step.
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
    """Population variance: the second moment about the mean (divide by N).

    Use this when `values` is the *entire* population, not a sample drawn
    from a larger one.
    """
    values = list(values)
    if not values:
        raise EmptyColumnError("variance of an empty column")
    return _sum_squared_deviations(values, mean(values)) / len(values)


def sample_variance(values):
    """Unbiased sample variance (Bessel's correction: divide by N - 1).

    Use this when `values` is a *sample* drawn from a larger population —
    the N-1 denominator corrects the bias of the population estimator.
    """
    values = list(values)
    if len(values) < 2:
        raise EmptyColumnError("sample variance needs at least two values")
    # BUG: this divides by N (the population estimator) instead of N - 1, so
    # the returned sample variance — and the sample stdev built on it — is
    # biased low. The fix is Bessel's correction: divide by len(values) - 1.
    return _sum_squared_deviations(values, mean(values)) / len(values)


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
