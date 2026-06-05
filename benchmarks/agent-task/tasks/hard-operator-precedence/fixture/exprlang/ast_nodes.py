"""AST node types produced by the parser and consumed by the evaluator."""


class Num:
    __slots__ = ("value",)

    def __init__(self, value):
        self.value = value


class BinOp:
    __slots__ = ("op", "left", "right")

    def __init__(self, op, left, right):
        self.op = op          # token type: PLUS / MINUS / STAR / SLASH
        self.left = left
        self.right = right


class Neg:
    __slots__ = ("operand",)

    def __init__(self, operand):
        self.operand = operand
