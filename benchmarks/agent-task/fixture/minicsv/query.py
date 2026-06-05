"""A tiny fluent query API over a Table."""

from . import aggregate


class Query:
    """Chainable summariser: `Query(table).group("city").agg("temp", "std")`."""

    def __init__(self, table):
        self.table = table
        self._key = None

    def group(self, key_column):
        self._key = key_column
        return self

    def agg(self, value_column, how):
        """Aggregate `value_column` by the grouped key using aggregator `how`.

        Without a prior `.group(...)`, aggregates the whole column into a
        single value under the key "*".
        """
        if self._key is None:
            fn = aggregate.aggregator(how)
            return {"*": fn(self.table.numeric_column(value_column))}
        return aggregate.group_by(self.table, self._key, value_column, how)
