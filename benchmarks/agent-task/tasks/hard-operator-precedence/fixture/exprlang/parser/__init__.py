"""The parser package: token stream → AST (Pratt / precedence-climbing)."""

from .pratt import parse

__all__ = ["parse"]
