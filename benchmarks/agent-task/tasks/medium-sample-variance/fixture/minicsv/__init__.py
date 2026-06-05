"""minicsv — a tiny in-memory CSV analytics library (benchmark fixture).

Deliberately small but multi-module so that *locating* the right code is a
real sub-task. The public surface is a `Table` you can load from CSV text,
select columns from, group by, and aggregate.
"""

from .reader import read_csv
from .table import Table
from .query import Query

__all__ = ["read_csv", "Table", "Query"]
