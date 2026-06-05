"""CSV text → Table. Minimal, dependency-free parsing (no quoting rules)."""

from .table import Table


def read_csv(text, has_header=True):
    """Parse `text` (a CSV string) into a `Table`.

    Splits on newlines and commas, trims whitespace, and coerces numeric
    cells to floats where possible. The first row is the header unless
    `has_header=False`, in which case columns are named c0, c1, ...
    """
    lines = [ln for ln in text.splitlines() if ln.strip()]
    if not lines:
        return Table([], [])
    rows = [[cell.strip() for cell in ln.split(",")] for ln in lines]
    if has_header:
        header, body = rows[0], rows[1:]
    else:
        header = [f"c{i}" for i in range(len(rows[0]))]
        body = rows
    coerced = [[_coerce(cell) for cell in row] for row in body]
    return Table(header, coerced)


def _coerce(cell):
    """Best-effort numeric coercion; leave non-numeric cells as strings."""
    try:
        return float(cell)
    except (TypeError, ValueError):
        return cell
