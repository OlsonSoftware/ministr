"""The in-memory Table: a header plus row-major data."""

from .errors import SchemaError


class Table:
    def __init__(self, header, rows):
        self.header = list(header)
        self.rows = [list(r) for r in rows]

    def __len__(self):
        return len(self.rows)

    def column_index(self, name):
        try:
            return self.header.index(name)
        except ValueError as exc:
            raise SchemaError(f"no such column: {name!r}") from exc

    def column(self, name):
        """Return the values of one column as a list (row order preserved)."""
        idx = self.column_index(name)
        return [row[idx] for row in self.rows]

    def numeric_column(self, name):
        """Return a column as floats, raising SchemaError on a non-numeric cell."""
        out = []
        for value in self.column(name):
            if not isinstance(value, (int, float)):
                raise SchemaError(f"column {name!r} has non-numeric value {value!r}")
            out.append(float(value))
        return out
