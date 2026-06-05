"""Group-by aggregation built on top of the column statistics."""

from . import stats
from .errors import SchemaError

# The aggregation functions a Query may request, by name. `std` deliberately
# routes to the SAMPLE standard deviation — group summaries treat each group
# as a sample drawn from a larger population.
_AGGREGATORS = {
    "mean": stats.mean,
    "median": stats.median,
    "std": stats.stdev,
    "var": stats.sample_variance,
    "pop_std": stats.population_stdev,
    "min": min,
    "max": max,
    "sum": sum,
    "count": len,
}


def aggregator(name):
    if name not in _AGGREGATORS:
        raise SchemaError(f"unknown aggregator: {name!r}")
    return _AGGREGATORS[name]


def group_by(table, key_column, value_column, agg):
    """Group `table` rows by `key_column`, aggregating `value_column` with `agg`.

    Returns a dict {key: aggregated_value}. `agg` is an aggregator name from
    `_AGGREGATORS` (e.g. "mean", "std").
    """
    fn = aggregator(agg)
    key_idx = table.column_index(key_column)
    buckets = {}
    for value, row in zip(table.numeric_column(value_column), table.rows):
        buckets.setdefault(row[key_idx], []).append(value)
    return {key: fn(vals) for key, vals in buckets.items()}


def covariance(xs, ys):
    """Sample covariance of two equal-length numeric sequences."""
    xs, ys = list(xs), list(ys)
    if len(xs) != len(ys):
        raise SchemaError("covariance needs equal-length columns")
    n = len(xs)
    if n < 2:
        raise SchemaError("covariance needs at least two pairs")
    mx, my = stats.mean(xs), stats.mean(ys)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / (n - 1)
