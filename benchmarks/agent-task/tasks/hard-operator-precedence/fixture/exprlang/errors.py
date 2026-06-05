"""Errors for exprlang."""


class LexError(Exception):
    """Unexpected character during tokenization."""


class ParseError(Exception):
    """Malformed expression."""
