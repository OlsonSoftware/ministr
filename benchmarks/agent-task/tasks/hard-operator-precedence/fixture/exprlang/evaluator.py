"""Tree-walking evaluator: AST → float. Correct as written (not the bug)."""

from .ast_nodes import BinOp, Neg, Num
from .errors import ParseError

_OPS = {
    "PLUS": lambda a, b: a + b,
    "MINUS": lambda a, b: a - b,
    "STAR": lambda a, b: a * b,
    "SLASH": lambda a, b: a / b,
}


def evaluate_ast(node):
    if isinstance(node, Num):
        return node.value
    if isinstance(node, Neg):
        return -evaluate_ast(node.operand)
    if isinstance(node, BinOp):
        return _OPS[node.op](evaluate_ast(node.left), evaluate_ast(node.right))
    raise ParseError(f"cannot evaluate node {node!r}")
