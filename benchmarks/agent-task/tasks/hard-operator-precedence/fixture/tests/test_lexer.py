"""Lexer tests — pass on the buggy fixture (the bug is in precedence, not lexing)."""

import unittest

from exprlang.lexer import tokenize


class LexerTest(unittest.TestCase):
    def test_token_types(self):
        types = [t.type for t in tokenize("2 + 3 * 4")]
        self.assertEqual(types, ["NUMBER", "PLUS", "NUMBER", "STAR", "NUMBER", "EOF"])

    def test_number_value(self):
        toks = tokenize("42")
        self.assertEqual(toks[0].type, "NUMBER")
        self.assertEqual(toks[0].value, 42.0)

    def test_parens(self):
        types = [t.type for t in tokenize("(1)")]
        self.assertEqual(types, ["LPAREN", "NUMBER", "RPAREN", "EOF"])


if __name__ == "__main__":
    unittest.main()
