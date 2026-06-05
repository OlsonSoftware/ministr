"""Evaluator tests. The precedence cases fail on the buggy fixture."""

import unittest

from exprlang import evaluate


class EvalTest(unittest.TestCase):
    def test_precedence_mul_over_add(self):
        # The headline failure: * must bind tighter than +.
        self.assertEqual(evaluate("2 + 3 * 4"), 14)

    def test_precedence_mul_first(self):
        self.assertEqual(evaluate("2 * 3 + 4"), 10)

    def test_parens_override(self):
        self.assertEqual(evaluate("(2 + 3) * 4"), 20)

    def test_mixed(self):
        self.assertEqual(evaluate("10 - 2 * 3"), 4)
        self.assertEqual(evaluate("1 + 2 * 3 - 4"), 3)

    def test_left_assoc_div(self):
        self.assertEqual(evaluate("100 / 10 / 2"), 5)

    def test_unary_minus(self):
        self.assertEqual(evaluate("-3 + 5"), 2)
        self.assertEqual(evaluate("-(2 + 3)"), -5)


if __name__ == "__main__":
    unittest.main()
