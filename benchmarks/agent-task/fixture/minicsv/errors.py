"""Error types for minicsv."""


class MiniCsvError(Exception):
    """Base class for all minicsv errors."""


class SchemaError(MiniCsvError):
    """Raised when a column is missing or has an unexpected type."""


class EmptyColumnError(MiniCsvError):
    """Raised when a statistic is requested over too few values."""
